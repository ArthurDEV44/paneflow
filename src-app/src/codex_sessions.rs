//! Codex CLI session discovery — reads the on-disk transcript store at
//! `~/.codex/sessions/YYYY/MM/DD/rollout-<TS>-<uuid>.jsonl` and produces
//! unified [`SessionMeta`](crate::agent_sessions::SessionMeta) entries
//! for the sessions popover.
//!
//! Format reference: PR openai/codex#3380 (RolloutItem envelope) and
//! community discussion #3827. The first line of every rollout file is a
//! `type:"session_meta"` envelope with `payload.id`, `payload.cwd`, and
//! `payload.timestamp`. Codex doesn't emit an `ai-title`-equivalent
//! record, so the title falls back to the first
//! `event_msg.user_message.message` content.
//!
//! All filesystem work happens off the GPUI main thread — call
//! [`read_sessions_for_cwd`] from inside `smol::unblock`.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::agent_sessions::{SessionAgent, SessionMeta};

/// Maximum number of leading lines to scan for the first user message.
/// In practice this lands within the first ~10 lines (after
/// `session_meta` + `turn_context` + a few state events). The cap is
/// generous so unusual prelude sequences still produce a label.
const TITLE_SCAN_LIMIT: usize = 256;

/// US-010 (cli-hardening-followup-2026-Q3): per-line byte cap for
/// `read_line`. Same rationale and value as in
/// `claude_sessions::MAX_LINE_BYTES` -- the Codex rollout store
/// (`~/.codex/sessions/YYYY/MM/DD/`) is agent-writable, so a
/// malicious 500 MB single-line JSONL would otherwise allocate
/// fully before the outer scan-count guard fires.
const MAX_LINE_BYTES: u64 = 64 * 1024;

/// Cap rendered first-user-message labels at this character count.
const LABEL_MAX_CHARS: usize = 80;

/// Compute the absolute path of `~/.codex/sessions/`. Returns `None` when
/// `dirs::home_dir()` fails.
pub fn sessions_root() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".codex").join("sessions"))
}

/// Read all Codex CLI sessions whose recorded `cwd` matches the given
/// directory. Returns sessions sorted by timestamp descending (most
/// recent first).
///
/// **Blocking I/O** — call from inside `smol::unblock` or
/// `cx.background_executor`. Codex's flat date-bucketed layout
/// (`YYYY/MM/DD`) means we must scan every rollout file and read the
/// first line to filter by `cwd`. For the typical user (≤ 200 sessions)
/// this is comfortably under 100 ms; cap heavy users via the
/// per-file fast bail-out (we stop after the session_meta line if cwd
/// doesn't match).
pub fn read_sessions_for_cwd(cwd: &str) -> Vec<SessionMeta> {
    let Some(root) = sessions_root() else {
        return Vec::new();
    };

    let mut sessions = Vec::new();
    walk_jsonl_files(&root, &mut |path| {
        if let Some(meta) = read_session_meta(path)
            && meta.cwd == cwd
        {
            sessions.push(meta);
        }
    });
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}

/// Walk Codex's `YYYY/MM/DD/*.jsonl` layout depth-first and invoke
/// `visit` on every `.jsonl` leaf.
fn walk_jsonl_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_jsonl_files(&path, visit);
        } else if is_jsonl_file(&path) {
            visit(&path);
        }
    }
}

fn is_jsonl_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
}

/// Read the head of a rollout file: extract the `session_meta` envelope
/// (line 1) and the first user message (typically a few lines later).
fn read_session_meta(path: &Path) -> Option<SessionMeta> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = String::new();

    // Line 1 must be session_meta or we skip the file.
    buf.clear();
    // US-010 (cli-hardening-followup-2026-Q3): cap line read at
    // MAX_LINE_BYTES. Truncated line fails serde_json parse below
    // and the file is skipped -- same outcome as a malformed line.
    let n = reader
        .by_ref()
        .take(MAX_LINE_BYTES)
        .read_line(&mut buf)
        .ok()?;
    if n == 0 {
        return None;
    }
    if n as u64 == MAX_LINE_BYTES && !buf.ends_with('\n') {
        log::warn!(
            target: "paneflow_app::codex_sessions",
            "session JSONL line truncated at {} bytes for {} -- skipping file",
            MAX_LINE_BYTES,
            path.display(),
        );
        return None;
    }
    let first_value: serde_json::Value = serde_json::from_str(buf.trim_end()).ok()?;
    if first_value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = first_value.get("payload")?;
    let session_id = payload.get("id").and_then(|v| v.as_str())?.to_string();
    let cwd = payload.get("cwd").and_then(|v| v.as_str())?.to_string();
    if cwd.is_empty() {
        return None;
    }
    // session_id lands verbatim in `codex resume <id>`, so hold it to the
    // strict `^[A-Za-z0-9_-]+$` allow-list (Codex ids are UUIDs): rejects a
    // `\r`/`\n` that would submit injected text and a `;`/space that would
    // chain a second shell command. cwd is display-only today but a future
    // `cd <cwd>` prefix would inherit the gap, and a path legitimately carries
    // `/` + spaces, so keep the control-char guard for it. Mirrors (and
    // tightens) the guard in `opencode_sessions::record_to_session`.
    if !crate::agent_sessions::is_valid_session_id(&session_id)
        || cwd.chars().any(|c| c.is_control())
    {
        log::warn!(
            "codex_sessions: dropped {} -- payload carries an invalid id or control chars in cwd",
            path.display(),
        );
        return None;
    }
    // Inner timestamp is the session start; outer envelope timestamp is
    // the moment the file was opened. They're typically within seconds
    // of each other — prefer the inner (session-relative) value.
    let timestamp = payload
        .get("timestamp")
        .and_then(|v| v.as_str())
        .or_else(|| first_value.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let summary = scan_first_user_message(&mut reader);

    Some(SessionMeta {
        agent: SessionAgent::Codex,
        session_id,
        timestamp,
        cwd,
        // Codex doesn't record git branch in `session_meta`. Leave empty
        // so the row collapses to `<time>` only — matches what the user
        // sees when they run `codex resume`.
        git_branch: String::new(),
        summary,
    })
}

/// Scan up to [`TITLE_SCAN_LIMIT`] lines looking for the first
/// `event_msg.user_message`. Codex wraps user input as
/// `{"type":"event_msg","payload":{"type":"user_message","message":"..."}}`.
///
/// Signature is concrete on `BufReader<File>` rather than the
/// generic `R: BufRead` it used to be: the `by_ref().take()`
/// pattern needed by US-010 for the per-line byte cap fails to
/// type-check against `&mut R` (the compiler auto-derefs to `R`
/// and the move blocks the borrow). The only call site already
/// passes a `BufReader<File>`, so the generic was vestigial.
fn scan_first_user_message(reader: &mut BufReader<fs::File>) -> Option<String> {
    let mut buf = String::new();
    for _ in 0..TITLE_SCAN_LIMIT {
        buf.clear();
        // US-010 (cli-hardening-followup-2026-Q3): cap each line read.
        // Oversize lines fall through to `serde_json::from_str` which
        // errors and the loop `continue`s -- the scan moves on to the
        // next line without OOMing.
        let n = reader
            .by_ref()
            .take(MAX_LINE_BYTES)
            .read_line(&mut buf)
            .ok()?;
        if n == 0 {
            return None;
        }
        let trimmed = buf.trim_end();
        if !trimmed.starts_with('{') {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("event_msg") {
            continue;
        }
        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };
        if payload.get("type").and_then(|v| v.as_str()) != Some("user_message") {
            continue;
        }
        if let Some(message) = payload.get("message").and_then(|v| v.as_str())
            && let Some(cleaned) = clean_user_message(message)
        {
            return Some(cleaned);
        }
    }
    None
}

fn clean_user_message(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let collapsed: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LABEL_MAX_CHARS {
        Some(collapsed)
    } else {
        let mut out: String = collapsed.chars().take(LABEL_MAX_CHARS).collect();
        out.push('…');
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduce the real Codex rollout sequence observed in the wild:
    /// line 1 is `session_meta`, then a few state events, then the first
    /// `event_msg` `user_message`.
    #[test]
    fn read_session_meta_extracts_envelope_and_first_user_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"timestamp":"2026-04-26T13:11:10.338Z","type":"session_meta","payload":{"id":"019dc9ea-38d7-7372-9cc4-253ce944d41b","timestamp":"2026-04-26T13:11:03.694Z","cwd":"/home/arthur/dev/paneflow","originator":"codex-tui","cli_version":"0.123.0","model_provider":"openai"}}"#,
                "\n",
                r#"{"type":"turn_context","payload":{"model":"gpt-5"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-26T13:11:10.345Z","type":"event_msg","payload":{"type":"user_message","message":"Explique le projet stp","images":[]}}"#,
                "\n",
            ),
        )
        .expect("write fixture");

        let meta = read_session_meta(&path).expect("envelope extracted");
        assert_eq!(meta.agent, SessionAgent::Codex);
        assert_eq!(meta.session_id, "019dc9ea-38d7-7372-9cc4-253ce944d41b");
        assert_eq!(meta.cwd, "/home/arthur/dev/paneflow");
        assert_eq!(meta.timestamp, "2026-04-26T13:11:03.694Z");
        assert!(meta.git_branch.is_empty());
        assert_eq!(meta.summary.as_deref(), Some("Explique le projet stp"));
    }

    #[test]
    fn read_session_meta_returns_none_for_non_session_meta_first_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-codex.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}
"#,
        )
        .expect("write fixture");
        assert!(read_session_meta(&path).is_none());
    }

    #[test]
    fn read_session_meta_returns_none_when_payload_missing_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("no-cwd.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"x","timestamp":"2026-04-26T13:11:03.694Z"}}
"#,
        )
        .expect("write fixture");
        assert!(read_session_meta(&path).is_none());
    }

    #[test]
    fn user_message_label_is_truncated_with_ellipsis_when_long() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("long-prompt.jsonl");
        let long_prompt = "x".repeat(200);
        let session_meta_line = r#"{"type":"session_meta","payload":{"id":"s","cwd":"/p","timestamp":"2026-04-26T13:00:00Z"}}"#;
        let user_msg_line = format!(
            r#"{{"type":"event_msg","payload":{{"type":"user_message","message":"{long_prompt}"}}}}"#
        );
        std::fs::write(&path, format!("{session_meta_line}\n{user_msg_line}\n"))
            .expect("write fixture");
        let meta = read_session_meta(&path).expect("meta");
        let summary = meta.summary.expect("summary");
        assert_eq!(summary.chars().count(), LABEL_MAX_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn session_id_control_char_guard() {
        // payload.id carries CR+LF + an injected shell command. Without
        // the guard, the id flows into `codex resume <id>` and submits
        // `rm -rf ~` as a separate PTY command.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("malicious.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"abc\r\nrm -rf ~","cwd":"/tmp/proj","timestamp":"2026-04-26T13:11:03.694Z"}}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        assert!(
            read_session_meta(&path).is_none(),
            "session with control chars in payload.id must be dropped"
        );
    }

    #[test]
    fn session_id_legitimate_uuid_passes_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ok.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"019dc9ea-38d7-7372-9cc4-253ce944d41b","cwd":"/tmp/proj","timestamp":"2026-04-26T13:11:03.694Z"}}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        let meta = read_session_meta(&path).expect("legitimate UUID must pass the guard");
        assert_eq!(meta.session_id, "019dc9ea-38d7-7372-9cc4-253ce944d41b");
    }

    #[test]
    fn cwd_control_char_guard() {
        // Same class of injection as session_id, just one field over.
        // cwd is display-only today but a future `cd <cwd>` prefix
        // would inherit the gap without any git-blame signal.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("malicious-cwd.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"019dc9ea-38d7-7372-9cc4-253ce944d41b","cwd":"/tmp/proj\r\nrm -rf ~","timestamp":"2026-04-26T13:11:03.694Z"}}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        assert!(
            read_session_meta(&path).is_none(),
            "session with control chars in cwd must be dropped"
        );
    }
}
