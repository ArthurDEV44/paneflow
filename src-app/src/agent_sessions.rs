//! Shared types and helpers for the AI-agent session readers
//! (`claude_sessions`, `codex_sessions`, `opencode_sessions`). Each reader
//! sources sessions from its agent's native store — JSONL transcripts on
//! disk for Claude / Codex, `opencode session list --format json` shell-out
//! for OpenCode (whose backing SQLite schema is intentionally not exposed
//! as a stable contract; see US-001 spike notes in
//! `tasks/prd-opencode-sessions-decisions.md`). All three normalise to
//! the unified [`SessionMeta`] below so the popover UI can render rows
//! with a single template.

/// Which AI agent created the session. Drives the row icon, the
/// `--resume` command shape, and the popover tab the row sits under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAgent {
    Claude,
    Codex,
    OpenCode,
}

/// Read user config and return the agents whose tab-bar button is currently
/// visible, in display order (Claude → Codex → OpenCode). Both the popover
/// tab strip and the on-open session scans filter through this so a hidden
/// agent never appears in the UI and we don't pay the I/O cost of a scan
/// the user can't see. An unset field is treated as visible (matches the
/// behaviour in `pane.rs` where the buttons render by default).
pub fn enabled_session_agents() -> Vec<SessionAgent> {
    let cfg = paneflow_config::loader::load_config();
    let mut agents = Vec::with_capacity(3);
    if cfg.claude_code_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::Claude);
    }
    if cfg.codex_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::Codex);
    }
    if cfg.opencode_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::OpenCode);
    }
    agents
}

/// Unified session metadata. Anything the UI needs to render a row +
/// resume the session is here; the heavier message payload stays on disk.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    /// Which CLI created the session — drives row routing and the resume
    /// command (`claude --resume <id>` vs `codex resume <id>`).
    pub agent: SessionAgent,
    pub session_id: String,
    /// ISO 8601 timestamp from the first event. Used for sorting (lexical
    /// sort matches chronological order for ISO 8601).
    pub timestamp: String,
    /// `cwd` recorded on the first line. Files where the first line lacks
    /// `cwd` are skipped, so this is always populated.
    pub cwd: String,
    /// Git branch — empty string when the session was outside a git repo
    /// (Claude Code) or when the agent doesn't record one (Codex CLI).
    pub git_branch: String,
    /// Human-readable session label. Sourced from an LLM-generated title
    /// when available, falling back to the cleaned first user message
    /// otherwise. `None` if neither could be extracted.
    pub summary: Option<String>,
}

/// Format an ISO 8601 timestamp into a short relative label. Pure string
/// math (no `chrono` dep) — parses `YYYY-MM-DDTHH:MM:SS` and computes the
/// delta against `std::time::SystemTime::now()` via a calendar-free
/// approximation good enough for "Xm ago" / "Xh ago" / "Xd ago" labels.
///
/// Falls back to the date prefix (`YYYY-MM-DD`) when parsing fails.
pub fn format_relative_time(iso8601: &str) -> String {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    match parse_iso8601_to_unix_secs(iso8601) {
        Some(ts_secs) => {
            let delta = now_secs.saturating_sub(ts_secs);
            relative_label(delta)
        }
        None => iso8601.split('T').next().unwrap_or(iso8601).to_string(),
    }
}

fn relative_label(delta_secs: i64) -> String {
    if delta_secs < 60 {
        return "just now".to_string();
    }
    if delta_secs < 3_600 {
        return format!("{}m ago", delta_secs / 60);
    }
    if delta_secs < 86_400 {
        return format!("{}h ago", delta_secs / 3_600);
    }
    if delta_secs < 30 * 86_400 {
        return format!("{}d ago", delta_secs / 86_400);
    }
    if delta_secs < 365 * 86_400 {
        return format!("{}mo ago", delta_secs / (30 * 86_400));
    }
    format!("{}y ago", delta_secs / (365 * 86_400))
}

/// Minimal ISO 8601 → Unix-seconds parser. Accepts
/// `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`. Treats the timestamp as UTC.
/// Calendar math via Howard Hinnant's "days from civil" algorithm; an
/// off-by-one on leap-second boundaries is acceptable for a relative-time
/// UI label.
fn parse_iso8601_to_unix_secs(iso: &str) -> Option<i64> {
    let (date, rest) = iso.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let time = rest
        .split_once(['Z', '+', '-'])
        .map(|(t, _)| t)
        .unwrap_or(rest);
    let time = time.split('.').next().unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146_097 + doe - 719_468;

    Some(days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_label_under_minute() {
        assert_eq!(relative_label(15), "just now");
    }

    #[test]
    fn relative_label_minutes() {
        assert_eq!(relative_label(125), "2m ago");
    }

    #[test]
    fn relative_label_hours() {
        assert_eq!(relative_label(7_400), "2h ago");
    }

    #[test]
    fn relative_label_days() {
        assert_eq!(relative_label(3 * 86_400 + 100), "3d ago");
    }

    #[test]
    fn iso8601_parses_z() {
        let secs = parse_iso8601_to_unix_secs("2025-01-15T12:30:45Z").unwrap();
        assert_eq!(secs, 1_736_944_245);
    }

    #[test]
    fn iso8601_parses_fractional_seconds() {
        let secs = parse_iso8601_to_unix_secs("2025-01-15T12:30:45.123Z").unwrap();
        assert_eq!(secs, 1_736_944_245);
    }

    #[test]
    fn iso8601_unparseable_falls_back_to_date_prefix() {
        let label = format_relative_time("not a real timestamp");
        assert_eq!(label, "not a real timestamp");
    }
}
