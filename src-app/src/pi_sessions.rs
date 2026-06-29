//! Pi Coding Agent session discovery.
//!
//! Pi documents its local session store under `~/.pi/agent/sessions/` with a
//! JSONL header record:
//! `{"type":"session","version":3,"id":"...","timestamp":"...","cwd":"..."}`.
//! The reader uses that public contract, never Pi internals beyond it, and
//! normalises matching sessions into [`SessionMeta`](crate::agent_sessions::SessionMeta).

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent_sessions::{SessionAgent, SessionMeta};

const MAX_WALK_DEPTH: usize = 10;
const MAX_HEADER_LINES: usize = 256;
const MAX_LINE_BYTES: usize = 256 * 1024;

pub(crate) fn sessions_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".pi").join("agent").join("sessions"))
}

pub(crate) fn read_sessions_for_cwd_with_omitted(cwd: &str) -> (Vec<SessionMeta>, usize) {
    let Some(root) = sessions_root() else {
        return (Vec::new(), 0);
    };
    read_sessions_under_root(&root, cwd)
}

fn read_sessions_under_root(root: &Path, cwd: &str) -> (Vec<SessionMeta>, usize) {
    if !root.is_dir() {
        return (Vec::new(), 0);
    }
    let paths = jsonl_files(root);
    let sessions = paths
        .iter()
        .filter_map(|path| read_session_meta(path))
        .filter(|meta| crate::agent_sessions::cwd_matches(&meta.cwd, cwd));
    crate::agent_sessions::collect_recent_sessions(
        sessions,
        crate::agent_sessions::SIDEBAR_SESSION_RETAINED_PER_SOURCE,
    )
}

fn jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth > MAX_WALK_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                queue.push_back((path, depth + 1));
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

fn read_session_meta(path: &Path) -> Option<SessionMeta> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut header: Option<PiHeader> = None;
    let mut summary: Option<String> = None;

    for _ in 0..MAX_HEADER_LINES {
        let mut line = String::new();
        let read = reader.read_line(&mut line).ok()?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_LINE_BYTES {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if header.is_none() {
            header = PiHeader::from_value(&value);
        }
        if summary.is_none() {
            summary = user_summary_from_value(&value);
        }
        if header.is_some() && summary.is_some() {
            break;
        }
    }

    let header = header?;
    Some(SessionMeta {
        agent: SessionAgent::Pi,
        session_id: header.id,
        timestamp: header.timestamp,
        cwd: header.cwd,
        git_branch: String::new(),
        summary,
        model: None,
        usage: None,
    })
}

struct PiHeader {
    id: String,
    timestamp: String,
    cwd: String,
}

impl PiHeader {
    fn from_value(value: &Value) -> Option<Self> {
        if value.get("type").and_then(Value::as_str) != Some("session") {
            return None;
        }
        let id = value.get("id").and_then(Value::as_str)?.to_string();
        let cwd = value.get("cwd").and_then(Value::as_str)?.to_string();
        if !crate::agent_sessions::is_valid_session_id(&id)
            || cwd.is_empty()
            || cwd.chars().any(char::is_control)
        {
            return None;
        }
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Some(Self { id, timestamp, cwd })
    }
}

fn user_summary_from_value(value: &Value) -> Option<String> {
    let role = value
        .get("message")
        .and_then(|message| message.get("role"))
        .or_else(|| value.get("role"))
        .and_then(Value::as_str)?;
    if role != "user" {
        return None;
    }
    let content = value
        .get("message")
        .and_then(|message| message.get("content"))
        .or_else(|| value.get("content"))?;
    content_to_string(content)
        .map(|s| s.chars().take(120).collect::<String>())
        .filter(|s| !s.is_empty())
}

fn content_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
                {
                    let text = text.trim();
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
            }
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        Value::Object(obj) => obj
            .get("text")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_documented_pi_jsonl_header_and_filters_by_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("nested");
        fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("session.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"type":"session","version":3,"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2026-06-29T09:10:11Z","cwd":"/repo"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"user","content":"Ship the sidebar sessions"}} "#
            ),
        )
        .unwrap();

        let (sessions, omitted) = read_sessions_under_root(dir.path(), "/repo");
        assert_eq!(omitted, 0);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, SessionAgent::Pi);
        assert_eq!(
            sessions[0].summary.as_deref(),
            Some("Ship the sidebar sessions")
        );
        assert!(read_sessions_under_root(dir.path(), "/other").0.is_empty());
    }

    #[test]
    fn drops_header_with_unsafe_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"ses_x; rm -rf ~","timestamp":"2026-06-29T09:10:11Z","cwd":"/repo"}"#,
        )
        .unwrap();
        assert!(read_sessions_under_root(dir.path(), "/repo").0.is_empty());
    }
}
