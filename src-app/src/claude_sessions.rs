//! Claude Code session discovery — reads the on-disk session store at
//! `~/.claude/projects/<slug>/<uuid>.jsonl` and produces unified
//! [`SessionMeta`](crate::agent_sessions::SessionMeta) entries for the
//! sessions popover.
//!
//! There is no public Claude Code API for listing sessions (issue #34318);
//! the `.jsonl` files are the source of truth. Each line is one event;
//! the *first* line that carries `cwd` is the session envelope. The
//! LLM-generated `type:"ai-title"` record (when present) provides the
//! human-readable label that the `claude --resume` picker shows.
//!
//! All filesystem work happens off the GPUI main thread — call
//! [`read_sessions_for_cwd`] from inside `smol::unblock`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agent_sessions::{SessionAgent, SessionMeta};

/// Maximum number of leading lines to scan for envelope + title. The
/// first lines of a Claude Code session file are typically
/// `permission-mode` and `file-history-snapshot` records with no `cwd`;
/// the actual user/assistant events start on line 3+. The `ai-title`
/// record (when present) is usually around line 9 but can appear later.
/// 256 covers >95% of files in the wild without the scan being visible
/// to the user.
const TITLE_SCAN_LIMIT: usize = 256;

/// Cap rendered first-user-message labels at this character count to keep
/// the popover row from overflowing horizontally.
const LABEL_MAX_CHARS: usize = 80;

/// First-line envelope. Tolerant: any missing field falls back via
/// `serde(default)` so the parser never bails on a forward-compatible schema
/// change.
#[derive(Debug, Deserialize)]
struct FirstLineEnvelope {
    #[serde(default, rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    cwd: String,
    #[serde(default, rename = "gitBranch")]
    git_branch: String,
}

/// Convert an absolute path into the slug Claude Code uses as the directory
/// name under `~/.claude/projects/`. Algorithm: every `/` becomes `-`,
/// every `\` becomes `-`. Spaces, dots, and non-ASCII characters are
/// preserved literally — there is no percent-encoding or hashing.
pub fn slug_for_cwd(cwd: &str) -> String {
    cwd.replace(['/', '\\'], "-")
}

/// Compute the absolute path of `~/.claude/projects/<slug>/`. Returns
/// `None` when `dirs::home_dir()` fails (no `$HOME` / `%USERPROFILE%`).
pub fn project_dir_for_cwd(cwd: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let slug = slug_for_cwd(cwd);
    Some(home.join(".claude").join("projects").join(slug))
}

/// Read all Claude Code session metadata for the given working directory.
/// Sessions are sorted by timestamp descending (most recent first) and
/// only those whose first-line `cwd` matches `cwd` exactly are kept —
/// dedupes the rare slug collision where two distinct paths produce the
/// same directory name (`/a/b-c` and `/a/b/c` both slug to `-a-b-c`).
///
/// **Blocking I/O** — call from inside `smol::unblock` or
/// `cx.background_executor`. Never invoke on the GPUI main thread.
pub fn read_sessions_for_cwd(cwd: &str) -> Vec<SessionMeta> {
    let Some(project_dir) = project_dir_for_cwd(cwd) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&project_dir) else {
        return Vec::new();
    };

    let mut sessions: Vec<SessionMeta> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !is_jsonl_file(&path) {
                return None;
            }
            read_session_meta(&path).filter(|meta| meta.cwd == cwd)
        })
        .collect();

    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}

fn is_jsonl_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
}

/// Read the head of a `.jsonl` and collect everything we need for a UI
/// row in a single pass: the first envelope carrying `cwd`, the
/// LLM-generated `ai-title` (when present), and the cleaned first
/// `type:"user"` message (used as a fallback title when `ai-title` is
/// absent — typical of sessions older than that feature's introduction).
///
/// Title priority:
/// 1. `type:"ai-title"` → `aiTitle` field. Matches what the
///    `claude --resume` picker shows for newer sessions.
/// 2. First `type:"user"` message, with `<command-*>` boilerplate
///    collapsed into `/<name> <args>` when present.
fn read_session_meta(path: &Path) -> Option<SessionMeta> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = String::new();

    let mut envelope: Option<FirstLineEnvelope> = None;
    let mut ai_title: Option<String> = None;
    let mut user_fallback: Option<String> = None;

    for _ in 0..TITLE_SCAN_LIMIT {
        buf.clear();
        let n = reader.read_line(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        let trimmed = buf.trim_end();
        if !trimmed.starts_with('{') {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if envelope.is_none()
            && value
                .get("cwd")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            && let Ok(parsed) = serde_json::from_value::<FirstLineEnvelope>(value.clone())
            && !parsed.session_id.is_empty()
            && !parsed.cwd.is_empty()
            && !parsed.session_id.chars().any(|c| c.is_control())
        {
            envelope = Some(parsed);
        }

        match value.get("type").and_then(|v| v.as_str()) {
            Some("ai-title") => {
                if let Some(title) = value.get("aiTitle").and_then(|v| v.as_str())
                    && !title.is_empty()
                {
                    ai_title = Some(title.to_string());
                    if envelope.is_some() {
                        break;
                    }
                }
            }
            Some("user") if user_fallback.is_none() => {
                if let Some(text) = extract_user_content(&value)
                    && let Some(cleaned) = clean_user_message(&text)
                {
                    user_fallback = Some(cleaned);
                }
            }
            _ => {}
        }
    }

    let envelope = envelope?;
    let summary = ai_title.or(user_fallback);

    Some(SessionMeta {
        agent: SessionAgent::Claude,
        session_id: envelope.session_id,
        timestamp: envelope.timestamp,
        cwd: envelope.cwd,
        git_branch: envelope.git_branch,
        summary,
    })
}

fn extract_user_content(line: &serde_json::Value) -> Option<String> {
    let content = line.get("message")?.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text")
                && let Some(text) = block.get("text").and_then(|v| v.as_str())
                && !text.is_empty()
            {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn clean_user_message(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(name) = extract_xml_block(trimmed, "command-name") {
        let args = extract_xml_block(trimmed, "command-args").unwrap_or_default();
        let joined = if args.is_empty() {
            name
        } else {
            format!("{name} {args}")
        };
        return Some(truncate_label(&joined));
    }

    Some(truncate_label(trimmed))
}

fn extract_xml_block(haystack: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = haystack.find(&open)? + open.len();
    let end = haystack[start..].find(&close)? + start;
    Some(haystack[start..end].trim().to_string())
}

fn truncate_label(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LABEL_MAX_CHARS {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(LABEL_MAX_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_unix_path() {
        assert_eq!(slug_for_cwd("/home/alice/myapp"), "-home-alice-myapp");
    }

    #[test]
    fn slug_preserves_spaces() {
        assert_eq!(
            slug_for_cwd("/home/alice/my project"),
            "-home-alice-my project"
        );
    }

    #[test]
    fn slug_preserves_dots() {
        assert_eq!(slug_for_cwd("/home/alice/.config"), "-home-alice-.config");
    }

    #[test]
    fn slug_windows_path() {
        assert_eq!(
            slug_for_cwd("C:\\Users\\alice\\myapp"),
            "C:-Users-alice-myapp"
        );
    }

    #[test]
    fn slug_root() {
        assert_eq!(slug_for_cwd("/"), "-");
    }

    #[test]
    fn read_session_meta_skips_leading_metadata_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join("aaaaaaaa-1111-2222-3333-444444444444.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"permission-mode","permissionMode":"default","sessionId":"aaaaaaaa-1111-2222-3333-444444444444"}"#,
                "\n",
                r#"{"type":"file-history-snapshot","messageId":"x","snapshot":{"trackedFileBackups":{}},"isSnapshotUpdate":false}"#,
                "\n",
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"hi"},"uuid":"x","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"aaaaaaaa-1111-2222-3333-444444444444","version":"2.1.119","gitBranch":"main"}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"Implement feature X","sessionId":"aaaaaaaa-1111-2222-3333-444444444444"}"#,
                "\n",
            ),
        )
        .expect("write fixture");

        let meta = read_session_meta(&path).expect("envelope extracted");
        assert_eq!(meta.agent, SessionAgent::Claude);
        assert_eq!(meta.session_id, "aaaaaaaa-1111-2222-3333-444444444444");
        assert_eq!(meta.cwd, "/tmp/proj");
        assert_eq!(meta.timestamp, "2026-04-26T13:38:41.095Z");
        assert_eq!(meta.git_branch, "main");
        assert_eq!(meta.summary.as_deref(), Some("Implement feature X"));
    }

    #[test]
    fn ai_title_wins_over_first_user_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ordering.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"first user message body"},"uuid":"u","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"s"}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"Fix the thing","sessionId":"s"}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        let meta = read_session_meta(&path).expect("meta");
        assert_eq!(meta.summary.as_deref(), Some("Fix the thing"));
    }

    #[test]
    fn falls_back_to_first_user_message_when_no_ai_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("legacy.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"Refactor the auth flow"},"uuid":"u","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"s"}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        let meta = read_session_meta(&path).expect("meta");
        assert_eq!(meta.summary.as_deref(), Some("Refactor the auth flow"));
    }

    #[test]
    fn cleans_slash_command_boilerplate_in_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slash.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"<command-message>implement-story</command-message>\n<command-name>/implement-story</command-name>\n<command-args>@tasks/prd-x.md US-001</command-args>"},"uuid":"u","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"s"}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        let meta = read_session_meta(&path).expect("meta");
        assert_eq!(
            meta.summary.as_deref(),
            Some("/implement-story @tasks/prd-x.md US-001")
        );
    }

    #[test]
    fn truncate_label_caps_long_text() {
        let long = "a".repeat(120);
        let label = truncate_label(&long);
        assert_eq!(label.chars().count(), LABEL_MAX_CHARS + 1);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn read_session_meta_returns_none_when_no_cwd_envelope_in_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("no-cwd.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"permission-mode","permissionMode":"default","sessionId":"x"}
{"type":"file-history-snapshot","snapshot":{}}
"#,
        )
        .expect("write fixture");
        assert!(read_session_meta(&path).is_none());
    }

    #[test]
    fn session_id_control_char_guard() {
        // sessionId carries CR+LF + an injected shell command. Without
        // the guard, this id would flow into `claude --resume <id>` and
        // submit `rm -rf ~` as a separate PTY command.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("malicious.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"hi"},"uuid":"u","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"abc\r\nrm -rf ~","version":"2.1.119","gitBranch":"main"}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        assert!(
            read_session_meta(&path).is_none(),
            "session with control chars in sessionId must be dropped"
        );
    }

    #[test]
    fn session_id_legitimate_uuid_passes_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ok.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":"hi"},"uuid":"u","timestamp":"2026-04-26T13:38:41.095Z","cwd":"/tmp/proj","sessionId":"550e8400-e29b-41d4-a716-446655440000","version":"2.1.119","gitBranch":"main"}"#,
                "\n",
            ),
        )
        .expect("write fixture");
        let meta = read_session_meta(&path).expect("legitimate UUID must pass the guard");
        assert_eq!(meta.session_id, "550e8400-e29b-41d4-a716-446655440000");
    }
}
