// US-016 wires this module into the Composer (`agents::composer`)
// and the ThreadView. While the wiring is in flight, the public API
// surface is referenced in fewer call-sites than it eventually will
// be -- silencing dead-code until the wiring lands keeps the build
// loop clean without scattering `#[allow]` on each item.
#![allow(dead_code)]

//! US-016 (prd-agents-view.md): live ACP session runtime that bridges
//! the `paneflow-acp` SDK (built on `tokio`) and Paneflow's GPUI main
//! thread (built on `smol`).
//!
//! Architecture in three layers:
//!
//! ```text
//!                 GPUI main thread                 |   background OS thread (tokio runtime)
//!   ThreadView / Composer entity                   |   ConnectionTo<Agent>
//!         |                                        |        |
//!         | send_prompt(text) / cancel() / ...     |        |
//!         v                                        |        |
//!   SessionRuntime  -- std::mpsc::Sender<Command> ----->    |  main_fn event loop:
//!                                                  |          - read Command
//!                                                  |          - send_request(Init/NewSession/Prompt)
//!                                                  |          - send_notification(Cancel)
//!                                                  |   .send_request                       send_notification
//!                                                  |        |                                  |
//!   ChannelSink ----<-- std::mpsc::Sender<Event> -------------- impl NotificationSink
//!   pump (cx.spawn loop)                           |
//!         |                                        |
//!         v
//!   ThreadView::push_streaming_chunk / etc.
//! ```
//!
//! The tokio runtime is single-threaded (one OS thread) per session so
//! the channel costs are negligible; spawning N runtimes for N
//! concurrent threads is acceptable because each agent process is
//! already a separate child.
//!
//! Shutdown semantics: dropping the [`SessionRuntime`] sends a
//! `Command::Shutdown`, drops the command sender (waking the runtime
//! if it was idle), and joins the OS thread inside `Drop`. The thread
//! returns when `main_fn` exits, which happens when the command loop
//! breaks. The agent child process is killed by the SDK when the
//! `ConnectionTo<Agent>` future completes.

use std::collections::HashMap;
use std::path::PathBuf;
// US-020 (cli-hardening-followup-2026-Q3): the runtime command
// channel is now `tokio::sync::mpsc::unbounded_channel`. The
// previous `std::sync::mpsc::Receiver` was wrapped in
// `Arc<Mutex<...>>` and pulled via `tokio::task::spawn_blocking`
// on every loop iteration -- one OS thread spawn + mutex lock +
// join per command. tokio's unbounded receiver is async-native
// (`.recv().await`) and the sender is interior-mutable
// (`&self`), matching the existing GPUI-thread `cmd_tx.send(...)`
// call sites unchanged.
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::mpsc::{
    UnboundedReceiver as CommandReceiver, UnboundedSender as CommandSender,
    unbounded_channel as command_channel,
};

// US-003 (cli-hardening-followup-2026-Q3): the runtime->GPUI event
// channel is now bounded at `RUNTIME_EVENT_CAPACITY`. The previous
// `futures::channel::mpsc::unbounded` could accumulate hundreds of
// `RuntimeEvent::Chunk(String)` entries on a tool-call burst (Claude
// Code doing 200 rapid edits) before the GPUI consumer drained it,
// transiently spiking 5-20 MB. Switched to `tokio::sync::mpsc` because
// its `Sender::try_send` takes `&self` (futures::mpsc::Sender needs
// `&mut self`), which lets `ChannelSink` (an ACP trait implementor
// whose methods take `&self`) call into it without an interior
// `Mutex`. Producer back-pressure is encoded in
// [`send_runtime_event`]: try_send, then log + drop on saturation --
// the consumer's existing coalescing at `composer.rs:1129` tolerates
// gaps in the Chunk stream.
use tokio::sync::mpsc::Receiver as UnboundedReceiver;
use tokio::sync::mpsc::Sender as UnboundedSender;

/// US-003 (cli-hardening-followup-2026-Q3): cap on the runtime->GPUI
/// event channel. 256 entries is enough headroom for the burstiest
/// observed agent traffic (Claude Code emitting ~150 ToolCall +
/// ToolCallUpdate frames during a multi-file refactor) without
/// allowing transient 5-20 MB allocations when the GPUI consumer is
/// behind on a large markdown render.
const RUNTIME_EVENT_CAPACITY: usize = 256;

/// US-003 (cli-hardening-followup-2026-Q3): single producer-side
/// entry point that all runtime event emissions go through.
///
/// Replaces the previous `event_tx.unbounded_send(event)` pattern.
/// On a successful `try_send`, the channel slot is consumed. On
/// `TrySendError::Full`, the event is dropped and logged at `warn!`
/// (or `debug!` for the high-frequency idempotent `Chunk`/`Thought`
/// variants -- the consumer's coalescing at `composer.rs:1129`
/// reconstructs the streaming buffer even with a few lost frames).
/// On `TrySendError::Closed`, the event is dropped silently because
/// the consumer side has already torn down -- mirrors the previous
/// `let _ = event_tx.unbounded_send(...)` behaviour on a dead
/// channel.
///
/// `tx` is `&Sender` (not `&mut`) because tokio's mpsc sender uses
/// interior mutability; this preserves the existing call-site shape
/// where `event_tx.clone()` is shared across multiple producer
/// holders (ChannelSink, BrokerCallback, run_blocking, Drop path).
fn send_runtime_event(tx: &UnboundedSender<RuntimeEvent>, event: RuntimeEvent) {
    use tokio::sync::mpsc::error::TrySendError;
    match tx.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(dropped)) => {
            // High-frequency idempotent variants get a quieter log
            // line so a slow-render burst does not flood
            // `paneflow-debug.log`.
            match &dropped {
                RuntimeEvent::Chunk(_) | RuntimeEvent::Thought(_) => log::debug!(
                    "runtime channel saturated; dropping idempotent event \
                     (consumer coalescing will rebuild)"
                ),
                other => log::warn!(
                    "runtime channel saturated; dropping non-coalescable event: {}",
                    runtime_event_variant_name(other)
                ),
            }
        }
        Err(TrySendError::Closed(_)) => {
            // Consumer torn down. Matches the pre-US-003 silent-drop
            // behaviour of `let _ = event_tx.unbounded_send(...)`.
        }
    }
}

/// US-003 (cli-hardening-followup-2026-Q3): drop-in compat shim that
/// lets every pre-existing `event_tx.unbounded_send(RuntimeEvent::X(...))`
/// call site keep its current shape after the channel switched from
/// `futures::channel::mpsc::unbounded` to `tokio::sync::mpsc::channel`.
/// The trait method delegates to [`send_runtime_event`], which
/// applies the saturation / closed-channel discipline. Returns
/// `Result<(), ()>` purely so call sites that wrap the result in
/// `let _ = ...` keep compiling without `let _ = ();` clippy churn.
trait UnboundedSendCompat {
    fn unbounded_send(&self, event: RuntimeEvent) -> Result<(), ()>;
}

impl UnboundedSendCompat for tokio::sync::mpsc::Sender<RuntimeEvent> {
    fn unbounded_send(&self, event: RuntimeEvent) -> Result<(), ()> {
        send_runtime_event(self, event);
        Ok(())
    }
}

/// Stable variant name for diagnostics; avoids needing `Debug` on
/// the full event payload (some variants embed large strings).
fn runtime_event_variant_name(ev: &RuntimeEvent) -> &'static str {
    match ev {
        RuntimeEvent::Chunk(_) => "Chunk",
        RuntimeEvent::Thought(_) => "Thought",
        RuntimeEvent::ToolCall(_) => "ToolCall",
        RuntimeEvent::ToolCallUpdate(_) => "ToolCallUpdate",
        RuntimeEvent::AvailableCommandsUpdate(_) => "AvailableCommandsUpdate",
        RuntimeEvent::UsageUpdate { .. } => "UsageUpdate",
        RuntimeEvent::SessionTitle(_) => "SessionTitle",
        RuntimeEvent::SessionReady { .. } => "SessionReady",
        RuntimeEvent::PermissionRequest { .. } => "PermissionRequest",
        RuntimeEvent::Fatal(_) => "Fatal",
        RuntimeEvent::TurnEnded(_) => "TurnEnded",
    }
}

use agent_client_protocol::schema::{
    AvailableCommand, ContentBlock, ContentChunk, ModelId,
    PermissionOptionKind as AcpPermissionOptionKind, RequestPermissionRequest, SessionId,
    SessionMode, SessionModeId, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCall as AcpToolCall, ToolCallContent as AcpToolCallContent,
    ToolCallStatus as AcpToolCallStatus, ToolCallUpdate as AcpToolCallUpdate,
    ToolKind as AcpToolKind,
};
use agent_client_protocol::{Agent, ConnectionTo};
use paneflow_acp::{
    AcpConnection, AgentConnection, BoxFuture, ClientConfig, NotificationSink, PermissionCallback,
    PermissionDecision, TerminalSpawner, always_deny, connect_with_handlers, spawn_acp_agent,
};

/// One event drained from the runtime's tokio side and delivered to
/// the GPUI side by [`SessionRuntime::poll`].
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// `session/new` succeeded -- ready for prompts. Carries the
    /// model+mode info derived from the response so the composer can
    /// hydrate its picker pills (AC #5/#6).
    SessionReady {
        modes: Vec<SessionMode>,
        current_mode_id: Option<SessionModeId>,
        models: Vec<ModelChoice>,
        current_model_id: Option<String>,
    },
    /// An `AgentMessageChunk` text fragment. The composer pushes
    /// this verbatim into the streaming pipeline (US-015).
    Chunk(String),
    /// An `AgentThoughtChunk` text fragment -- reasoning tokens. The
    /// UI currently dimmed-renders these as a separate stream; for
    /// US-016 we collapse them into the regular chunk channel and
    /// the renderer prefixes them. Future stories (tool calls)
    /// route them differently.
    Thought(String),
    /// The turn finished. Carries the stop reason so the composer
    /// can morph Stop back to Send (AC #3 / #8).
    TurnEnded(StopReasonKind),
    /// US-017: the agent invoked a new tool. Carries the initial
    /// snapshot (id, title, kind, status, raw input/output, content).
    ToolCall(ToolCallSnapshot),
    /// US-017: an existing tool call changed state (status, title,
    /// content, output, ...). Only the fields the agent reported are
    /// `Some`; the renderer applies the patch in-place.
    ToolCallUpdate(ToolCallUpdate),
    /// US-018: the agent requested user approval for a tool call.
    /// The composer surfaces this by flipping the matching
    /// ToolCallView into `WaitingForConfirmation` and rendering the
    /// Allow / Deny buttons. The user click resolves the request
    /// via [`SessionRuntime::resolve_permission`].
    PermissionRequest {
        tool_call_id: String,
        options: Vec<PermissionOptionInfo>,
    },
    /// US-112: the agent published (or updated) the list of slash
    /// commands it can execute via the ACP `available_commands_update`
    /// notification. The composer merges these with the built-in
    /// `/clear` + `/export` commands when the user opens the `/`
    /// picker. An empty vec is a valid value (Codex today reports
    /// nothing) and resets the cache.
    AvailableCommandsUpdate(Vec<AvailableCommand>),
    /// US-120 / US-121: ACP `session/usage_update` notification
    /// (unstable feature `unstable_session_usage`). Carries the
    /// cumulative `used` token count + the context-window `size`. The
    /// composer renders the count as a `Nk tokens` badge in the footer
    /// (US-120) and the activity bar gates the throughput suffix on
    /// `used >= 1000` (US-121). Today's shipping ACP wrappers (Claude
    /// Code @0.16, Codex @0.14) do not emit this notification -- the
    /// badge stays absent until a wrapper opts in, which is the AC #4
    /// "no count available -> badge absent" contract.
    UsageUpdate { used: u64, size: u64 },
    /// ACP `SessionInfoUpdate` — the agent (Claude Code via `/resume`
    /// summaries, Codex when it produces a session label, ...)
    /// suggested a human-readable title for the conversation. The
    /// composer routes this up to the ThreadView which emits
    /// `TitleSuggested` so the parent app can rename the sidebar row
    /// + persist via `ThreadStore::set_summary`.
    SessionTitle(String),
    /// Runtime-level failure (process died, JSON-RPC error, etc.).
    /// The composer surfaces this as an inline error row and
    /// disables Send until the user reopens the thread.
    Fatal(String),
}

/// UI-side mirror of `agent_client_protocol::schema::PermissionOption`.
/// `kind` is mirrored as a plain enum so the Composer doesn't take a
/// direct dep on the schema crate at the renderer layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOptionInfo {
    pub id: String,
    pub name: String,
    pub kind: PermissionOptionKindKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOptionKindKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Other,
}

impl From<AcpPermissionOptionKind> for PermissionOptionKindKind {
    fn from(k: AcpPermissionOptionKind) -> Self {
        match k {
            AcpPermissionOptionKind::AllowOnce => Self::AllowOnce,
            AcpPermissionOptionKind::AllowAlways => Self::AllowAlways,
            AcpPermissionOptionKind::RejectOnce => Self::RejectOnce,
            AcpPermissionOptionKind::RejectAlways => Self::RejectAlways,
            _ => Self::Other,
        }
    }
}

/// UI-side mirror of `agent_client_protocol::schema::ToolCall`.
/// Decoupling from the wire type lets the ThreadView store these
/// in its `items` list without pulling the schema crate into
/// downstream renderers, and gives us a stable surface even if the
/// ACP enum gains variants under `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallSnapshot {
    pub id: String,
    pub title: String,
    pub kind: ToolKindKind,
    pub status: ToolCallStatusKind,
    /// Pretty-printed JSON of `raw_input`, ready for inline display
    /// in the tool card's expanded view (AC #4 -- "input args rendered
    /// as YAML or JSON, syntax-highlighted"). `None` when the agent
    /// did not include raw_input.
    pub raw_input_pretty: Option<String>,
    /// Pretty-printed JSON of `raw_output`. Same display rules as
    /// `raw_input_pretty`.
    pub raw_output_pretty: Option<String>,
    /// Joined textual rendering of the `content` array. Diff and
    /// terminal variants are summarised as `[diff]` / `[terminal]`
    /// for the MVP; US-018 wires the proper diff renderer.
    pub content_text: String,
    /// Structured diffs emitted by the agent via the
    /// `ToolCallContent::Diff` variant. The standalone EditToolBlock
    /// renderer reads these to paint Claude-Code-style +/- gutters
    /// outside the reasoning card. Empty vec when the tool call has
    /// not surfaced any diffs (typical for Read / Search / Bash).
    pub diffs: Vec<DiffSnapshot>,
    /// UI-side expand/collapse flag (AC #3). The ACP wire shape
    /// does not carry this -- it is a local view-state bit.
    pub expanded: bool,
    /// US-018: the permission options offered by the agent. Empty
    /// vec when the tool call is not in `WaitingForConfirmation`
    /// (default). Populated by
    /// [`crate::agents::thread_view::ThreadView::set_pending_permission`]
    /// when the runtime forwards a `RequestPermissionRequest`.
    pub permission_options: Vec<PermissionOptionInfo>,
    /// US-124: transient view-state flag toggled by the user clicking
    /// the "Allow Always" button while a permission row is open. When
    /// `true`, the row expands to reveal the inline pattern picker
    /// (this-thread / everywhere / parsed terminal pattern proposals).
    /// Not persisted -- defaults to `false` on every snapshot rebuild.
    pub permission_picker_open: bool,
    /// File targets the agent declared for this tool call -- mirrors
    /// Zed's `acp_thread::ToolCall.locations` (`crates/acp_thread/src/
    /// acp_thread.rs:259`). Used by the inline label renderer to
    /// append a path chip (e.g. `Read file \`Cargo.toml\``) when the
    /// agent's `title` is generic ("Read File") and would otherwise
    /// leave the row uninformative. Pure metadata; no view state.
    pub locations: Vec<ToolCallLocation>,
}

/// File reference attached to a tool call (mirror of
/// `agent_client_protocol::schema::ToolCallLocation`). The `line`
/// field is an optional 0-based start line; when present, the inline
/// label can render `(line N+1)` next to the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallLocation {
    pub path: std::path::PathBuf,
    pub line: Option<u32>,
}

/// UI-side mirror of `agent_client_protocol::schema::Diff`. Kept as
/// a plain-data struct so downstream renderers (EditToolBlock) don't
/// take a direct dep on the ACP schema crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSnapshot {
    /// File path being modified.
    pub path: std::path::PathBuf,
    /// Original content (`None` for newly created files).
    pub old_text: Option<String>,
    /// New content after modification.
    pub new_text: String,
    /// US-001 (cli-hardening-followup): set by
    /// `ThreadView::purge_reviewed_tool_ui_state` after a Keep/Reject
    /// review to record the original diff-line count before
    /// `old_text` is freed. `None` = diff body is intact (live or
    /// freshly loaded from disk). `Some(n)` = `old_text` has been
    /// cleared post-review; the renderer shows a placeholder citing
    /// `n` lines instead of computing a `[]` vs `new_text` diff.
    /// Runtime-only; never persisted (review state on disk is the
    /// `reviewed` flag on `PersistedToolCall`).
    pub cleared_diff_lines: Option<u32>,
}

/// Patch describing the changes from one `ToolCallUpdate` wire
/// message. Only the `Some` fields should overwrite the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCallUpdate {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKindKind>,
    pub status: Option<ToolCallStatusKind>,
    pub raw_input_pretty: Option<String>,
    pub raw_output_pretty: Option<String>,
    /// `None` means "no change", `Some(text)` means "replace the
    /// content text wholesale" (matches the ACP semantics where
    /// `content` is replaced, not extended).
    pub content_text: Option<String>,
    /// Same wholesale-replace semantics as `content_text`: `None`
    /// keeps the existing diffs, `Some(vec)` replaces them.
    pub diffs: Option<Vec<DiffSnapshot>>,
    /// `None` = keep existing locations; `Some(vec)` = wholesale
    /// replace. Mirrors ACP `ToolCallUpdateFields.locations` semantics.
    pub locations: Option<Vec<ToolCallLocation>>,
}

/// UI-side mirror of `agent_client_protocol::schema::ToolKind`. We
/// keep the variant order stable so adding a kind upstream surfaces
/// as `Other` rather than breaking pattern matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKindKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

impl From<AcpToolKind> for ToolKindKind {
    fn from(k: AcpToolKind) -> Self {
        match k {
            AcpToolKind::Read => Self::Read,
            AcpToolKind::Edit => Self::Edit,
            AcpToolKind::Delete => Self::Delete,
            AcpToolKind::Move => Self::Move,
            AcpToolKind::Search => Self::Search,
            AcpToolKind::Execute => Self::Execute,
            AcpToolKind::Think => Self::Think,
            AcpToolKind::Fetch => Self::Fetch,
            AcpToolKind::SwitchMode => Self::SwitchMode,
            AcpToolKind::Other => Self::Other,
            unknown => {
                // US-014 AC #3: a future ACP schema variant (the enum
                // carries `#[non_exhaustive]`) reached us before the
                // match was extended. Fall back to Other (renders as
                // ToolHammer, matching Zed's catch-all) and log once so
                // schema additions surface during dogfooding without
                // spamming the log on every tool call.
                static LOG_ONCE: std::sync::Once = std::sync::Once::new();
                LOG_ONCE.call_once(|| {
                    log::info!(
                        "agents::runtime: unrecognised ACP tool kind {:?}; rendering as Other (ToolHammer)",
                        unknown
                    );
                });
                Self::Other
            }
        }
    }
}

impl ToolKindKind {
    /// AC #5: edit tools use the boxed card layout; AC #6: read-only
    /// tools use the inline disclosure. Anything that modifies the
    /// filesystem, terminal, or session counts as "edit-like" for
    /// this decision.
    pub fn is_edit_like(self) -> bool {
        matches!(
            self,
            Self::Edit | Self::Delete | Self::Move | Self::Execute | Self::SwitchMode
        )
    }
}

impl ToolCallSnapshot {
    /// True when the tool call should render as a standalone block
    /// outside the reasoning card (Claude-Code-style diff block).
    /// False when it should fold into the reasoning card alongside
    /// thinking chunks (Read / Search / Bash / Fetch / ...).
    ///
    /// Decision matrix:
    /// - `Edit | Delete | Move` -> always standalone
    /// - Any kind WITH `diffs.is_empty() == false` -> standalone
    ///   (the diff is the user-facing artefact and deserves the
    ///   full-width treatment)
    /// - `WaitingForConfirmation | Failed` -> standalone (user must
    ///   see the action that needs attention)
    /// - everything else -> in the reasoning card
    pub fn is_standalone(&self) -> bool {
        if matches!(
            self.kind,
            ToolKindKind::Edit | ToolKindKind::Delete | ToolKindKind::Move
        ) {
            return true;
        }
        if !self.diffs.is_empty() {
            return true;
        }
        matches!(
            self.status,
            ToolCallStatusKind::WaitingForConfirmation | ToolCallStatusKind::Failed
        )
    }

    /// Convert to the plain-data persisted form for the SQLite blob.
    /// Kind and status are encoded as stable lowercase strings so the
    /// on-disk shape is decoupled from the in-memory enum variants.
    pub fn to_persisted(&self) -> paneflow_threads::PersistedToolCall {
        paneflow_threads::PersistedToolCall {
            id: self.id.clone(),
            title: self.title.clone(),
            tool_kind: self.kind.persist_str().to_string(),
            status: self.status.persist_str().to_string(),
            raw_input_pretty: self.raw_input_pretty.clone(),
            raw_output_pretty: self.raw_output_pretty.clone(),
            content_text: self.content_text.clone(),
            diffs: self
                .diffs
                .iter()
                .map(|d| paneflow_threads::PersistedDiff {
                    path: d.path.clone(),
                    old_text: d.old_text.clone(),
                    new_text: d.new_text.clone(),
                })
                .collect(),
            // Review state lives on the ThreadView, not on the
            // snapshot. The caller (`collect_persisted_items`)
            // overrides this from its `reviewed_edits` HashSet before
            // writing to disk.
            reviewed: false,
        }
    }

    /// Build an in-memory snapshot from its persisted form. Status is
    /// normalised: `Pending` / `InProgress` become `Failed` because a
    /// reloaded tool call cannot have an active runtime backing it
    /// (mirrors Zed's `replay_tool_call` semantics — `agent/src/
    /// thread.rs:1224-1232`).
    pub fn from_persisted(p: paneflow_threads::PersistedToolCall) -> Self {
        let raw_status = ToolCallStatusKind::from_persist_str(&p.status);
        let status = match raw_status {
            ToolCallStatusKind::Pending | ToolCallStatusKind::InProgress => {
                ToolCallStatusKind::Failed
            }
            other => other,
        };
        Self {
            id: p.id,
            title: p.title,
            kind: ToolKindKind::from_persist_str(&p.tool_kind),
            status,
            raw_input_pretty: p.raw_input_pretty,
            raw_output_pretty: p.raw_output_pretty,
            content_text: p.content_text,
            diffs: p
                .diffs
                .into_iter()
                .map(|d| DiffSnapshot {
                    path: d.path,
                    old_text: d.old_text,
                    new_text: d.new_text,
                    cleared_diff_lines: None,
                })
                .collect(),
            expanded: false,
            permission_options: Vec::new(),
            permission_picker_open: false,
            // Persisted shape predates the locations field. Reloaded
            // tool calls render without the path chip; this is fine
            // because their titles were already enriched (or not)
            // before persistence by `enrich_title_with_location` and
            // the persisted title is what we trust on reload.
            locations: Vec::new(),
        }
    }
}

impl ToolKindKind {
    /// Stable lowercase discriminator used in the persisted blob. The
    /// inverse is [`Self::from_persist_str`].
    pub fn persist_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Search => "search",
            Self::Execute => "execute",
            Self::Think => "think",
            Self::Fetch => "fetch",
            Self::SwitchMode => "switch_mode",
            Self::Other => "other",
        }
    }

    pub fn from_persist_str(s: &str) -> Self {
        match s {
            "read" => Self::Read,
            "edit" => Self::Edit,
            "delete" => Self::Delete,
            "move" => Self::Move,
            "search" => Self::Search,
            "execute" => Self::Execute,
            "think" => Self::Think,
            "fetch" => Self::Fetch,
            "switch_mode" => Self::SwitchMode,
            _ => Self::Other,
        }
    }
}

impl ToolCallStatusKind {
    pub fn persist_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::WaitingForConfirmation => "waiting_for_confirmation",
            Self::Rejected => "rejected",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    pub fn from_persist_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "waiting_for_confirmation" => Self::WaitingForConfirmation,
            "rejected" => Self::Rejected,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => Self::Pending,
        }
    }
}

/// UI-side mirror of `agent_client_protocol::schema::ToolCallStatus`,
/// PLUS the three UI-only states named in PRD AC #2:
/// `WaitingForConfirmation`, `Rejected`, `Canceled`. The wire
/// protocol does not carry these directly:
/// - `WaitingForConfirmation` is set by US-018 when a
///   `RequestPermissionRequest` is pending for this tool call.
/// - `Rejected` is set by US-018 when the user clicks Deny.
/// - `Canceled` is set when `session/cancel` aborts the running
///   turn before this tool call completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatusKind {
    Pending,
    WaitingForConfirmation,
    Rejected,
    InProgress,
    Completed,
    Failed,
    Canceled,
}

impl From<AcpToolCallStatus> for ToolCallStatusKind {
    fn from(s: AcpToolCallStatus) -> Self {
        match s {
            AcpToolCallStatus::Pending => Self::Pending,
            AcpToolCallStatus::InProgress => Self::InProgress,
            AcpToolCallStatus::Completed => Self::Completed,
            AcpToolCallStatus::Failed => Self::Failed,
            _ => Self::Pending,
        }
    }
}

impl ToolCallStatusKind {
    /// Returns the asset path for the status icon. `None` when the
    /// state should render a spinner instead (so the caller can swap
    /// in `loader-circle.svg` with its own rotation animation).
    pub fn icon_path(self) -> Option<&'static str> {
        match self {
            Self::Pending | Self::InProgress => None, // -> spinner
            Self::Completed => Some("icons/checks.svg"),
            Self::Failed | Self::Rejected => Some("icons/generic_close.svg"),
            Self::WaitingForConfirmation => Some("icons/loader-circle.svg"),
            Self::Canceled => Some("icons/generic_close.svg"),
        }
    }
}

fn pretty_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Convert ACP `ToolCallContent` items into a single text body PLUS
/// a separate vec of structured diffs. The text body gets bracketed
/// placeholders for diffs / terminals so the in-card compact row
/// still has something readable to fall back on; the diffs vec feeds
/// the standalone EditToolBlock renderer.
fn flatten_content(content: &[AcpToolCallContent]) -> (String, Vec<DiffSnapshot>) {
    let mut out = String::new();
    let mut diffs = Vec::new();
    for block in content {
        match block {
            AcpToolCallContent::Content(c) => match &c.content {
                ContentBlock::Text(t) => {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&t.text);
                }
                _ => out.push_str("\n[non-text content]"),
            },
            AcpToolCallContent::Diff(d) => {
                diffs.push(DiffSnapshot {
                    path: d.path.clone(),
                    old_text: d.old_text.clone(),
                    new_text: d.new_text.clone(),
                    cleared_diff_lines: None,
                });
            }
            AcpToolCallContent::Terminal(_) => out.push_str("\n[terminal]"),
            _ => out.push_str("\n[unknown tool content]"),
        }
    }
    (out, diffs)
}

fn snapshot_from_tool_call(tc: AcpToolCall) -> ToolCallSnapshot {
    let (content_text, diffs) = flatten_content(&tc.content);
    let locations: Vec<ToolCallLocation> = tc
        .locations
        .into_iter()
        .map(|l| ToolCallLocation {
            path: l.path,
            line: l.line,
        })
        .collect();
    let kind: ToolKindKind = tc.kind.into();
    // Zed-parity: edit/diff cards open expanded by default so the user
    // sees the file change inline without an extra click. Mirrors Zed's
    // `render_diff_editor` which renders the diff body unconditionally
    // once the tool call is `Completed`. Read / Search / Execute cards
    // stay collapsed -- their output is opt-in chrome.
    let starts_expanded = matches!(kind, ToolKindKind::Edit) || !diffs.is_empty();
    ToolCallSnapshot {
        id: tc.tool_call_id.0.to_string(),
        title: enrich_title_with_location(tc.title, kind, &locations),
        kind,
        status: tc.status.into(),
        raw_input_pretty: tc.raw_input.as_ref().map(pretty_json),
        raw_output_pretty: tc.raw_output.as_ref().map(pretty_json),
        content_text,
        diffs,
        expanded: starts_expanded,
        permission_options: Vec::new(),
        permission_picker_open: false,
        locations,
    }
}

/// If the agent's `title` looks generic (e.g. "Read File", "Edit",
/// "Search") and we have at least one location, append the file
/// basename as a markdown inline-code chip so the row reads "Read
/// file `Cargo.toml`" instead of just "Read File". Mirrors what Zed
/// gets from Claude Code when the agent sends a pre-enriched title --
/// some Claude versions/configurations only send the kind name, in
/// which case Paneflow has to recover the file info from `locations`
/// to be useful at all (otherwise every Read row reads identically).
///
/// Heuristic: when the title already contains the basename (case
/// sensitive) of the first location, leave it alone. Otherwise:
/// - Read   -> "Read `name`"
/// - Edit   -> "Edit `name`"
/// - Delete -> "Delete `name`"
/// - Move   -> "Move `name`"
/// - Search -> "Search `name`"
/// - Other / Execute / Think / Fetch / SwitchMode: leave the title
///   untouched -- those rarely correspond to a single file target.
pub(crate) fn enrich_title_with_location(
    title: String,
    kind: ToolKindKind,
    locations: &[ToolCallLocation],
) -> String {
    let Some(loc) = locations.first() else {
        return title;
    };
    let Some(basename) = loc.path.file_name().and_then(|s| s.to_str()) else {
        return title;
    };
    if title.contains(basename) {
        return title;
    }
    let verb = match kind {
        ToolKindKind::Read => "Read",
        ToolKindKind::Edit => "Edit",
        ToolKindKind::Delete => "Delete",
        ToolKindKind::Move => "Move",
        ToolKindKind::Search => "Search",
        _ => return title,
    };
    format!("{verb} `{basename}`")
}

fn update_from_tool_call_update(u: AcpToolCallUpdate) -> ToolCallUpdate {
    let (content_text, diffs) = match u.fields.content.as_ref() {
        Some(c) => {
            let (text, diffs) = flatten_content(c);
            (Some(text), Some(diffs))
        }
        None => (None, None),
    };
    let locations = u.fields.locations.map(|locs| {
        locs.into_iter()
            .map(|l| ToolCallLocation {
                path: l.path,
                line: l.line,
            })
            .collect()
    });
    ToolCallUpdate {
        id: u.tool_call_id.0.to_string(),
        title: u.fields.title,
        kind: u.fields.kind.map(Into::into),
        status: u.fields.status.map(Into::into),
        raw_input_pretty: u.fields.raw_input.as_ref().map(pretty_json),
        raw_output_pretty: u.fields.raw_output.as_ref().map(pretty_json),
        content_text,
        diffs,
        locations,
    }
}

/// Mirrors `paneflow_acp` re-export of `agent_client_protocol`'s
/// `ModelInfo` but as a plain-data struct so the UI never depends on
/// the `unstable_session_model` feature transitively. (The composer
/// only reads `id` + `name`.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub name: String,
}

/// `StopReason` is `#[non_exhaustive]` on the wire side. We expose
/// every meaningful UI branch as a distinct variant so the composer
/// can show the user *why* the assistant stopped (especially when
/// the text was cut off mid-word):
/// - [`Self::EndTurn`] -- the agent finished cleanly. Notification
///   reads "Finished running tools" or "New message" (US-116 AC #3).
/// - [`Self::MaxTokens`] -- the agent hit the output-token budget.
///   The visible text is truncated by the model, NOT by Paneflow.
///   The composer surfaces "Response truncated: max tokens reached"
///   so the user can choose to re-prompt.
/// - [`Self::MaxTurnRequests`] -- the agent hit the per-turn request
///   cap. Surfaced as "Stopped: too many tool calls this turn".
/// - [`Self::Cancelled`] -- the user (or Paneflow on shutdown) sent
///   `session/cancel`. Surfaced as "Stopped: cancelled" so the user
///   knows the partial state is intentional.
/// - [`Self::Refusal`] -- the agent refused to respond. Composer
///   suppresses success notification; renderer warns.
/// - [`Self::Other`] -- future wire variants we haven't classified
///   yet. Treated like a generic stop with no specific copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReasonKind {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Cancelled,
    Refusal,
    Other,
}

impl StopReasonKind {
    /// Human-readable status line for the composer's stop-status row.
    /// `None` for `EndTurn` (clean exit, nothing to show) and `Refusal`
    /// (the refusal already has its own visible treatment via the
    /// composer's `fatal_error` / notification path).
    pub fn status_message(self) -> Option<&'static str> {
        match self {
            // Cancelled is user-initiated (Stop button, or composer
            // send_prompt_immediate auto-cancel). The user already
            // knows they stopped it -- a banner would be noise.
            // Refusal has its own treatment via the assistant body;
            // EndTurn is the success path.
            Self::EndTurn | Self::Refusal | Self::Cancelled => None,
            Self::MaxTokens => Some(
                "Response truncated: max output tokens reached. Send a new prompt to continue.",
            ),
            Self::MaxTurnRequests => Some("Stopped: agent reached the per-turn request limit."),
            Self::Other => Some("Stopped before the agent finished."),
        }
    }
}

impl From<StopReason> for StopReasonKind {
    fn from(reason: StopReason) -> Self {
        match reason {
            StopReason::EndTurn => Self::EndTurn,
            StopReason::MaxTokens => Self::MaxTokens,
            StopReason::MaxTurnRequests => Self::MaxTurnRequests,
            StopReason::Cancelled => Self::Cancelled,
            StopReason::Refusal => Self::Refusal,
            _ => Self::Other,
        }
    }
}

/// One command from the GPUI side to the runtime's event loop.
#[derive(Debug)]
enum Command {
    /// Send a `session/prompt` with a fully-built block list. US-019:
    /// the Composer now mixes text with `ContentBlock::Image` /
    /// `ContentBlock::ResourceLink` attachments, so the runtime
    /// accepts the wire shape directly rather than wrapping a string.
    SendPrompt(Vec<ContentBlock>),
    /// Send a `session/cancel` notification. Fire-and-forget; the
    /// agent will emit a `Cancelled` `StopReason` shortly after.
    Cancel,
    /// Send `session/set_mode` for the given mode id.
    SetMode(SessionModeId),
    /// Send `session/set_model` for the given model id. Mid-session
    /// model switch — the agent applies it to subsequent prompts.
    SetModel(ModelId),
    /// US-018: resolve a pending permission request. The
    /// PermissionBroker looks up the oneshot for this tool_call_id
    /// and completes it with the user's decision.
    ResolvePermission(String, PermissionDecision),
    /// Tell the loop to break -- triggered by `Drop`.
    Shutdown,
}

/// Live ACP session bound to one ThreadView. Created on first
/// prompt, dropped when the ThreadView is dropped (US-016 AC #7 --
/// switching agent mid-thread creates a NEW session by dropping the
/// old runtime and constructing a new one).
pub struct SessionRuntime {
    cmd_tx: CommandSender<Command>,
    /// Push-based event stream from the tokio side. `take_event_receiver`
    /// hands it to the Composer's `cx.spawn` task exactly once; from
    /// then on the Composer awaits chunks directly without polling.
    /// Replaces the previous `Arc<Mutex<std::sync::mpsc::Receiver>>` so
    /// the GPUI side never holds a Mutex on the hot streaming path.
    event_rx: Option<UnboundedReceiver<RuntimeEvent>>,
    /// US-018: shared with the tokio side's [`BrokerCallback`].
    /// `Some` whenever the runtime is using the interactive
    /// permission flow; `None` when the caller supplied their own
    /// [`PermissionCallback`] (tests).
    broker: Option<Arc<PermissionBroker>>,
    join: Option<JoinHandle<()>>,
}

/// Spawn options used by [`SessionRuntime::spawn`].
pub struct SpawnOptions {
    /// Full command-line passed to [`paneflow_acp::spawn_acp_agent`]
    /// (e.g. `"env -u CLAUDECODE bunx -y @zed-industries/claude-code-acp@latest"`).
    pub spawn_command: String,
    /// Working directory for the new session. Echoed back into
    /// `NewSessionRequest::cwd` and used for filesystem sandboxing
    /// by the file-ops handler in `paneflow-acp`.
    pub cwd: PathBuf,
    /// Permission policy override. `None` (production UI) installs
    /// the [`PermissionBroker`] callback so each request lands on
    /// the Composer's Allow / Deny buttons. `Some(cb)` (tests) wires
    /// a deterministic policy.
    pub permission_callback: Option<Arc<dyn PermissionCallback>>,
    /// Terminal spawner override. `None` (production UI) installs
    /// [`crate::agents::agent_terminal::AgentTerminalSpawner`] so
    /// `terminal/create` runs real PTYs through portable-pty.
    /// `Some(spawner)` (tests) wires a deterministic stub.
    pub terminal_spawner: Option<Arc<dyn TerminalSpawner>>,
    /// US-118: when `Some(id)` AND the spawned backend reports
    /// [`AgentConnection::supports_load_session`] as `true`, the
    /// runtime calls [`AgentConnection::load_session`] instead of
    /// [`AgentConnection::new_session`] so the agent re-binds to the
    /// existing thread state instead of starting a fresh history.
    /// `None` (today's default) -- and any `Some(id)` against a
    /// backend that does NOT support load_session -- both fall back
    /// to the new-session path so the ThreadView's local replay of
    /// the persisted blob remains the authoritative source.
    pub resume_session_id: Option<SessionId>,
}

/// US-018: bridge between paneflow-acp's permission callback (which
/// runs on the tokio thread) and the Composer's Allow / Deny
/// buttons (which fire on the GPUI thread). The broker stores an
/// in-flight `oneshot::Sender` per tool-call id; the GPUI side
/// resolves it via [`SessionRuntime::resolve_permission`].
pub struct PermissionBroker {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<PermissionDecision>>>,
    event_tx: UnboundedSender<RuntimeEvent>,
}

impl PermissionBroker {
    fn new(event_tx: UnboundedSender<RuntimeEvent>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            event_tx,
        })
    }

    /// Resolve the pending request for `tool_call_id` with `decision`.
    /// Called from the runtime's main_fn loop in response to a
    /// `Command::ResolvePermission`. Unknown ids are silently
    /// ignored (idempotent on double-click).
    fn resolve(&self, tool_call_id: &str, decision: PermissionDecision) {
        let mut map =
            super::agent_terminal::lock_with_poison_log(&self.pending, "permission_broker_pending");
        if let Some(tx) = map.remove(tool_call_id) {
            let _ = tx.send(decision);
        }
    }

    /// Drop all pending requests, resolving each as `Deny`. Called
    /// from the runtime's shutdown path so the agent does not hang
    /// waiting for a response when the user closes the thread.
    fn drain_as_deny(&self) {
        let mut map =
            super::agent_terminal::lock_with_poison_log(&self.pending, "permission_broker_pending");
        for (_, tx) in map.drain() {
            let _ = tx.send(PermissionDecision::Reject);
        }
    }
}

/// [`PermissionCallback`] implementation that proxies to a
/// [`PermissionBroker`]. Constructed by `SessionRuntime` when no
/// explicit callback is supplied in [`SpawnOptions`].
struct BrokerCallback {
    broker: Arc<PermissionBroker>,
}

impl PermissionCallback for BrokerCallback {
    fn decide(&self, request: &RequestPermissionRequest) -> BoxFuture<'_, PermissionDecision> {
        let tool_call_id = request.tool_call.tool_call_id.0.to_string();
        let options: Vec<PermissionOptionInfo> = request
            .options
            .iter()
            .map(|o| PermissionOptionInfo {
                id: o.option_id.0.to_string(),
                name: o.name.clone(),
                kind: o.kind.into(),
            })
            .collect();
        // Global bypass: when the user has flipped "Bypass permissions"
        // in Settings -> AI Agent, every ACP `RequestPermission`
        // resolves to AllowOnce regardless of tool kind, agent backend,
        // or always-allow / always-deny rules. Mirrors the same intent
        // that's baked into the terminal Claude Code launcher
        // (`pane.rs:685` adds `--permission-mode bypassPermissions`).
        // Config is reloaded on every decide so flipping the toggle
        // takes effect on the very next request without restart.
        if paneflow_config::loader::load_config()
            .claude_code_bypass_permissions
            .unwrap_or(false)
        {
            return Box::pin(async { PermissionDecision::AllowOnce });
        }
        // US-111 AC #5 + US-124: an `always_allow` rule that the user
        // already persisted skips the prompt. The tool kind comes off
        // the `ToolCallUpdate.fields.kind` when the agent included it;
        // otherwise the call falls through to the UI. US-124 refines
        // matching: when the rule carries a substring pattern (rather
        // than the bare any-input shape), the broker hashes the
        // pretty-printed raw_input and checks each pattern as a
        // substring -- so `"npm install"` matches `npm install react`
        // but not `npm test`.
        let kind_hint: Option<ToolKindKind> = request.tool_call.fields.kind.map(ToolKindKind::from);
        let raw_input_pretty = request.tool_call.fields.raw_input.as_ref().map(pretty_json);
        if let Some(kind) = kind_hint {
            let raw = raw_input_pretty.as_deref();
            if crate::agents::panel_config::tool_kind_is_always_allowed_for(kind, raw) {
                return Box::pin(async { PermissionDecision::AllowOnce });
            }
            if crate::agents::panel_config::tool_kind_is_always_denied_for(kind, raw) {
                return Box::pin(async { PermissionDecision::Reject });
            }
            // Zed-parity (2026-05-25): file edits bypass the permission
            // gate. Zed exposes a separate `WriteTextFile` RPC for this
            // (acp.rs:3972) which has no permission check at all -- the
            // write lands and the user reviews afterwards via Keep All /
            // Reject All. Paneflow's ACP agents still call the unified
            // `RequestPermission` flow, so we auto-approve `Edit` kind
            // here to land at the same user-visible behavior. Delete /
            // Move stay user-confirmed -- they are destructive in a way
            // Edit is not (Edit is reversible from the captured diff;
            // Delete is not unless the agent emits the prior content).
            if kind == ToolKindKind::Edit {
                return Box::pin(async { PermissionDecision::AllowOnce });
            }
        }
        let broker = Arc::clone(&self.broker);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = super::agent_terminal::lock_with_poison_log(
                    &broker.pending,
                    "permission_broker_pending",
                );
                map.insert(tool_call_id.clone(), tx);
            }
            let _ = broker
                .event_tx
                .unbounded_send(RuntimeEvent::PermissionRequest {
                    tool_call_id,
                    options,
                });
            // If the receiver is dropped (composer + runtime tore
            // down) we default to Reject so the agent does not stall.
            rx.await.unwrap_or(PermissionDecision::Reject)
        })
    }
}

impl SessionRuntime {
    /// Spawn the agent on a background tokio thread and return a
    /// runtime handle. Returns immediately -- the caller polls
    /// [`SessionRuntime::poll`] for [`RuntimeEvent::SessionReady`]
    /// to know when `session/new` has resolved and the model/mode
    /// pickers can hydrate.
    pub fn spawn(opts: SpawnOptions) -> Self {
        let (cmd_tx, cmd_rx) = command_channel::<Command>();
        let (event_tx, event_rx) =
            tokio::sync::mpsc::channel::<RuntimeEvent>(RUNTIME_EVENT_CAPACITY);

        let SpawnOptions {
            spawn_command,
            cwd,
            permission_callback,
            terminal_spawner,
            resume_session_id,
        } = opts;

        // US-018: build the broker first, then choose the callback.
        // The broker is always allocated (cheap) so unit tests can
        // pass `None` for the callback and still get the
        // interactive Allow/Deny event flow; but tests that wire
        // `always_allow` / `always_deny` skip the broker by passing
        // the callback explicitly.
        let broker_for_runtime = PermissionBroker::new(event_tx.clone());
        let (callback, broker_for_handle): (
            Arc<dyn PermissionCallback>,
            Option<Arc<PermissionBroker>>,
        ) = match permission_callback {
            Some(cb) => (cb, None),
            None => (
                Arc::new(BrokerCallback {
                    broker: Arc::clone(&broker_for_runtime),
                }),
                Some(Arc::clone(&broker_for_runtime)),
            ),
        };

        // The tokio runtime lives on its own OS thread so the
        // `paneflow-acp` reactor never blocks GPUI's smol-driven
        // event loop.
        let broker_for_loop = Arc::clone(&broker_for_runtime);
        let join = match std::thread::Builder::new()
            .name("paneflow-acp-runtime".to_string())
            .spawn({
                let event_tx_clone = event_tx.clone();
                move || {
                    run_blocking(
                        spawn_command,
                        cwd,
                        callback,
                        terminal_spawner,
                        broker_for_loop,
                        cmd_rx,
                        event_tx_clone,
                        resume_session_id,
                    );
                }
            }) {
            Ok(j) => Some(j),
            Err(e) => {
                // Thread spawn can fail on EAGAIN: containers with low
                // `ulimit -u`, fork-bombed dev hosts, or systems where
                // RLIMIT_NPROC is exhausted. Surface as a Fatal event
                // instead of panicking the GPUI main thread -- the
                // Composer's RuntimeEvent::Fatal handler shows a clean
                // error in the thread view and tears down the runtime.
                let _ = event_tx.unbounded_send(RuntimeEvent::Fatal(format!(
                    "Could not spawn ACP runtime thread: {e}. \
                     Check `ulimit -u` / container thread limits."
                )));
                None
            }
        };

        Self {
            cmd_tx,
            event_rx: Some(event_rx),
            broker: broker_for_handle,
            join,
        }
    }

    /// Hand the event stream to the Composer's `cx.spawn` task.
    /// Returns `None` on subsequent calls — the receiver lives in
    /// exactly one consumer. Push-based design replaces the previous
    /// 16 ms poll loop: the Composer awaits chunks directly so the
    /// pipeline hops drop from two-stage 16 ms polling to single-stage
    /// notification, matching Zed's terminal event loop semantics
    /// (`crates/terminal/src/terminal.rs:701`).
    pub fn take_event_receiver(&mut self) -> Option<UnboundedReceiver<RuntimeEvent>> {
        self.event_rx.take()
    }

    /// US-007 (cli-hardening-followup-2026-Q3): hand the OS-thread
    /// `JoinHandle` to the event-pump task so it can `.join()` the
    /// thread asynchronously after the event channel closes and
    /// surface any panic payload as a synthetic `RuntimeEvent::Fatal`
    /// on the Composer. The previous `Drop`-time `let _ = self.join.take()`
    /// detached the thread silently, masking panics that left the
    /// US-019 streaming spinner stuck indefinitely. Returns `None` if
    /// the thread spawn at construction failed (the `Fatal` was
    /// already emitted at that point).
    pub fn take_join_handle(&mut self) -> Option<std::thread::JoinHandle<()>> {
        self.join.take()
    }

    /// Send a `session/prompt` with a plain text body. Convenience
    /// wrapper around [`Self::send_prompt_blocks`] for the common
    /// "user just typed text" path.
    pub fn send_prompt(&self, text: String) {
        let block = ContentBlock::Text(TextContent::new(text));
        self.send_prompt_blocks(vec![block]);
    }

    /// US-019: send a `session/prompt` with a fully-built block
    /// list (text + image + resource link). Non-blocking; success is
    /// signalled via a `Chunk` / `TurnEnded` event sequence.
    pub fn send_prompt_blocks(&self, blocks: Vec<ContentBlock>) {
        // US-019 (audit P2-9): a closed channel here is symptomatic
        // of the runtime thread having exited (panic, fatal init
        // error, or Drop racing the caller). Logging at warn surfaces
        // the dropped prompt in any bug report instead of silently
        // swallowing user input.
        if let Err(err) = self.cmd_tx.send(Command::SendPrompt(blocks)) {
            log::warn!(
                target: "paneflow_app::agents::runtime",
                "cmd_tx.send(SendPrompt) failed (runtime shut down? prompt lost): {err}"
            );
        }
    }

    /// Send a `session/cancel` notification. AC #3 / #8: the
    /// notification leaves the GPUI thread within the same event
    /// cycle (no blocking).
    pub fn cancel(&self) {
        let _ = self.cmd_tx.send(Command::Cancel);
    }

    /// Send `session/set_mode`. AC #6.
    ///
    /// Mirrors the SendPrompt logging: SetMode is a user-driven state
    /// change (mode-picker click) that, if dropped on a closed
    /// channel, would leave the UI showing the old mode forever. A
    /// warn-level log surfaces the dropped notification in any bug
    /// report (US-019 review follow-up; Cancel/Shutdown stay silent
    /// because they're fire-and-forget lifecycle signals).
    pub fn set_mode(&self, mode_id: SessionModeId) {
        if let Err(err) = self.cmd_tx.send(Command::SetMode(mode_id)) {
            log::warn!(
                target: "paneflow_app::agents::runtime",
                "cmd_tx.send(SetMode) failed (runtime shut down?): {err} -- UI may stay on previous mode",
            );
        }
    }

    /// Send `session/set_model`. Mirrors [`Self::set_mode`] for the
    /// model picker — same warn-on-drop rationale: a silently dropped
    /// set_model leaves the UI showing the old model.
    pub fn set_model(&self, model_id: ModelId) {
        if let Err(err) = self.cmd_tx.send(Command::SetModel(model_id)) {
            log::warn!(
                target: "paneflow_app::agents::runtime",
                "cmd_tx.send(SetModel) failed (runtime shut down?): {err} -- UI may stay on previous model",
            );
        }
    }

    /// US-018: resolve a pending permission request fired by an
    /// earlier [`RuntimeEvent::PermissionRequest`]. The Composer
    /// forwards Allow / Deny button clicks here; the runtime's
    /// loop then completes the oneshot held in [`PermissionBroker`].
    pub fn resolve_permission(&self, tool_call_id: String, decision: PermissionDecision) {
        let _ = self
            .cmd_tx
            .send(Command::ResolvePermission(tool_call_id, decision));
    }
}

impl Drop for SessionRuntime {
    fn drop(&mut self) {
        // US-018: unblock the agent on any in-flight permission
        // request before tearing down -- otherwise the tokio loop
        // would await a oneshot whose Sender we are about to drop,
        // and the agent's `request_permission` call would hang
        // until JSON-RPC timed out. Resolving as Deny is the safe
        // default (matches the auth-required spirit of US-005).
        if let Some(broker) = self.broker.as_ref() {
            broker.drain_as_deny();
        }
        // Signal the loop to break, then drop the sender so any
        // subsequent recv() on the runtime side returns
        // `Disconnected` (covers the case where the loop was
        // blocked on a different command path). We do NOT block on
        // join here: the JoinHandle was extracted by
        // `take_join_handle()` and lives on the event-pump task
        // (US-007 cli-hardening-followup-2026-Q3); the task awaits
        // the join with a 5 s deadline and emits a Fatal banner on
        // panic. If the handle was never extracted (Composer dropped
        // before consuming the runtime), we fall back to detach --
        // same outcome as the pre-US-007 path.
        let _ = self.cmd_tx.send(Command::Shutdown);
        let _ = self.join.take();
    }
}

/// Channel-backed [`NotificationSink`]. The tokio side calls
/// `handle()` from inside the SDK's notification dispatch; we
/// translate the verbose `SessionNotification` into the smaller
/// [`RuntimeEvent`] surface and forward.
struct ChannelSink {
    tx: UnboundedSender<RuntimeEvent>,
}

impl NotificationSink for ChannelSink {
    fn handle(&self, notification: SessionNotification) {
        match notification.update {
            SessionUpdate::AgentMessageChunk(ContentChunk {
                content: ContentBlock::Text(TextContent { text, .. }),
                ..
            }) => {
                let _ = self.tx.unbounded_send(RuntimeEvent::Chunk(text));
            }
            // Non-text chunks (images, audio, resource links) on
            // either message or thought streams are out of scope
            // for US-016. US-019 wires attachments; US-017 wires
            // tool-call cards.
            SessionUpdate::AgentMessageChunk(_) => {}
            SessionUpdate::AgentThoughtChunk(ContentChunk {
                content: ContentBlock::Text(TextContent { text, .. }),
                ..
            }) => {
                let _ = self.tx.unbounded_send(RuntimeEvent::Thought(text));
            }
            SessionUpdate::AgentThoughtChunk(_) => {}
            // US-017: forward the two tool-call notifications to
            // the Composer pump. ToolCall is the initial declaration;
            // ToolCallUpdate is a patch (status/title/content/...).
            SessionUpdate::ToolCall(tc) => {
                let _ = self
                    .tx
                    .unbounded_send(RuntimeEvent::ToolCall(snapshot_from_tool_call(tc)));
            }
            SessionUpdate::ToolCallUpdate(u) => {
                let _ = self.tx.unbounded_send(RuntimeEvent::ToolCallUpdate(
                    update_from_tool_call_update(u),
                ));
            }
            // US-112: forward the agent's advertised slash commands so
            // the composer can merge them with built-ins. An empty vec
            // is a valid value (it resets the cache).
            SessionUpdate::AvailableCommandsUpdate(u) => {
                let _ = self
                    .tx
                    .unbounded_send(RuntimeEvent::AvailableCommandsUpdate(u.available_commands));
            }
            // US-120 / US-121: forward the cumulative token-usage update
            // (unstable ACP feature) to the composer. Today no shipping
            // wrapper emits this, so the arm is dead under production
            // wiring -- the moment Claude Code / Codex flips the flag,
            // the badge + throughput suffix light up with no further
            // plumbing change.
            SessionUpdate::UsageUpdate(u) => {
                let _ = self.tx.unbounded_send(RuntimeEvent::UsageUpdate {
                    used: u.used,
                    size: u.size,
                });
            }
            // ACP `session/info_update` notification. Agents (Claude
            // Code, Codex, ...) push it whenever they generate or
            // refresh a session summary; surfaces under `/resume` in
            // Claude Code as the "title" of past sessions. Trim +
            // forward only non-empty values; Null / Undefined / empty
            // strings leave the current title alone.
            SessionUpdate::SessionInfoUpdate(info) => {
                if let Some(title) = info.title.take() {
                    let trimmed = title.trim();
                    if !trimmed.is_empty() {
                        let _ = self
                            .tx
                            .unbounded_send(RuntimeEvent::SessionTitle(trimmed.to_string()));
                    }
                }
            }
            _ => {
                // Plan / user-message chunks / mode update etc. --
                // not routed yet. Future stories layer on top.
            }
        }
    }
}

/// The tokio thread's entry point. Builds a current-thread runtime,
/// spawns the ACP agent, and `block_on`s the event loop until
/// shutdown.
#[allow(clippy::too_many_arguments)]
fn run_blocking(
    spawn_command: String,
    cwd: PathBuf,
    permission_callback: Arc<dyn PermissionCallback>,
    terminal_spawner_override: Option<Arc<dyn TerminalSpawner>>,
    broker: Arc<PermissionBroker>,
    cmd_rx: CommandReceiver<Command>,
    event_tx: UnboundedSender<RuntimeEvent>,
    resume_session_id: Option<SessionId>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            let _ = event_tx.unbounded_send(RuntimeEvent::Fatal(format!(
                "failed to build tokio runtime: {err}"
            )));
            return;
        }
    };

    // LocalSet so the command loop can `spawn_local` the in-flight
    // prompt as a detached task -- see the `Command::SendPrompt` arm
    // below for why this matters (Stop button responsiveness).
    let local_set = tokio::task::LocalSet::new();
    local_set.block_on(&runtime, async move {
        let agent = match spawn_acp_agent(&spawn_command).await {
            Ok(a) => a,
            Err(err) => {
                let _ = event_tx.unbounded_send(RuntimeEvent::Fatal(format!(
                    "spawn_acp_agent failed: {err}"
                )));
                return;
            }
        };

        // US-018: build a SessionRegistry up front so the terminal
        // spawner (which needs cwd lookups for the sandbox check)
        // and the file-ops handler share the same map. ClientConfig
        // creates an empty registry by default; we replace it with
        // ours so the spawner sees the same data.
        let sessions = paneflow_acp::SessionRegistry::new();
        let terminal_spawner: Arc<dyn TerminalSpawner> = match terminal_spawner_override {
            Some(s) => s,
            None => super::agent_terminal::AgentTerminalSpawner::new(sessions.clone()),
        };

        let mut config = ClientConfig::new(permission_callback, terminal_spawner)
            .with_notification_sink(Arc::new(ChannelSink {
                tx: event_tx.clone(),
            }));
        config.sessions = sessions.clone();

        // Register the cwd up front (the file-ops handler reads from
        // the SessionRegistry to enforce the sandbox). The session
        // id is not known until session/new returns, so we register
        // after that resolves.
        let cwd_for_registry = cwd.clone();

        let event_tx_for_main = event_tx.clone();
        // US-020 (cli-hardening-followup-2026-Q3): the receiver is
        // now `tokio::sync::mpsc::UnboundedReceiver<Command>` --
        // async-native, owned by this task directly. The previous
        // `Arc<Mutex<std::sync::mpsc::Receiver>>` + spawn_blocking
        // dance is gone.
        let mut cmd_rx = cmd_rx;

        let connect_result =
            connect_with_handlers(agent, config, async move |cx: ConnectionTo<Agent>| {
                // US-101: wrap the raw ConnectionTo<Agent> in the
                // polymorphic `AgentConnection` trait surface. Every
                // dispatch below goes through the trait so swapping in
                // a non-ACP backend (Gemini CLI, OpenCode TCP, ...)
                // requires only a new `impl AgentConnection` block,
                // not touching this command loop.
                let conn: Arc<dyn AgentConnection> = Arc::new(AcpConnection::new(cx));

                // Init handshake.
                if let Err(err) = conn.initialize().await {
                    let _ = event_tx_for_main
                        .unbounded_send(RuntimeEvent::Fatal(format!("initialize failed: {err}")));
                    return Ok(());
                }

                // US-118: branch between native session/load and the
                // local new-session-plus-replay path. The trait flag
                // is authoritative -- when `supports_load_session`
                // returns false (today's reality for every shipping
                // ACP wrapper) we always take the new-session path
                // even when `resume_session_id` was supplied, so the
                // ThreadView's local blob replay stays the source of
                // truth. Once a wrapper flips the flag, this branch
                // calls `load_session` and the agent re-binds without
                // any further UI change.
                let session = match resume_session_id.clone() {
                    Some(id) if conn.supports_load_session() => {
                        match conn
                            .load_session(id.clone(), cwd_for_registry.clone())
                            .await
                        {
                            Ok(resp) => SessionEstablished::from_load(id, resp),
                            Err(err) => {
                                // US-118 AC #3: partial-load failure
                                // surfaces a self-describing error in
                                // the message stream area so the user
                                // can tell why the resume failed and
                                // start a new thread.
                                let _ = event_tx_for_main.unbounded_send(RuntimeEvent::Fatal(
                                    format!("Failed to resume agent -- start a new thread ({err})"),
                                ));
                                return Ok(());
                            }
                        }
                    }
                    _ => {
                        let response = match conn.new_session(cwd_for_registry.clone()).await {
                            Ok(s) => s,
                            Err(err) => {
                                let _ = event_tx_for_main.unbounded_send(RuntimeEvent::Fatal(
                                    format!("new_session failed: {err}"),
                                ));
                                return Ok(());
                            }
                        };
                        SessionEstablished::from_new(response)
                    }
                };

                sessions.register(session.session_id.clone(), cwd_for_registry);

                let _ = event_tx_for_main.unbounded_send(RuntimeEvent::SessionReady {
                    modes: session.modes.clone(),
                    current_mode_id: session.current_mode_id.clone(),
                    models: session.models.clone(),
                    current_model_id: session.current_model_id.clone(),
                });

                // US-020 (cli-hardening-followup-2026-Q3): drive
                // commands from the GPUI side via the async tokio
                // receiver. `recv().await` yields cooperatively so
                // the current-thread runtime keeps servicing the
                // JSON-RPC reactor AND any in-flight prompt task
                // between commands. The previous spawn_blocking +
                // mutex + join overhead (~100 us per command) is
                // gone. A `None` return means the GPUI side dropped
                // the sender (clean shutdown via Drop); the runtime
                // loop exits gracefully. The pre-fix Mutex-poison
                // and tokio-spawn-blocking-join failure branches are
                // both unreachable now -- there is no Mutex and no
                // spawn_blocking on this path.
                loop {
                    let cmd = match cmd_rx.recv().await {
                        Some(c) => c,
                        None => {
                            log::info!(
                                target: "paneflow_app::agents::runtime",
                                "ACP cmd_rx sender closed -- runtime loop exiting cleanly"
                            );
                            return Ok(());
                        }
                    };

                    match cmd {
                        Command::SendPrompt(blocks) => {
                            // SPAWN the prompt as its own task so the
                            // command loop stays free to process the
                            // next command -- crucially `Command::Cancel`.
                            //
                            // The previous code `match conn.prompt(...).await`
                            // blocked the loop for the entire duration
                            // of the turn; a Stop click queued a Cancel
                            // command on `cmd_tx` but `recv()` couldn't
                            // pick it up until the prompt returned, by
                            // which point the cancellation was useless.
                            //
                            // Zed reaches the same outcome differently:
                            // `acp_thread::AcpThread::cancel`
                            // (`crates/acp_thread/src/acp_thread.rs:2527`)
                            // calls `connection.cancel(&session_id, cx)`
                            // DIRECTLY on the shared connection, with no
                            // command queue gating the dispatch. We
                            // emulate that by detaching the in-flight
                            // prompt from the command loop -- the loop
                            // can then process Cancel on the very next
                            // iteration and route it straight through to
                            // `conn.cancel(...)`.
                            let conn_for_prompt = conn.clone();
                            let session_id = session.session_id.clone();
                            let event_tx_for_prompt = event_tx_for_main.clone();
                            tokio::task::spawn_local(async move {
                                match conn_for_prompt.prompt(session_id, blocks).await {
                                    Ok(resp) => {
                                        let _ = event_tx_for_prompt.unbounded_send(
                                            RuntimeEvent::TurnEnded(resp.stop_reason.into()),
                                        );
                                    }
                                    Err(err) => {
                                        let _ = event_tx_for_prompt.unbounded_send(
                                            RuntimeEvent::Fatal(format!("prompt failed: {err}")),
                                        );
                                    }
                                }
                            });
                        }
                        Command::Cancel => {
                            conn.cancel(session.session_id.clone());
                        }
                        Command::SetMode(mode_id) => {
                            if let Err(err) = conn
                                .set_session_mode(session.session_id.clone(), mode_id)
                                .await
                            {
                                tracing::warn!(
                                    target: "paneflow_app::agents::runtime",
                                    "set_mode failed: {err}",
                                );
                            }
                        }
                        Command::SetModel(model_id) => {
                            if let Err(err) = conn
                                .set_session_model(session.session_id.clone(), model_id)
                                .await
                            {
                                tracing::warn!(
                                    target: "paneflow_app::agents::runtime",
                                    "set_model failed: {err}",
                                );
                            }
                        }
                        Command::ResolvePermission(tool_call_id, decision) => {
                            // US-018: looks up the pending oneshot
                            // and completes it; the awaiting
                            // PermissionCallback future then maps
                            // the decision to the ACP outcome
                            // (`permission::map_decision`).
                            broker.resolve(&tool_call_id, decision);
                        }
                        Command::Shutdown => return Ok(()),
                    }
                }
            })
            .await;

        if let Err(err) = connect_result {
            let _ = event_tx
                .unbounded_send(RuntimeEvent::Fatal(format!("ACP connection error: {err}")));
        }
    });
}

/// US-118: unifies the post-init state regardless of which entry
/// point established the session. `from_new` is the today-default
/// branch; `from_load` is the future branch that lights up the moment
/// a wrapper ships `supports_load_session = true`.
struct SessionEstablished {
    session_id: SessionId,
    modes: Vec<SessionMode>,
    current_mode_id: Option<SessionModeId>,
    models: Vec<ModelChoice>,
    current_model_id: Option<String>,
}

impl SessionEstablished {
    fn from_new(response: agent_client_protocol::schema::NewSessionResponse) -> Self {
        let (modes, current_mode_id) = match response.modes.as_ref() {
            Some(state) => (
                state.available_modes.clone(),
                Some(state.current_mode_id.clone()),
            ),
            None => (Vec::new(), None),
        };
        let (models, current_model_id) = extract_model_state_new(&response);
        Self {
            session_id: response.session_id.clone(),
            modes,
            current_mode_id,
            models,
            current_model_id,
        }
    }

    fn from_load(
        session_id: SessionId,
        response: agent_client_protocol::schema::LoadSessionResponse,
    ) -> Self {
        let (modes, current_mode_id) = match response.modes.as_ref() {
            Some(state) => (
                state.available_modes.clone(),
                Some(state.current_mode_id.clone()),
            ),
            None => (Vec::new(), None),
        };
        let (models, current_model_id) = extract_model_state_load(&response);
        Self {
            session_id,
            modes,
            current_mode_id,
            models,
            current_model_id,
        }
    }
}

/// US-118: model-state extractor for [`LoadSessionResponse`]. Same
/// `unstable_session_model` gating as [`extract_model_state_new`] so
/// the build stays clean without the feature flag.
fn extract_model_state_load(
    response: &agent_client_protocol::schema::LoadSessionResponse,
) -> (Vec<ModelChoice>, Option<String>) {
    let Some(state) = response.models.as_ref() else {
        return (Vec::new(), None);
    };
    let models = state
        .available_models
        .iter()
        .map(|m| ModelChoice {
            id: m.model_id.to_string(),
            name: m.name.clone(),
        })
        .collect();
    (models, Some(state.current_model_id.to_string()))
}

/// Pull `(available_models, current_model_id)` from a
/// `NewSessionResponse`. The model state is gated behind the
/// `unstable_session_model` feature; the function compiles either
/// way -- without the feature it returns empty vecs.
fn extract_model_state_new(
    session: &agent_client_protocol::schema::NewSessionResponse,
) -> (Vec<ModelChoice>, Option<String>) {
    let Some(state) = session.models.as_ref() else {
        return (Vec::new(), None);
    };
    let models = state
        .available_models
        .iter()
        .map(|m| ModelChoice {
            id: m.model_id.to_string(),
            name: m.name.clone(),
        })
        .collect();
    (models, Some(state.current_model_id.to_string()))
}

/// Convenience: a permission callback that always denies. Kept
/// around for tests that need a deterministic policy without
/// driving the PermissionBroker. (Production code passes
/// `permission_callback: None` so the broker handles each request
/// through the Composer.)
pub fn deny_all_permissions() -> Arc<dyn PermissionCallback> {
    always_deny()
}
