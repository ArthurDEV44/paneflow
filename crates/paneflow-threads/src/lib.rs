#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
    )
)]
//! paneflow-threads: durable storage for Agents-view conversations.
//!
//! Stores every thread as a single row in a SQLite file at
//! `~/.local/share/paneflow/threads/threads.db` (cross-platform via
//! [`dirs::data_local_dir`]). Each row carries the project_id grouping,
//! human-readable `summary`, the spawn-time `agent_id`, and a
//! zstd-compressed JSON BLOB of the full [`Message`] history. The
//! compression default (level 3) trades a few percent of CPU for a 5x
//! size win on typical chat content (see `tests/integration.rs`).
//!
//! Why a separate crate (not a module inside `paneflow-acp`):
//! - `paneflow-acp` is the protocol layer -- it has no business pulling
//!   in `rusqlite` + `zstd` (~1 MB bundled SQLite shaves test/build
//!   feedback latency for everything that only needs ACP types).
//! - Persistence has its own evolutionary axis (schema versioning,
//!   migration). Keeping it separate makes that surface explicit.
//!
//! Public surface (the only stable parts):
//! - [`ThreadStore`] -- the API: open, create_thread, append_message,
//!   load_thread, list_for_project, delete_thread, replace_messages.
//! - [`Message`] / [`MessageRole`] -- the in-memory model UI code uses.
//! - [`ThreadMetadata`] -- the sidebar's view of a thread (no body).
//! - [`OpenOutcome`] -- discriminates "fresh open" vs "recovered from
//!   corruption" so the UI can surface the banner from US-006 AC #8.
//! - [`ThreadStoreError`] -- typed error surface.
//!
//! See `tasks/prd-agents-view.md` US-006 for the source ACs.

pub mod blob;
pub mod error;
pub mod item;
pub mod message;
pub mod paths;
pub mod store;

pub use error::ThreadStoreError;
pub use item::{
    PersistedAssistant, PersistedAssistantChunk, PersistedDiff, PersistedThreadItem,
    PersistedToolCall,
};
pub use message::{Message, MessageRole};
pub use paths::default_db_path;
pub use store::{OpenOutcome, ThreadId, ThreadMetadata, ThreadStore};

/// Re-export of the ACP `ContentBlock` variant since it is the type
/// of `Message::content`. UI crates (US-013's `ThreadView`, US-014's
/// markdown renderer) need to match on the variants without taking
/// a direct `agent-client-protocol` dependency.
pub use agent_client_protocol::schema::ContentBlock;
