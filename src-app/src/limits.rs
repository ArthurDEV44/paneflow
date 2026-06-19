//! Centralized ingress/egress size caps (EP-003, US-013).
//!
//! Every bound that protects an allocation, a parse, or a spawn from untrusted
//! or hand-edited input is gathered here so a read cap and its matching write
//! cap stay in sync - the "validate on both directions" invariant can't
//! silently rot when the two numbers live a screen apart in different files.
//!
//! Three caps are owned by the module/crate that defines their domain and are
//! cross-referenced (not duplicated) here, since a `const` cannot span a crate
//! boundary and moving a `pub(crate)` const would only churn its many import
//! sites for no behavioral gain:
//!
//! - **`MAX_PANES`** (32) - [`crate::layout::MAX_PANES`]. Live UI create cap
//!   (split / drop-to-split / IPC `surface.split` / `workspace.create`) ↔ read
//!   cap in [`paneflow_config::loader::validate_layout`] (US-011) and at session
//!   restore (US-009).
//! - **`MAX_WORKSPACES`** (20) - [`crate::workspace::MAX_WORKSPACES`]. Live
//!   `workspace.create` cap ↔ `restore_workspaces` cap (US-009).
//! - **`MAX_CONFIG_SIZE_BYTES`** (1 MiB) - `paneflow_config::loader`. Read cap
//!   on `paneflow.json`; the app's own config writer never approaches it.

/// Per-line read cap on untrusted agent-written session JSONL
/// (`claude_sessions`, `codex_sessions`). The agents are the (external) write
/// side; a line over this is treated as malformed and the file is skipped.
/// Deduplicated from two identical `const`s (US-013).
pub(crate) const MAX_LINE_BYTES: u64 = 64 * 1024;

/// Scrollback character cap applied by the WRITE side
/// (`TerminalState::extract_scrollback_from` → `cap_scrollback_at_char_boundary`,
/// US-001) when persisting a pane. Its READ-side counterpart is
/// [`MAX_SESSION_SIZE_BYTES`], which is sized from this × panes × workspaces.
pub(crate) const MAX_CHARS: usize = 400_000;

/// Cap on an OSC52 clipboard payload, applied on BOTH the Store (write) and
/// Load (read) paths in the terminal. Deduplicated from two identical `const`s
/// (US-013); keeping one source is what guarantees Store and Load stay
/// symmetric (EP-004 US-015).
pub(crate) const MAX_OSC52_BYTES: usize = 100 * 1024;

/// JSON-RPC framing ceiling on the local IPC socket: the server's per-line read
/// (`read_capped_line` in `ipc.rs`) and the MCP bridge client's reply read
/// (`paneflow_mcp::ipc_client::MAX_RESPONSE_LEN`, a cross-crate mirror of this
/// value) both bound a single request/reply to this many bytes.
pub(crate) const MAX_REQUEST_LEN: u64 = 256 * 1024;

/// Read cap on `session.json` (U-008/U-016). Sized from the write side:
/// [`MAX_CHARS`] per pane × `MAX_PANES` × `MAX_WORKSPACES` is a few hundred MB
/// worst-case, but a realistic maxed session is a few MB - 64 MiB sits far
/// above any legitimate session while bounding a multi-hundred-MB tampered file
/// before it is read whole into RAM. Mirrors `MAX_CONFIG_SIZE_BYTES` for
/// `paneflow.json`, which previously had no session-side counterpart.
pub(crate) const MAX_SESSION_SIZE_BYTES: u64 = 64 * 1024 * 1024;
