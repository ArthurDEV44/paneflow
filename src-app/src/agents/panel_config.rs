//! US-103: process-wide accessor for the Agents-view's `agent_panel`
//! configuration block.
//!
//! The bootstrap installs the initial config; the `ConfigWatcher`
//! deposits fresh copies on every successful reload via
//! [`install_agent_panel_config`]. Rendering code reads via
//! [`active_agent_panel_config`] -- the read path is lock-free for
//! the common case (an [`std::sync::RwLock`] read guard with no
//! contention).
//!
//! Lives in `agents::` rather than `theme::` because the
//! AgentPanelConfig keeps expanding through Phases B-E of
//! `tasks/prd-agent-ui-refactor-2026-Q3.md` (thinking display mode,
//! notification gate, profiles, ...). Centralising the accessor here
//! avoids smearing config-read calls across every renderer.

use paneflow_config::schema::{
    AgentPanelConfig, NotifyWhenAgentWaiting, ProfileConfig, ToolPermissionsEntry,
};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock, RwLockReadGuard};

use super::runtime::ToolKindKind;

static AGENT_PANEL_CONFIG: OnceLock<RwLock<AgentPanelConfig>> = OnceLock::new();
static TOOL_PERMISSIONS: OnceLock<RwLock<HashMap<String, ToolPermissionsEntry>>> = OnceLock::new();
static DEFAULT_SHELL: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn slot() -> &'static RwLock<AgentPanelConfig> {
    AGENT_PANEL_CONFIG.get_or_init(|| RwLock::new(AgentPanelConfig::default()))
}

fn permissions_slot() -> &'static RwLock<HashMap<String, ToolPermissionsEntry>> {
    TOOL_PERMISSIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn default_shell_slot() -> &'static RwLock<Option<String>> {
    DEFAULT_SHELL.get_or_init(|| RwLock::new(None))
}

/// Replace the cached `AgentPanelConfig`. Called by the bootstrap on
/// startup and by the config hot-reload handler on every successful
/// reload. Idempotent for an identical config (no broadcast event).
pub fn install_agent_panel_config(config: AgentPanelConfig) {
    let mut guard = match slot().write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    *guard = config;
}

/// Read the currently-installed `AgentPanelConfig`. Returns a clone
/// so the caller does not hold the lock across the render pass
/// (which would risk contention with a concurrent config-reload).
pub fn active_agent_panel_config() -> AgentPanelConfig {
    let guard: RwLockReadGuard<'_, _> = match slot().read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clone()
}

/// Convenience: the resolved max-content-width in pixels, with
/// default + clamp applied (US-103 AC #1 / #5). Used by
/// [`crate::agents::thread_view::ThreadView`]'s render pass.
pub fn active_max_content_width() -> u32 {
    active_agent_panel_config().resolved_max_content_width()
}

/// US-116: resolved notification gate. Read on every `TurnEnded` /
/// `Refusal` / `Fatal` runtime event by
/// [`crate::agents::notifications`] so a live edit to
/// `agent_panel.notify_when_agent_waiting` (via the config watcher)
/// takes effect on the very next turn -- no app restart.
pub fn active_notify_when_agent_waiting() -> NotifyWhenAgentWaiting {
    active_agent_panel_config().resolved_notify_when_agent_waiting()
}

/// Replace the cached per-tool permissions map (US-111).
/// Called by the bootstrap and by the config hot-reload handler so
/// edits to `tool_permissions` in `paneflow.json` take effect
/// without a restart (US-111 AC #7).
pub fn install_tool_permissions(map: HashMap<String, ToolPermissionsEntry>) {
    let mut guard = match permissions_slot().write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    *guard = map;
}

/// Read the currently-installed tool-permissions map. Returns a
/// clone so the caller is not holding the lock during the
/// permission-resolve check.
pub fn active_tool_permissions() -> HashMap<String, ToolPermissionsEntry> {
    let guard = match permissions_slot().read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clone()
}

/// Replace the cached default-shell path (US-111 AC #6).
/// Called by the bootstrap once after the config is loaded; the
/// shell name is then read by `terminal::shell::active_basename()`
/// without an extra disk read on every render.
pub fn install_default_shell(path: Option<String>) {
    let mut guard = match default_shell_slot().write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    *guard = path;
}

/// Read the cached default shell path. `None` indicates the
/// bootstrap has not run yet (only possible during very early
/// startup or in a unit test environment).
pub fn active_default_shell() -> Option<String> {
    let guard = match default_shell_slot().read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clone()
}

/// Whether the active tool-permissions map auto-resolves a call of
/// `kind` to `Allow`. US-124 refinement: an entry whose `always_allow`
/// is empty matches every call (the v1 "any-input" behavior US-111
/// shipped); a non-empty `always_allow` matches only when one of its
/// substring patterns is present in `raw_input` (when supplied). When
/// the caller has no raw_input (most kinds today), the v1 any-input
/// contract still applies so existing entries do not silently stop
/// firing.
#[allow(dead_code)] // kept as the no-input convenience for future callers
pub fn tool_kind_is_always_allowed(kind: ToolKindKind) -> bool {
    tool_kind_is_always_allowed_for(kind, None)
}

/// US-124: pattern-aware variant. Returns `true` when an entry exists
/// for `kind` AND either has no `always_allow` patterns (any-input
/// match) OR at least one of its patterns is a substring of
/// `raw_input`. `always_deny` always takes precedence elsewhere via
/// [`tool_kind_is_always_denied_for`].
pub fn tool_kind_is_always_allowed_for(kind: ToolKindKind, raw_input: Option<&str>) -> bool {
    let map = active_tool_permissions();
    let key = tool_kind_key(kind);
    let Some(entry) = map.get(key) else {
        return false;
    };
    if !entry.always_deny.is_empty() && raw_input.is_none() {
        // Defensive: when we don't know the input, do not auto-allow a
        // kind that also has deny patterns -- the user explicitly
        // restricted some inputs, so fall through to the UI.
        return false;
    }
    if entry.always_allow.is_empty() {
        // Bare entry means "any input for this kind" (US-111 shape).
        // Honor it even when the caller has no input to match against.
        return true;
    }
    let Some(raw) = raw_input else {
        // Patterns are set but the caller cannot pass the input --
        // fall through to the UI prompt rather than guess.
        return false;
    };
    entry
        .always_allow
        .iter()
        .any(|pattern| matches_pattern(pattern, raw))
}

/// Whether the active map auto-rejects the call. Mirror of
/// [`tool_kind_is_always_allowed`]: an `always_deny` list with no
/// patterns matches every call; a non-empty list matches by substring.
#[allow(dead_code)] // kept as the no-input convenience for future callers
pub fn tool_kind_is_always_denied(kind: ToolKindKind) -> bool {
    tool_kind_is_always_denied_for(kind, None)
}

/// US-124: pattern-aware deny variant. Returns `true` when one of the
/// `always_deny` patterns is a substring of `raw_input` (or any-input
/// when the list is bare). `None` for `raw_input` keeps the legacy
/// contract: only bare deny entries match.
pub fn tool_kind_is_always_denied_for(kind: ToolKindKind, raw_input: Option<&str>) -> bool {
    let map = active_tool_permissions();
    let key = tool_kind_key(kind);
    let Some(entry) = map.get(key) else {
        return false;
    };
    if entry.always_deny.is_empty() {
        return false;
    }
    let Some(raw) = raw_input else {
        // Without input, deny only when the list is bare (legacy
        // semantic: "Reject Always for this kind, any input").
        return entry.always_deny.iter().any(|p| p.is_empty());
    };
    entry
        .always_deny
        .iter()
        .any(|pattern| matches_pattern(pattern, raw))
}

/// US-124 helper (pure): substring match a stored pattern against the
/// pretty-printed `raw_input`. Empty pattern matches everything (this
/// preserves the v1 contract where an empty `always_allow` vec implies
/// any input). Case-sensitive by design -- developer tooling commands
/// are case-significant (`Cargo.toml` vs `cargo.toml`).
pub fn matches_pattern(pattern: &str, raw_input: &str) -> bool {
    pattern.is_empty() || raw_input.contains(pattern)
}

/// Canonical persisted key for a `ToolKindKind`. Kept in `agents::`
/// so the wire enum stays free of persistence concerns. Matches
/// Zed §13's snake_case tool-name convention.
pub fn tool_kind_key(kind: ToolKindKind) -> &'static str {
    match kind {
        ToolKindKind::Read => "read",
        ToolKindKind::Edit => "edit",
        ToolKindKind::Delete => "delete",
        ToolKindKind::Move => "move",
        ToolKindKind::Search => "search",
        ToolKindKind::Execute => "execute",
        ToolKindKind::Think => "think",
        ToolKindKind::Fetch => "fetch",
        ToolKindKind::SwitchMode => "switch_mode",
        ToolKindKind::Other => "other",
    }
}

/// US-115: read the persisted profiles. Returns a clone so the
/// caller is not holding the lock during the popover render.
pub fn active_profiles() -> HashMap<String, ProfileConfig> {
    active_agent_panel_config().profiles
}

/// US-115: read the `default_profile` field if set. `None` is the
/// "no preference" signal -- the composer falls back to whatever
/// the user last picked in-memory, ultimately the `Write` built-in.
pub fn active_default_profile() -> Option<String> {
    active_agent_panel_config().default_profile
}

/// US-115: write the profile map back to `paneflow.json` (under the
/// nested `agent_panel.profiles` key) and update the in-process
/// cache. Mirrors [`record_tool_permission_always_allow`] -- the
/// cache update happens first so a hot-reload race does not
/// re-install the stale value before the disk write completes.
pub fn save_profiles_to_disk(profiles: HashMap<String, ProfileConfig>) {
    let mut cfg = active_agent_panel_config();
    cfg.profiles = profiles.clone();
    install_agent_panel_config(cfg);
    write_agent_panel_subfield("profiles", serde_json::to_value(&profiles).ok());
}

/// US-115: write the `default_profile` field back to `paneflow.json`.
/// `None` removes the key (idempotent on missing).
pub fn save_default_profile_to_disk(name: Option<String>) {
    let mut cfg = active_agent_panel_config();
    cfg.default_profile = name.clone();
    install_agent_panel_config(cfg);
    let value = name.map(serde_json::Value::String);
    write_agent_panel_subfield("default_profile", value);
}

/// US-115: shared writer for fields nested under the top-level
/// `agent_panel` object in `paneflow.json`. Read-modify-write of a
/// raw JSON value keeps the format + unknown sibling fields intact.
/// Passing `None` for `value` removes the field. Sibling fields
/// (`max_content_width`, `thinking_display`, ...) are untouched.
fn write_agent_panel_subfield(field: &str, value: Option<serde_json::Value>) {
    let Some(path) = paneflow_config::loader::config_path() else {
        tracing::warn!(
            target: "paneflow_app::agents::panel_config",
            "cannot resolve paneflow.json path, agent_panel.{field} not persisted",
        );
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut root: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
    let obj = match root.as_object_mut() {
        Some(o) => o,
        None => {
            tracing::warn!(
                target: "paneflow_app::agents::panel_config",
                "paneflow.json root is not a JSON object",
            );
            return;
        }
    };
    if !obj.contains_key("agent_panel") {
        obj.insert(
            "agent_panel".to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
    }
    if let Some(agent_panel) = obj.get_mut("agent_panel").and_then(|v| v.as_object_mut()) {
        match value {
            Some(v) => {
                agent_panel.insert(field.to_string(), v);
            }
            None => {
                agent_panel.remove(field);
            }
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&root) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                tracing::warn!(
                    target: "paneflow_app::agents::panel_config",
                    "failed to persist agent_panel.{field}: {err}",
                );
            }
        }
        Err(err) => tracing::warn!(
            target: "paneflow_app::agents::panel_config",
            "failed to serialize paneflow.json: {err}",
        ),
    }
}

/// Persist an "always allow" rule for `kind` to `paneflow.json` and
/// update the in-process cache so the next call resolves without
/// prompting. Kept as the legacy single-arg entry point; US-124's
/// inline picker calls [`record_tool_permission_with_pattern`]
/// directly. `_title` is unused -- left in the signature so external
/// callers that linked against the US-111 surface keep compiling.
#[allow(dead_code)] // kept as the legacy convenience for future callers
pub fn record_tool_permission_always_allow(kind: ToolKindKind, _title: &str) {
    record_tool_permission_with_pattern(kind, None);
}

/// US-124: persist an "always allow" rule with an optional pattern.
/// `Some(pattern)` appends a substring pattern (so a future call with
/// matching `raw_input` auto-allows); `None` clears the patterns and
/// records a bare entry (matches every call of this kind -- the
/// US-111 default). Either way, any prior `always_deny` for this
/// kind is cleared so the rule is unambiguous after the user
/// explicitly opts in.
pub fn record_tool_permission_with_pattern(kind: ToolKindKind, pattern: Option<String>) {
    let key = tool_kind_key(kind).to_string();
    let mut map = active_tool_permissions();
    let entry = map.entry(key.clone()).or_default();
    match pattern {
        Some(p) if !p.is_empty() => {
            // Avoid duplicate inserts so the persisted list stays a
            // small set rather than growing on every click.
            if !entry.always_allow.iter().any(|e| e == &p) {
                entry.always_allow.push(p);
            }
            entry.always_deny.retain(|e| !matches_pattern(e, ""));
        }
        _ => {
            entry.always_allow.clear();
            entry.always_deny.clear();
        }
    }
    install_tool_permissions(map.clone());
    crate::config_writer::save_config_value(
        "tool_permissions",
        serde_json::to_value(&map).unwrap_or(serde_json::Value::Null),
    );
}

/// US-124 AC #3: auto-promote a previously-allowed pattern to the
/// deny list when the user rejects a tool call that matched it. The
/// pattern is removed from `always_allow` and appended to
/// `always_deny`. Idempotent: re-running with the same pattern is a
/// no-op (already promoted). Returns `Some(pattern)` when a promotion
/// happened so the UI layer can toast "Pattern moved to deny list".
pub fn auto_promote_to_deny(kind: ToolKindKind, raw_input: &str) -> Option<String> {
    let key = tool_kind_key(kind).to_string();
    let mut map = active_tool_permissions();
    let entry = map.get_mut(&key)?;
    // Find the FIRST always_allow pattern that matches the raw input.
    // Bare-entry / any-input rules ("" pattern) are special-cased: we
    // do NOT promote them (it would silently flip the meaning of the
    // entry from "allow everything" to "deny everything", which is
    // never what the user intends on a single Reject click).
    let promoted_idx = entry
        .always_allow
        .iter()
        .position(|pattern| !pattern.is_empty() && matches_pattern(pattern, raw_input))?;
    let pattern = entry.always_allow.remove(promoted_idx);
    if !entry.always_deny.iter().any(|e| e == &pattern) {
        entry.always_deny.push(pattern.clone());
    }
    install_tool_permissions(map.clone());
    crate::config_writer::save_config_value(
        "tool_permissions",
        serde_json::to_value(&map).unwrap_or(serde_json::Value::Null),
    );
    Some(pattern)
}

/// US-124 (pure): given a terminal command (the `command` field of an
/// Execute tool call's raw input), propose a small ordered list of
/// patterns the user can pick from in the inline picker.
///
/// Heuristic:
/// - Skip blanks / empties -> single "any-input" proposal.
/// - First token is the executable name (e.g. `npm`).
/// - If a second token exists, propose the exact 2-token subcommand
///   (e.g. `npm install`) FIRST, then the executable family
///   (`npm <any>`).
/// - Single-token commands propose just the executable family.
/// - Pipes / `&&` / `;` are NOT parsed -- if the command contains any
///   of those, only the `<any>` proposal is returned (the user can
///   still grant blanket access; specific patterns become misleading
///   in compound commands).
pub fn propose_terminal_patterns(command: &str) -> Vec<PatternProposal> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let contains_chain = trimmed.contains("&&")
        || trimmed.contains("||")
        || trimmed.contains(';')
        || trimmed.contains('|');
    let mut tokens = trimmed.split_whitespace();
    let Some(executable) = tokens.next() else {
        return Vec::new();
    };
    let mut out: Vec<PatternProposal> = Vec::new();
    if !contains_chain
        && let Some(second) = tokens.next()
        && !second.starts_with('-')
    {
        // Exact two-token subcommand (`npm install`). Reject
        // option-like second tokens (`ls -la`) -- the option pattern
        // would feel arbitrary; only positional sub-cmds get a row.
        out.push(PatternProposal::exact(format!("{executable} {second}")));
    }
    out.push(PatternProposal::family(executable.to_string()));
    out
}

/// US-124: one row in the inline pattern picker. `label` is what the
/// user sees ("Always allow `npm install`"); `pattern` is what
/// persists to `paneflow.json` (`"npm install"`, substring-matched
/// against raw_input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternProposal {
    pub label: String,
    pub pattern: String,
}

impl PatternProposal {
    fn exact(s: String) -> Self {
        Self {
            label: format!("Always allow `{s}`"),
            pattern: s,
        }
    }

    fn family(executable: String) -> Self {
        Self {
            label: format!("Always allow `{executable} <any>`"),
            pattern: executable,
        }
    }

    /// "Always allow this tool everywhere" sentinel: empty pattern
    /// persists as the bare-entry / any-input rule (matches every
    /// future call of this kind). Used by non-terminal kinds (read /
    /// edit / search) where command parsing has no signal.
    pub fn everywhere() -> Self {
        Self {
            label: "Always allow this tool everywhere".to_string(),
            pattern: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_returns_the_constant() {
        // Note: the OnceLock is process-wide so we cannot reset between
        // tests. We assert the default is exposed without forcing a
        // specific call order with other tests in this binary.
        let cfg = AgentPanelConfig::default();
        assert_eq!(
            cfg.resolved_max_content_width(),
            AgentPanelConfig::DEFAULT_MAX_CONTENT_WIDTH,
        );
    }

    #[test]
    fn clamp_below_minimum() {
        let cfg = AgentPanelConfig {
            max_content_width: Some(100),
            ..AgentPanelConfig::default()
        };
        assert_eq!(
            cfg.resolved_max_content_width(),
            AgentPanelConfig::MIN_CONTENT_WIDTH_PX,
        );
    }

    #[test]
    fn clamp_above_maximum() {
        let cfg = AgentPanelConfig {
            max_content_width: Some(99_999),
            ..AgentPanelConfig::default()
        };
        assert_eq!(
            cfg.resolved_max_content_width(),
            AgentPanelConfig::MAX_CONTENT_WIDTH_PX,
        );
    }

    #[test]
    fn passes_through_in_range_value() {
        let cfg = AgentPanelConfig {
            max_content_width: Some(900),
            ..AgentPanelConfig::default()
        };
        assert_eq!(cfg.resolved_max_content_width(), 900);
    }

    // ----------------------------------------------------------------
    // US-124: pattern-aware permission resolution. These tests cover
    // the pure helpers; the persistence + UI sides are exercised
    // manually + via the broker round-trip in tests/agent_connection.
    // ----------------------------------------------------------------

    #[test]
    fn matches_pattern_empty_matches_anything() {
        // The empty pattern is the v1 any-input sentinel; preserves
        // the US-111 contract where a bare entry matches every call.
        assert!(matches_pattern("", ""));
        assert!(matches_pattern("", "some raw input"));
    }

    #[test]
    fn matches_pattern_substring_check_is_case_sensitive() {
        assert!(matches_pattern("npm install", "command: npm install react"));
        assert!(!matches_pattern("npm install", "command: NPM INSTALL"));
        assert!(!matches_pattern("npm install", "command: npm test"));
    }

    #[test]
    fn propose_terminal_patterns_basic_two_token_command() {
        let proposals = propose_terminal_patterns("npm install");
        assert_eq!(proposals.len(), 2);
        // Exact subcommand first per AC #1 (`npm install` is more
        // specific than the executable family).
        assert_eq!(proposals[0].pattern, "npm install");
        assert!(proposals[0].label.contains("npm install"));
        assert_eq!(proposals[1].pattern, "npm");
        assert!(proposals[1].label.contains("<any>"));
    }

    #[test]
    fn propose_terminal_patterns_single_token_skips_exact() {
        let proposals = propose_terminal_patterns("git");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].pattern, "git");
    }

    #[test]
    fn propose_terminal_patterns_skips_option_like_second_token() {
        // `ls -la` -> only the `ls <any>` family proposal; `-la` is
        // not a positional subcommand worth promoting to a label.
        let proposals = propose_terminal_patterns("ls -la");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].pattern, "ls");
    }

    #[test]
    fn propose_terminal_patterns_compound_command_offers_family_only() {
        // Pipes / `&&` / `;` should not surface a specific subcommand
        // proposal -- the pattern would mis-match in compound shells.
        let proposals = propose_terminal_patterns("npm install && npm test");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].pattern, "npm");
    }

    #[test]
    fn propose_terminal_patterns_blank_command_returns_empty() {
        assert!(propose_terminal_patterns("").is_empty());
        assert!(propose_terminal_patterns("    ").is_empty());
    }

    #[test]
    fn pattern_proposal_everywhere_is_empty_pattern() {
        let p = PatternProposal::everywhere();
        assert!(p.pattern.is_empty());
        assert!(p.label.contains("everywhere"));
    }
}
