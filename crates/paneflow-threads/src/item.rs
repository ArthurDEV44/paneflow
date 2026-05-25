//! Persisted timeline entries.
//!
//! A thread's on-disk representation is a `Vec<PersistedThreadItem>` —
//! a flat ordered sequence of three kinds of rows:
//! - a chat message from the user (or a system note),
//! - one assistant turn made of an ordered list of text + thought
//!   chunks (matches the Zed `AssistantMessage { chunks: Vec<...> }`
//!   model),
//! - a top-level tool call (read / search / edit / execute / ...).
//!
//! The Zed inline layout interleaves thoughts inside the assistant
//! message rather than grouping them into a "reasoning card", which
//! is why thoughts are chunks of an `Assistant` row instead of their
//! own variant.

use crate::message::Message;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One row in the persisted timeline. Variants mirror the in-memory
/// `ThreadItem` enum on the UI side but are explicitly plain-data
/// (no GPUI entities) so they can round-trip through serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistedThreadItem {
    /// A user (or system) chat message. Assistant messages are stored
    /// as [`PersistedThreadItem::Assistant`] instead so their inline
    /// thoughts survive the round-trip.
    Message(Message),
    /// One assistant turn: an ordered sequence of text and thought
    /// chunks. Tool calls live as separate top-level entries.
    Assistant(PersistedAssistant),
    /// A top-level tool call (any kind: Read / Search / Edit /
    /// Execute / ...).
    Tool(PersistedToolCall),
}

/// Persisted snapshot of one assistant turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAssistant {
    pub chunks: Vec<PersistedAssistantChunk>,
}

/// One chunk inside an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistedAssistantChunk {
    /// Regular markdown body the model emitted.
    Text { text: String },
    /// One thinking burst. `signature` carries the Anthropic
    /// extended-thinking signature so future tool-loops can re-prove
    /// the chain-of-thought wasn't tampered with.
    Thought {
        text: String,
        #[serde(default)]
        signature: Option<String>,
    },
}

/// Persisted snapshot of an in-memory `ToolCallSnapshot` (defined
/// in `src-app/src/agents/runtime.rs`). All fields are owned data so
/// the renderer can reconstruct the snapshot without re-running the
/// ACP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedToolCall {
    pub id: String,
    pub title: String,
    /// Lower-case ACP-style kind: `"read" | "edit" | "delete" |
    /// "move" | "search" | "execute" | "think" | "fetch" |
    /// "switch_mode" | "other"`. Unknown values render as `Other`.
    pub tool_kind: String,
    /// Lower-case status: `"pending" | "waiting_for_confirmation" |
    /// "rejected" | "in_progress" | "completed" | "failed" |
    /// "canceled"`. Unknown values map to `Pending`.
    pub status: String,
    #[serde(default)]
    pub raw_input_pretty: Option<String>,
    #[serde(default)]
    pub raw_output_pretty: Option<String>,
    #[serde(default)]
    pub content_text: String,
    #[serde(default)]
    pub diffs: Vec<PersistedDiff>,
    /// True when the user has resolved this tool call's diffs via
    /// Keep All or Reject All. Persisted so the activity-bar review
    /// footer stays dismissed across reloads (revisiting the thread,
    /// restarting the app). `#[serde(default)]` keeps blobs written
    /// before 2026-05-25 readable; they decode as `reviewed = false`,
    /// which means the footer reappears once on first load -- the
    /// expected upgrade behaviour, not a regression.
    #[serde(default)]
    pub reviewed: bool,
}

/// File-level diff stored on a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedDiff {
    pub path: PathBuf,
    pub old_text: Option<String>,
    pub new_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[test]
    fn message_variant_round_trips() {
        let item = PersistedThreadItem::Message(Message::user_text("hi"));
        let json = serde_json::to_string(&item).unwrap();
        let parsed: PersistedThreadItem = serde_json::from_str(&json).unwrap();
        match parsed {
            PersistedThreadItem::Message(m) => assert_eq!(m.content.len(), 1),
            other => panic!("expected Message, got {other:?}"),
        }
    }

    #[test]
    fn tool_variant_round_trips_with_diffs() {
        let item = PersistedThreadItem::Tool(PersistedToolCall {
            id: "tool-1".into(),
            title: "Update src/main.rs".into(),
            tool_kind: "edit".into(),
            status: "completed".into(),
            raw_input_pretty: None,
            raw_output_pretty: None,
            content_text: String::new(),
            diffs: vec![PersistedDiff {
                path: PathBuf::from("src/main.rs"),
                old_text: Some("a\nb".into()),
                new_text: "a\nB".into(),
            }],
            reviewed: false,
        });
        let json = serde_json::to_string(&item).unwrap();
        let parsed: PersistedThreadItem = serde_json::from_str(&json).unwrap();
        match parsed {
            PersistedThreadItem::Tool(t) => {
                assert_eq!(t.diffs.len(), 1);
                assert_eq!(t.diffs[0].path, PathBuf::from("src/main.rs"));
            }
            other => panic!("expected Tool, got {other:?}"),
        }
    }

    #[test]
    fn assistant_variant_round_trips_mixed_chunks() {
        let item = PersistedThreadItem::Assistant(PersistedAssistant {
            chunks: vec![
                PersistedAssistantChunk::Text {
                    text: "Let me look it up".into(),
                },
                PersistedAssistantChunk::Thought {
                    text: "internal reasoning".into(),
                    signature: Some("sig-abc".into()),
                },
                PersistedAssistantChunk::Text {
                    text: "Here is the answer.".into(),
                },
            ],
        });
        let json = serde_json::to_string(&item).unwrap();
        let parsed: PersistedThreadItem = serde_json::from_str(&json).unwrap();
        match parsed {
            PersistedThreadItem::Assistant(a) => {
                assert_eq!(a.chunks.len(), 3);
                assert!(matches!(
                    a.chunks[1],
                    PersistedAssistantChunk::Thought { .. }
                ));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let json = r#"{
            "kind": "tool",
            "id": "x",
            "title": "t",
            "tool_kind": "edit",
            "status": "pending"
        }"#;
        let parsed: PersistedThreadItem = serde_json::from_str(json).unwrap();
        match parsed {
            PersistedThreadItem::Tool(t) => {
                assert_eq!(t.raw_input_pretty, None);
                assert_eq!(t.content_text, "");
                assert!(t.diffs.is_empty());
                assert!(!t.reviewed);
            }
            other => panic!("expected Tool, got {other:?}"),
        }
    }

    #[test]
    fn reviewed_flag_round_trips() {
        let item = PersistedThreadItem::Tool(PersistedToolCall {
            id: "tool-2".into(),
            title: "Edit lib.rs".into(),
            tool_kind: "edit".into(),
            status: "completed".into(),
            raw_input_pretty: None,
            raw_output_pretty: None,
            content_text: String::new(),
            diffs: Vec::new(),
            reviewed: true,
        });
        let json = serde_json::to_string(&item).unwrap();
        let parsed: PersistedThreadItem = serde_json::from_str(&json).unwrap();
        match parsed {
            PersistedThreadItem::Tool(t) => assert!(t.reviewed),
            other => panic!("expected Tool, got {other:?}"),
        }
    }
}
