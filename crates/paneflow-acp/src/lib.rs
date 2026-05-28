#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
    )
)]
//! paneflow-acp: ACP (Agent Client Protocol) wrappers for Paneflow.
//!
//! This crate is the single place in Paneflow that knows how to spawn an
//! ACP-compatible CLI agent and how to handle the agent's inbound
//! requests (permission prompts, file ops, terminal ops). Centralization
//! is required so that:
//!
//! 1. Every spawn site funnels through one `CLAUDECODE`-scrubbing entry
//!    point ([`spawn::spawn_acp_agent`]). Without the scrub, the spike
//!    discovered that `claude-code-acp` refuses to launch when inherited
//!    from a host Claude Code session.
//! 2. All file writes are sandboxed to the session's cwd via a single
//!    canonicalization helper ([`file_ops::resolve_inside_cwd`]) -- no
//!    symlink-escape vector by construction.
//! 3. Streaming pacing uses one deterministic buffer
//!    ([`streaming::StreamingBuffer`]) so UI behaviour matches across
//!    the active-thread view and any future preview surfaces.
//!
//! See `tasks/prd-agents-view.md` US-001..US-003 for the foundation
//! stories; US-018 will plug in the real PTY backend behind
//! [`terminal::TerminalSpawner`].

pub mod auth;
pub mod client;
pub mod connection;
pub mod discovery;
pub mod file_ops;
pub mod permission;
pub mod session;
pub mod spawn;
pub mod streaming;
pub mod terminal;

pub use auth::{
    canonical_login_command, is_auth_required, is_signed_in, AuthMethodSummary, AuthRequirement,
    MissingAgentsGuidance, MISSING_AGENTS_GUIDANCE,
};
pub use client::{connect_with_handlers, ClientConfig, NoopNotificationSink, NotificationSink};
pub use connection::{AcpConnection, AgentConnection};
pub use discovery::{AgentDiscovery, AgentKind, DiscoveredAgent, PathProbe, SystemPathProbe};
pub use file_ops::{handle_read, handle_write, FileOpError};
pub use permission::{
    always_allow, always_deny, map_decision, BoxFuture, FnPermissionCallback, PermissionCallback,
    PermissionDecision,
};
pub use session::{SessionMeta, SessionRegistry};
pub use spawn::{scrub_claudecode_env, spawn_acp_agent};
pub use streaming::{PacingConfig, StreamingBuffer};
pub use terminal::{
    handle_create as handle_create_terminal, handle_kill as handle_kill_terminal,
    handle_output as handle_output_terminal, handle_release as handle_release_terminal,
    handle_wait_for_exit as handle_wait_for_exit_terminal, TerminalError, TerminalOutputSnapshot,
    TerminalRegistry, TerminalSession, TerminalSpawner,
};
