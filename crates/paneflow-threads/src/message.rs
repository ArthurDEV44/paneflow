//! In-memory representation of one chat turn, persisted as JSON inside
//! the per-thread zstd BLOB.
//!
//! Why reuse [`agent_client_protocol::schema::ContentBlock`]:
//! - The ACP wire shape already covers text + image + audio + resource
//!   links + embedded resources -- exactly the surface a chat thread
//!   needs. Defining a parallel Paneflow content enum would be
//!   duplication that drifts over time.
//! - ACP's `ContentBlock` is `#[non_exhaustive]` and derives
//!   `Serialize`/`Deserialize` with a stable `type` discriminator. New
//!   variants in future ACP releases stay forward-compatible: serde
//!   round-trips an unknown tag as long as we keep enabling the
//!   appropriate features.
//!
//! Schema evolution: every new field on [`Message`] MUST carry
//! `#[serde(default)]` so older rows decompress cleanly. Adding a new
//! variant to [`MessageRole`] is a breaking change -- gate it on a
//! `schema_version` bump and a migration.

use agent_client_protocol::schema::ContentBlock;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Who authored a [`Message`]. `System` is reserved for future use
/// (e.g. ACP-emitted notices, "agent restarted" markers). The current
/// agents view only emits `User` and `Assistant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// One persisted chat turn. The vector of these is what [`crate::ThreadStore`]
/// serialises to JSON and compresses with zstd.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    /// Unix epoch milliseconds (UTC). Stored as `i64` to round-trip
    /// JSON cleanly (no f64 precision loss for the next ~290 million
    /// years).
    pub created_at: i64,
}

impl Message {
    /// Build a message with [`created_at`](Self::created_at) set to
    /// "now". Falls back to `0` if the clock predates the Unix epoch
    /// (impossible on any sane system; we still avoid panicking per
    /// the workspace clippy policy).
    pub fn new(role: MessageRole, content: Vec<ContentBlock>) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| {
                let millis = d.as_millis();
                i64::try_from(millis).unwrap_or(i64::MAX)
            })
            .unwrap_or(0);
        Self {
            role,
            content,
            created_at,
        }
    }

    /// Convenience: a user-authored text message.
    pub fn user_text(text: impl Into<String>) -> Self {
        use agent_client_protocol::schema::TextContent;
        Self::new(
            MessageRole::User,
            vec![ContentBlock::Text(TextContent::new(text))],
        )
    }

    /// Convenience: an assistant-authored text message.
    pub fn assistant_text(text: impl Into<String>) -> Self {
        use agent_client_protocol::schema::TextContent;
        Self::new(
            MessageRole::Assistant,
            vec![ContentBlock::Text(TextContent::new(text))],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::TextContent;

    #[test]
    fn user_text_round_trips_through_json() {
        let msg = Message::user_text("hello");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, MessageRole::User);
        assert_eq!(parsed.content.len(), 1);
        match &parsed.content[0] {
            ContentBlock::Text(TextContent { text, .. }) => assert_eq!(text, "hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn created_at_is_monotonic_per_message() {
        let a = Message::user_text("first");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = Message::user_text("second");
        assert!(b.created_at >= a.created_at);
    }

    #[test]
    fn role_serializes_snake_case() {
        let user_json = serde_json::to_string(&MessageRole::User).unwrap();
        let asst_json = serde_json::to_string(&MessageRole::Assistant).unwrap();
        let sys_json = serde_json::to_string(&MessageRole::System).unwrap();
        assert_eq!(user_json, "\"user\"");
        assert_eq!(asst_json, "\"assistant\"");
        assert_eq!(sys_json, "\"system\"");
    }
}
