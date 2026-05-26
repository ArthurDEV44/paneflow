#![allow(dead_code)]

//! Virtual-scroll chat log for one agent thread, with an auto-follow-
//! tail flag that disengages when the user scrolls up and re-engages
//! when they return to the bottom or send a new message.
//!
//! Inline-timeline data model (matches Zed's `AgentThreadEntry`):
//!
//! - `ThreadItem::UserMessage`     — one user (or system) message
//! - `ThreadItem::AssistantMessage` — one assistant turn, made of an
//!   ordered list of [`AssistantMessageChunk::Text`] and
//!   [`AssistantMessageChunk::Thought`] chunks
//! - `ThreadItem::ToolCall`        — a top-level tool invocation
//!
//! The timeline is flat: every thinking burst lives inside an
//! AssistantMessage (as a `Thought` chunk), and every tool call is a
//! top-level row alongside the messages — there is no grouping card.
//! This is the Zed layout: each thinking / tool call is a single
//! editorial line in the conversation.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnimationExt, AnyElement, AppContext, AsyncApp, ClickEvent, Context, Entity, EventEmitter,
    InteractiveElement, IntoElement, ListAlignment, ListScrollEvent, ListSizingBehavior, ListState,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Task, WeakEntity,
    Window, div, list, px, rgb,
};
use markdown::Markdown;
use paneflow_acp::{AgentDiscovery, AgentKind, PacingConfig, StreamingBuffer};
use paneflow_threads::{
    ContentBlock, Message, MessageRole, PersistedAssistant, PersistedAssistantChunk,
    PersistedThreadItem, ThreadId, ThreadStore,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use super::composer::Composer;
use super::runtime::{ToolCallSnapshot, ToolCallUpdate, ToolKindKind};

/// Tick cadence for the streaming pipeline.
const STREAMING_TICK: Duration = Duration::from_millis(16);

/// One row in the thread's scrollable timeline. Mirrors Zed's
/// `AgentThreadEntry` (acp_thread.rs:178): user messages, assistant
/// turns (with internal text+thought chunks), and top-level tool
/// calls live as separate variants — no grouping card.
pub enum ThreadItem {
    UserMessage(UserMessage),
    AssistantMessage(AssistantMessage),
    ToolCall(ToolCallSnapshot),
}

impl std::fmt::Debug for ThreadItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadItem::UserMessage(_) => f.debug_tuple("UserMessage").finish(),
            ThreadItem::AssistantMessage(_) => f.debug_tuple("AssistantMessage").finish(),
            ThreadItem::ToolCall(t) => f.debug_tuple("ToolCall").field(t).finish(),
        }
    }
}

/// One user (or system) chat message, plus the `Markdown` entity that
/// renders its body.
pub struct UserMessage {
    pub msg: Message,
    pub markdown: Option<Entity<Markdown>>,
}

/// One assistant turn: an ordered sequence of text and thought
/// chunks. New chunks append to the trailing assistant message (the
/// one immediately before a tool call or end-of-thread); a tool call
/// or user message naturally interrupts the turn, so the next
/// inbound chunk opens a fresh [`AssistantMessage`] just by virtue
/// of not finding one at the back of the timeline.
pub struct AssistantMessage {
    pub chunks: Vec<AssistantMessageChunk>,
}

/// One chunk inside an assistant turn. Mirrors Zed's
/// `AssistantMessageChunk` (acp_thread.rs:150).
pub enum AssistantMessageChunk {
    /// Regular assistant body — rendered as markdown.
    Text {
        text: String,
        markdown: Entity<Markdown>,
    },
    /// Inline "Thinking" burst — rendered as a collapsible row with a
    /// muted lightbulb icon. Body is hidden by default; the user
    /// expands it via the disclosure chevron.
    Thought {
        text: String,
        markdown: Entity<Markdown>,
        signature: Option<String>,
    },
}

/// One thread's chat log, virtual-scrolled.
pub struct ThreadView {
    store_id: Option<ThreadId>,
    store: Option<ThreadStore>,
    /// Ordered timeline. Each item is rendered in its own row of the
    /// virtual list.
    items: Vec<ThreadItem>,
    list_state: ListState,
    should_be_following: Rc<Cell<bool>>,
    streaming_buffer: StreamingBuffer,
    /// Index in `items` of the currently-open assistant message, or
    /// `None` when no streaming turn is active. The currently-open
    /// assistant message is the only one new `Text` / `Thought` chunks
    /// append to; user messages and tool calls close it.
    streaming_message_idx: Option<usize>,
    _streaming_task: Option<Task<()>>,
    composer: Option<Entity<Composer>>,
    editing: Option<EditState>,
    /// Expand state for inline `Thought` chunks. Keyed by
    /// `(entry_ix, chunk_ix)` so chunks survive reorderings as long
    /// as the entry stays at the same index.
    collapsed_thoughts: HashSet<(usize, usize)>,
    /// Persistent `Markdown` entities for tool-call labels, keyed by
    /// the snapshot id. The tool title arrives as a markdown string
    /// (e.g. ``"Read file `Cargo.toml` (lines 1-160)"``) — rendering
    /// it via `MarkdownElement` is what gets the backtick path
    /// styled as inline code, matching Zed verbatim.
    tool_label_markdown: HashMap<String, Entity<Markdown>>,

    /// Tracks whether the GPUI `list_state` currently has a virtual
    /// extra slot reserved at the tail for the inline pixel spinner
    /// (`render_generating_spinner`). Kept in sync inside `Render`
    /// with `composer.is_streaming()`: when it flips, the matching
    /// `splice(len..len, 1)` / `splice(len..len+1, 0)` rebalances
    /// the list_state so item count stays consistent. Mirrors Zed's
    /// `generating_indicator_in_list` at
    /// `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:4848`.
    generating_indicator_active: bool,
    /// Zed-parity (2026-05-25): set of tool-call ids whose edits the
    /// user has already accepted or reverted via Keep All / Reject All.
    /// A reviewed id hides the review footer in `edit_tool_block.rs`.
    /// Lives in the view (not the snapshot) so subsequent ACP updates
    /// can't accidentally re-open the footer.
    reviewed_edits: HashSet<String>,
    /// One `ScrollHandle` per edit-tool card, keyed by tool-call id.
    /// `.track_scroll(handle)` on the diff body is what tells GPUI to
    /// route wheel events to that handle exclusively when the cursor
    /// is over the card. Without an explicit tracked handle, the
    /// surrounding `list()` widget also consumes the wheel and the
    /// thread scrolls in sync with the diff. Handles persist across
    /// renders so the scroll position is preserved.
    diff_scroll_handles: HashMap<String, gpui::ScrollHandle>,
}

/// US-020: inline edit-in-progress on a user message.
#[derive(Clone)]
pub struct EditState {
    pub message_idx: usize,
    pub text_area: Entity<crate::widgets::text_area::TextArea>,
    pub original_text: String,
}

#[derive(Debug, Clone)]
pub struct ForkRequested {
    pub message_idx: usize,
    pub new_text: String,
}

impl EventEmitter<ForkRequested> for ThreadView {}

/// Suggested human-readable title for this thread. Two sources feed
/// this event: the ACP `SessionInfoUpdate` notification (agent-pushed
/// summary -- forward-compat, dormant on Claude Code 0.16 / Codex
/// 0.14) and a client-side fallback that derives a title from the
/// first user message on send.
#[derive(Debug, Clone)]
pub struct TitleSuggested {
    pub title: String,
    /// Tri-state replace gate. See [`TitleReplacePolicy`] for the
    /// rationale -- the background summarizer is the reason this
    /// can't be a single boolean any more (it needs the "still equal
    /// to a captured snapshot" form to lose races with user renames).
    pub policy: TitleReplacePolicy,
}

/// How aggressively a new title suggestion should override the
/// current sidebar row title. Modelled after Zed's `Thread::title`
/// replacement guards (`agent/src/thread.rs` around `set_title` and
/// `generate_title`): an ACP push wins unconditionally, an auto-
/// derive only fills the default, and the async summarizer must not
/// clobber a rename that landed while it was waiting on the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleReplacePolicy {
    /// Replace regardless of the current value. Used by ACP-pushed
    /// `SessionInfoUpdate.title` (the agent's authoritative summary).
    Always,
    /// Replace only when the current title is still the literal
    /// `"New thread"` sentinel. Used by the client-side auto-derive
    /// on the first user prompt so a user rename or an agent push
    /// already in flight is never clobbered.
    OnlyIfDefault,
    /// Replace only when the current title still matches a captured
    /// snapshot. Used by the background title summarizer (Zed
    /// `Thread::generate_title` parity): the summarizer captures the
    /// title at trigger time, awaits a transient session round-trip
    /// (~1-3 s), and only applies the result if no user rename or
    /// agent push happened in the meantime.
    OnlyIfStillEqualTo(String),
}

impl EventEmitter<TitleSuggested> for ThreadView {}

impl ThreadView {
    pub fn new(
        store_id: Option<ThreadId>,
        store: Option<ThreadStore>,
        agent_kind: AgentKind,
        cwd: PathBuf,
        discovery: Arc<AgentDiscovery>,
        cx: &mut Context<Self>,
    ) -> Self {
        let persisted_items = match (store.as_ref(), store_id.as_ref()) {
            (Some(s), Some(id)) => s.load_items(id).unwrap_or_else(|err| {
                log::warn!("ThreadView: load_items failed: {err}");
                Vec::new()
            }),
            _ => Vec::new(),
        };

        let mut items: Vec<ThreadItem> = Vec::with_capacity(persisted_items.len());
        // The `reviewed` flag travels with each `PersistedToolCall` but
        // `ToolCallSnapshot::from_persisted` drops it (the snapshot has
        // no field for it -- review state lives on the view). Harvest
        // it here so the activity-bar footer stays dismissed across
        // reloads instead of reappearing every time the ThreadView is
        // rebuilt (sidebar re-mount, app restart).
        let mut reviewed_edits: HashSet<String> = HashSet::new();
        for p in persisted_items {
            match p {
                PersistedThreadItem::Message(m) => {
                    let markdown = match m.role {
                        MessageRole::User | MessageRole::System => {
                            let text = join_text_blocks(&m.content);
                            Some(Self::make_markdown(&text, cx))
                        }
                        // Assistant messages are stored as the
                        // `Assistant` variant; an `assistant`-roled
                        // `Message` only ever arrives from older
                        // builds — render it as a single-chunk
                        // assistant turn for forward compatibility.
                        MessageRole::Assistant => {
                            let text = join_text_blocks(&m.content);
                            let chunk = AssistantMessageChunk::Text {
                                text: text.clone(),
                                markdown: Self::make_markdown(&text, cx),
                            };
                            items.push(ThreadItem::AssistantMessage(AssistantMessage {
                                chunks: vec![chunk],
                            }));
                            continue;
                        }
                    };
                    items.push(ThreadItem::UserMessage(UserMessage { msg: m, markdown }));
                }
                PersistedThreadItem::Assistant(a) => {
                    let mut chunks = Vec::with_capacity(a.chunks.len());
                    for c in a.chunks {
                        chunks.push(match c {
                            PersistedAssistantChunk::Text { text } => AssistantMessageChunk::Text {
                                markdown: Self::make_markdown(&text, cx),
                                text,
                            },
                            PersistedAssistantChunk::Thought { text, signature } => {
                                AssistantMessageChunk::Thought {
                                    markdown: Self::make_markdown(&text, cx),
                                    text,
                                    signature,
                                }
                            }
                        });
                    }
                    items.push(ThreadItem::AssistantMessage(AssistantMessage { chunks }));
                }
                PersistedThreadItem::Tool(p) => {
                    if p.reviewed {
                        reviewed_edits.insert(p.id.clone());
                    }
                    items.push(ThreadItem::ToolCall(ToolCallSnapshot::from_persisted(p)));
                }
            }
        }

        let count = items.len();
        let list_state = ListState::new(count, ListAlignment::Top, px(400.));
        let should_be_following = Rc::new(Cell::new(true));
        {
            let flag = Rc::clone(&should_be_following);
            list_state.set_scroll_handler(move |event: &ListScrollEvent, _w, _app| {
                let at_bottom = is_at_bottom(event);
                if !at_bottom {
                    flag.set(false);
                } else if event.is_scrolled {
                    flag.set(true);
                }
            });
        }
        if count > 0 {
            list_state.scroll_to_end();
        }

        let weak_self = cx.weak_entity();
        let draft_id = store_id.clone();
        let draft_store = store.clone();
        let composer = Some(cx.new(|cx| {
            Composer::new(
                agent_kind,
                discovery,
                cwd,
                weak_self,
                draft_id,
                draft_store,
                cx,
            )
        }));

        Self {
            store_id,
            store,
            items,
            list_state,
            should_be_following,
            streaming_buffer: StreamingBuffer::new(PacingConfig::default()),
            streaming_message_idx: None,
            _streaming_task: None,
            composer,
            editing: None,
            collapsed_thoughts: HashSet::new(),
            tool_label_markdown: HashMap::new(),
            generating_indicator_active: false,
            reviewed_edits,
            diff_scroll_handles: HashMap::new(),
        }
    }

    fn make_markdown(text: &str, cx: &mut Context<Self>) -> Entity<Markdown> {
        cx.new(|cx| Markdown::new(SharedString::new(text), None, None, cx))
    }

    /// Append a chat message to the in-memory buffer and the GPUI
    /// list counter, then persist. User messages stay as
    /// `UserMessage`; assistant messages route through the
    /// chunk-based `AssistantMessage` model so subsequent thoughts /
    /// streamed text fold into the same turn.
    pub fn append_message(&mut self, msg: Message, cx: &mut Context<Self>) {
        match msg.role {
            MessageRole::User | MessageRole::System => {
                self.close_open_assistant_message();
                let text = join_text_blocks(&msg.content);
                let markdown = Some(Self::make_markdown(&text, cx));
                let prev_count = self.items.len();
                self.items
                    .push(ThreadItem::UserMessage(UserMessage { msg, markdown }));
                self.list_state.splice(prev_count..prev_count, 1);
            }
            MessageRole::Assistant => {
                let text = join_text_blocks(&msg.content);
                let idx = self.ensure_open_assistant_message(cx);
                if let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(idx) {
                    let markdown = Self::make_markdown(&text, cx);
                    am.chunks
                        .push(AssistantMessageChunk::Text { text, markdown });
                }
            }
        }
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    /// Push a top-level tool call. Always appends as its own row;
    /// closes the currently-open assistant message so subsequent
    /// chunks open a fresh turn.
    pub fn add_tool_call(&mut self, snapshot: ToolCallSnapshot, cx: &mut Context<Self>) {
        self.close_open_assistant_message();
        let id = snapshot.id.clone();
        let title = snapshot.title.clone();
        let prev_count = self.items.len();
        self.items.push(ThreadItem::ToolCall(snapshot));
        self.list_state.splice(prev_count..prev_count, 1);
        let md = Self::make_markdown(&title, cx);
        self.tool_label_markdown.insert(id, md);
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    /// Append a thinking chunk to the currently-open assistant
    /// message. Opens a new assistant turn if none is active.
    pub fn push_thinking_chunk(&mut self, chunk: &str, cx: &mut Context<Self>) {
        if chunk.is_empty() {
            return;
        }
        let idx = self.ensure_open_assistant_message(cx);
        let owned = chunk.to_string();
        // Merge with the trailing chunk when it is also a Thought, so
        // a single thinking burst sent in N pieces renders as one
        // collapsible block.
        if let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(idx) {
            match am.chunks.last_mut() {
                Some(AssistantMessageChunk::Thought { text, markdown, .. }) => {
                    text.push_str(&owned);
                    markdown.update(cx, |m, cx| m.append(&owned, cx));
                }
                _ => {
                    let md = Self::make_markdown(&owned, cx);
                    am.chunks.push(AssistantMessageChunk::Thought {
                        text: owned,
                        markdown: md,
                        signature: None,
                    });
                }
            }
        }
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    /// Close the currently-open assistant turn. After this returns,
    /// the next text / thought chunk opens a fresh turn. Idempotent.
    pub fn finalize_thinking(&mut self, cx: &mut Context<Self>) {
        self.close_open_assistant_message();
        self.persist_snapshot_now(cx);
    }

    /// Sweep any tool calls still in `Pending` / `InProgress` /
    /// `WaitingForConfirmation` to a terminal status when the turn
    /// ends. Defensive against ACP backends that finish the turn
    /// without sending a final `Completed` / `Failed` status update
    /// for every tool call -- without this sweep, the activity bar's
    /// "Reading file" spinner never stops because
    /// `compute_activity_state` keeps finding an `InProgress` snapshot.
    ///
    /// Behaviour by `StopReasonKind`:
    /// - `EndTurn`: the agent finished cleanly so any leftover
    ///   in-flight tool must have succeeded enough for the model to
    ///   keep generating -- mark `Completed`. Covers the observed
    ///   Claude Code / Codex case where the final status update is
    ///   dropped between the tool result and the next text token.
    /// - `MaxTokens` / `MaxTurnRequests`: the agent had a working
    ///   model but ran out of budget mid-stream. Tool results that
    ///   came back DID succeed -- the model just couldn't continue.
    ///   Mark `Completed` so the panel doesn't lie about tool state.
    /// - `Cancelled`: the user / Paneflow killed the turn. Matches
    ///   Zed's `mark_pending_tools_as_canceled`
    ///   (`crates/acp_thread/src/acp_thread.rs:2540`): mark `Canceled`.
    /// - `Other`: unknown future variant -- safest default is
    ///   `Canceled` so the spinner stops without falsely claiming
    ///   success.
    /// - `Refusal`: the model refused to respond -- leave statuses
    ///   alone so the user sees the partial state the refusal
    ///   happened in. The composer's status row carries the refusal
    ///   indicator separately.
    ///
    /// Returns `true` when at least one tool call was transitioned
    /// so the caller knows to repaint / re-persist.
    pub fn sweep_pending_tools_at_turn_end(
        &mut self,
        reason: super::runtime::StopReasonKind,
        cx: &mut Context<Self>,
    ) -> bool {
        use super::runtime::{StopReasonKind, ToolCallStatusKind};
        let target = match reason {
            StopReasonKind::EndTurn
            | StopReasonKind::MaxTokens
            | StopReasonKind::MaxTurnRequests => ToolCallStatusKind::Completed,
            StopReasonKind::Cancelled | StopReasonKind::Other => ToolCallStatusKind::Canceled,
            StopReasonKind::Refusal => return false,
        };
        let mut touched = false;
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(
                    snap.status,
                    ToolCallStatusKind::Pending
                        | ToolCallStatusKind::InProgress
                        | ToolCallStatusKind::WaitingForConfirmation
                )
            {
                snap.status = target;
                touched = true;
            }
        }
        if touched {
            self.persist_snapshot_now(cx);
            cx.notify();
        }
        touched
    }

    /// Sweep all in-flight tool calls to `Failed` when the runtime
    /// dies (fatal error: agent process crashed, JSON-RPC pipe
    /// broke). Same shape as
    /// [`sweep_pending_tools_at_turn_end`] but always terminal-with-
    /// failure, because we can't ask the agent whether the call
    /// actually succeeded.
    pub fn sweep_pending_tools_on_fatal(&mut self, cx: &mut Context<Self>) -> bool {
        use super::runtime::ToolCallStatusKind;
        let mut touched = false;
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(
                    snap.status,
                    ToolCallStatusKind::Pending
                        | ToolCallStatusKind::InProgress
                        | ToolCallStatusKind::WaitingForConfirmation
                )
            {
                snap.status = ToolCallStatusKind::Failed;
                touched = true;
            }
        }
        if touched {
            self.persist_snapshot_now(cx);
            cx.notify();
        }
        touched
    }

    /// Apply a `ToolCallUpdate` patch. Silently no-ops on unknown
    /// ids (defensive: ACP servers occasionally emit updates for
    /// already-finalised calls).
    pub fn update_tool_call(&mut self, patch: ToolCallUpdate, cx: &mut Context<Self>) {
        let new_title = patch.title.clone();
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && snap.id == patch.id
            {
                let id = snap.id.clone();
                apply_patch(snap, patch);
                if let Some(title) = new_title {
                    let md = Self::make_markdown(&title, cx);
                    self.tool_label_markdown.insert(id, md);
                }
                self.persist_snapshot_now(cx);
                cx.notify();
                return;
            }
        }
    }

    /// Toggle the expand/collapse state of one tool-call row.
    pub fn toggle_tool_call_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && snap.id == id
            {
                snap.expanded = !snap.expanded;
                cx.notify();
                return;
            }
        }
    }

    /// `agent::KeepEdits` action handler. Keeps every unreviewed edit
    /// tool call in the thread in one shot -- the global Shift+Alt+Y
    /// equivalent of clicking "Keep All" on each card.
    pub fn keep_all_edits(
        &mut self,
        _: &crate::KeepEdits,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for item in &self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(snap.status, super::runtime::ToolCallStatusKind::Completed)
                && !snap.diffs.is_empty()
                && !self.reviewed_edits.contains(&snap.id)
                && self.reviewed_edits.insert(snap.id.clone())
            {
                changed = true;
            }
        }
        if changed {
            self.persist_snapshot_now(cx);
            cx.notify();
        }
    }

    /// `agent::RejectEdits` action handler. Reverts every unreviewed
    /// edit tool call in the thread -- the global Shift+Alt+Z
    /// equivalent of clicking "Reject All" on each card.
    pub fn reject_all_edits(
        &mut self,
        _: &crate::RejectEdits,
        _w: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let mut targets: Vec<(String, Vec<super::runtime::DiffSnapshot>)> = Vec::new();
        for item in &self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(snap.status, super::runtime::ToolCallStatusKind::Completed)
                && !snap.diffs.is_empty()
                && !self.reviewed_edits.contains(&snap.id)
            {
                targets.push((snap.id.clone(), snap.diffs.clone()));
            }
        }
        if targets.is_empty() {
            return;
        }
        for (id, diffs) in targets {
            for diff in &diffs {
                revert_one_diff(diff);
            }
            self.reviewed_edits.insert(id);
        }
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    /// US-124: toggle the inline pattern-picker popover for one tool
    /// call. `true` after the toggle means the picker is visible and
    /// the user is about to choose between "Everywhere" / parsed
    /// terminal pattern proposals. Closing all OTHER pickers on open
    /// keeps the UI to one popover at a time.
    pub fn toggle_permission_picker(&mut self, id: &str, cx: &mut Context<Self>) {
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item {
                if snap.id == id {
                    snap.permission_picker_open = !snap.permission_picker_open;
                } else if snap.permission_picker_open {
                    snap.permission_picker_open = false;
                }
            }
        }
        cx.notify();
    }

    /// US-124 AC #6: dismiss every open picker -- called by the
    /// outside-click handler so the popover behaves like the profile
    /// picker. Idempotent: no-op when nothing is open.
    pub fn close_permission_pickers(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && snap.permission_picker_open
            {
                snap.permission_picker_open = false;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Toggle the expand/collapse state of one inline `Thought` chunk.
    ///
    /// Thinking blocks now default to **expanded** (Zed parity --
    /// `crates/agent_ui/src/conversation_view/thread_view.rs:5831
    /// auto_expand_streaming_thought` keeps the body visible while a
    /// thought streams in, and `ThinkingBlockDisplay::AlwaysExpanded`
    /// keeps it visible after). The set therefore tracks the
    /// **user-collapsed** keys: empty set = every thought open;
    /// presence in the set = the user has hidden that specific block.
    pub fn toggle_thinking_expanded(&mut self, key: (usize, usize), cx: &mut Context<Self>) {
        if self.collapsed_thoughts.contains(&key) {
            self.collapsed_thoughts.remove(&key);
        } else {
            self.collapsed_thoughts.insert(key);
        }
        cx.notify();
    }

    /// Mark a tool call as awaiting user approval.
    pub fn set_pending_permission(
        &mut self,
        tool_call_id: &str,
        options: Vec<super::runtime::PermissionOptionInfo>,
        cx: &mut Context<Self>,
    ) {
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && snap.id == tool_call_id
            {
                snap.status = super::runtime::ToolCallStatusKind::WaitingForConfirmation;
                snap.permission_options = options;
                self.persist_snapshot_now(cx);
                cx.notify();
                return;
            }
        }
    }

    /// Clear a pending permission state after the user chose Allow /
    /// Deny.
    pub fn clear_pending_permission(
        &mut self,
        tool_call_id: &str,
        decision: paneflow_acp::PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && snap.id == tool_call_id
            {
                snap.permission_options.clear();
                snap.status = match decision {
                    paneflow_acp::PermissionDecision::AllowOnce
                    | paneflow_acp::PermissionDecision::AllowAlways => {
                        super::runtime::ToolCallStatusKind::InProgress
                    }
                    paneflow_acp::PermissionDecision::Reject => {
                        super::runtime::ToolCallStatusKind::Rejected
                    }
                };
                self.persist_snapshot_now(cx);
                cx.notify();
                return;
            }
        }
    }

    /// Inline pixel-spinner rendered just below the last visible
    /// message, in the gap between prompt submission and the first
    /// chunk / tool call, and kept visible throughout the turn so
    /// the user always has confirmation that something is happening.
    /// Mirrors Zed's `render_generating` at
    /// `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:5743`
    /// by reusing the exact same `SpinnerLabel` widget from Zed's
    /// `ui` crate (`crates/ui/src/components/label/spinner_label.rs`):
    /// a 10-frame Braille animation cycling every 1000ms in muted
    /// text color. No surrounding label so it reads as a tiny pixel
    /// indicator rather than chrome.
    fn render_generating_spinner(
        &self,
        ui: crate::theme::UiColors,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        use ui::{LabelCommon, LabelSize, SpinnerLabel};
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(20.))
            .py(px(8.))
            .child(SpinnerLabel::dots().size(LabelSize::Small))
            .child(
                div()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .child("Generating…"),
            )
            .into_any_element()
    }

    /// US-107: activity bar above the composer.
    fn render_activity_bar(&self, state: ActivityBarState, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let mut sections: Vec<AnyElement> = Vec::new();
        if let Some(running) = state.running {
            // US-121: assemble "Working · 12s · 1.2k tokens" by chaining
            // the optional suffixes after the kind label. The whole line
            // shares one row so the activity bar height stays constant
            // when the suffixes appear (AC #3).
            let mut composed = running.kind_label.clone();
            if let Some(elapsed) = running.elapsed_label.as_ref() {
                composed.push_str(" · ");
                composed.push_str(elapsed);
            }
            if let Some(tokens) = running.tokens_label.as_ref() {
                composed.push_str(" · ");
                composed.push_str(tokens);
            }
            let label: SharedString = composed.into();
            // Mirror Zed `thread_view.rs:5743-5828` `render_generating`
            // (US-010 in `prd-agent-ui-visual-parity-2026-Q3.md`):
            // `py_2 / px_{22px} / gap_2`. The outer bar already provides
            // px/h, so the section just matches gap_2 (8 px) for the
            // spinner→label spacing — that's the visual rhythm Zed gives
            // to the active turn indicator.
            let section = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .child(
                    gpui::svg()
                        .size(px(13.))
                        .flex_none()
                        .path("icons/loader-circle.svg")
                        .text_color(ui.text)
                        .with_animation(
                            "activity-bar-spinner",
                            gpui::Animation::new(Duration::from_secs(1))
                                .repeat()
                                .with_easing(gpui::ease_in_out),
                            |this, delta| {
                                this.with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(delta),
                                ))
                            },
                        ),
                )
                .child(
                    div()
                        .text_color(ui.text)
                        .text_size(px(12.))
                        .overflow_hidden()
                        .child(label),
                )
                .into_any_element();
            sections.push(section);
        }
        if let Some(awaiting) = state.awaiting {
            // Zed uses `SpinnerVariant::Sand` + a pulsating
            // "Awaiting Confirmation" label when the turn is blocked on
            // a permission prompt. Paneflow has no Sand variant — the
            // closest substitute is the existing loader icon in the
            // permission-amber tint with the label pulsating between
            // 0.5 and 1.0 over 2 s, matching Zed's `pulsating_between`.
            // The title suffix (`: foo bar`) is a Paneflow-only addition
            // that gives the user context about which tool is blocked —
            // intentionally preserved beyond strict Zed parity.
            let title: SharedString = format!("Awaiting Confirmation: {}", awaiting.title).into();
            let idx = awaiting.item_idx;
            let amber = rgb(0xeab676);
            let pulsating_label = div()
                .text_size(px(11.))
                .overflow_hidden()
                .child(title)
                .with_animation(
                    "activity-bar-awaiting-pulse",
                    gpui::Animation::new(Duration::from_secs(2))
                        .repeat()
                        .with_easing(gpui::pulsating_between(0.5, 1.0)),
                    |this, delta| this.opacity(delta),
                );
            let section = div()
                .id("activity-bar-awaiting")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .cursor_pointer()
                .hover(|d| d.opacity(0.85))
                .text_color(amber)
                .child(
                    gpui::svg()
                        .size(px(11.))
                        .flex_none()
                        .path("icons/loader-circle.svg")
                        .text_color(amber),
                )
                .child(pulsating_label)
                .on_click(cx.listener(move |this, _ev, _w, cx| {
                    this.scroll_to_item(idx, cx);
                }))
                .into_any_element();
            sections.push(section);
        }
        if state.queued > 0 {
            let label: SharedString = if state.queued == 1 {
                "1 prompt queued".into()
            } else {
                format!("{} prompts queued", state.queued).into()
            };
            let section = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .child(div().text_color(ui.muted).text_size(px(11.)).child(label))
                .child(
                    div()
                        .id("activity-bar-clear-queue")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .w(px(14.))
                        .h(px(14.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .text_color(ui.muted)
                        .hover(|d| {
                            let ui = crate::theme::ui_colors();
                            d.bg(ui.subtle).text_color(ui.text)
                        })
                        .child(
                            gpui::svg()
                                .size(px(9.))
                                .path("icons/generic_close.svg")
                                .text_color(ui.muted),
                        )
                        .on_click(cx.listener(|this, _ev, _w, cx| {
                            this.clear_pending_queue(cx);
                        })),
                )
                .into_any_element();
            sections.push(section);
        }
        if let Some(summary) = state.edits_summary.as_ref() {
            sections.push(self.render_edits_review_section(summary, ui, cx));
        }
        let mut children: Vec<AnyElement> = Vec::with_capacity(sections.len() * 2);
        for (i, section) in sections.into_iter().enumerate() {
            if i > 0 {
                children.push(
                    div()
                        .w(px(1.))
                        .h(px(14.))
                        .bg(ui.border)
                        .mx(px(4.))
                        .into_any_element(),
                );
            }
            children.push(section);
        }
        // US-010 (visual-parity port, 2026-05-24 follow-up): match
        // Zed's `render_activity_bar` shell at
        // `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:2541-2648`.
        // Zed's chain:
        //   h_flex().w_full().px_2().justify_center()
        //     .child(
        //         v_flex().w_full()  // (or max_w basis)
        //             .bg(self.activity_bar_bg(cx))      // editor_bg blended w/ element_selected@0.3
        //             .border_1().border_b_0().border_color(border)
        //             .rounded_t_md()
        //             .shadow(<box shadow>)
        //             .child(<sections>)
        //     )
        //
        // The previous Paneflow render used `bg(title_bar_background)`
        // which is identical to the panel background -- making the bar
        // appear as a detached rounded box floating above the composer
        // instead of a lifted region that visually merges into the
        // composer band below. `ui.subtle` is the Paneflow analog for
        // Zed's lifted activity-bar tone (a touch lighter than
        // `ui.surface` used by the composer band, so the two read as
        // distinct horizontal stripes rather than one flat slab).
        //
        // Wrapped in an outer `px(8) justify_center` band so the bar
        // can later flex to a `max_content_width` basis if/when
        // Paneflow exposes one; today the inner uses `w_full()`.
        // Floating pill above the composer: matches the composer
        // wrapper's horizontal padding (`px(12.)`) so it can't extend
        // past the composer's left/right edges, and adds an 8 px gap
        // below so it reads as a discrete floating element instead of
        // a slab fused to the composer's top edge. Full rounded
        // corners + border on all four sides reinforce that.
        div()
            .flex()
            .flex_row()
            .w_full()
            .px(px(12.))
            .pb(px(8.))
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_h(px(28.))
                    .rounded_md()
                    .border_1()
                    .border_color(ui.border)
                    .bg(ui.subtle)
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::black().opacity(0.18),
                        offset: gpui::point(px(0.), px(1.)),
                        blur_radius: px(4.),
                        spread_radius: px(0.),
                    }])
                    .children(children),
            )
            .into_any_element()
    }

    /// Edits-review cluster of the activity bar (Zed parity port of
    /// `render_edits_summary`). Renders "Edits · N files · +X -Y" on
    /// the left plus a Reject All / Keep All button pair on the right.
    /// Clicks dispatch the same KeepEdits / RejectEdits actions that
    /// Shift+Alt+Y / Shift+Alt+Z fire, so the chord hint next to each
    /// button is a discoverability cue, not a separate code path.
    fn render_edits_review_section(
        &self,
        summary: &EditsReviewSummary,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files = summary.files;
        let added = summary.added;
        let removed = summary.removed;
        let files_label: SharedString = if files == 1 {
            "1 file".into()
        } else {
            format!("{files} files").into()
        };

        let dot = || {
            div()
                .text_size(px(11.))
                .text_color(ui.muted)
                .child("·")
                .into_any_element()
        };

        let summary_cluster = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(div().text_color(ui.muted).text_size(px(11.)).child("Edits"))
            .child(dot())
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(11.))
                    .child(files_label),
            )
            .when(added > 0 || removed > 0, |d| d.child(dot()))
            .when(added > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(super::edit_tool_block::added_fg())
                        .child(format!("+{added}")),
                )
            })
            .when(removed > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(super::edit_tool_block::removed_fg())
                        .child(format!("-{removed}")),
                )
            });

        let reject_btn = self.render_edits_review_button(
            "activity-bar-reject-all",
            "Reject All",
            "Shift+Alt+Z",
            ui,
            cx.listener(|this, _ev, w, cx| {
                this.reject_all_edits(&crate::RejectEdits, w, cx);
            }),
        );
        let keep_btn = self.render_edits_review_button(
            "activity-bar-keep-all",
            "Keep All",
            "Shift+Alt+Y",
            ui,
            cx.listener(|this, _ev, w, cx| {
                this.keep_all_edits(&crate::KeepEdits, w, cx);
            }),
        );

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.))
            .child(summary_cluster)
            .child(reject_btn)
            .child(keep_btn)
            .into_any_element()
    }

    fn render_edits_review_button(
        &self,
        id: &'static str,
        label: &'static str,
        chord: &'static str,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        div()
            .id(SharedString::from(id))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(5.))
            .cursor_pointer()
            // Activity bar background is already `ui.subtle`, so hovering
            // with the same colour produces no visible change. Lift one
            // step lighter via `ui.border` to mirror the composer send
            // button's idle-hover pattern (transparent -> ui.subtle on
            // a ui.surface composer band).
            .hover(|d| {
                let ui = crate::theme::ui_colors();
                d.bg(ui.border)
            })
            .on_click(on_click)
            .child(div().text_size(px(11.)).text_color(ui.text).child(label))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .font_family("Lilex")
                    .child(chord),
            )
            .into_any_element()
    }

    fn compute_activity_state(&self, cx: &gpui::App) -> Option<ActivityBarState> {
        let mut awaiting: Option<AwaitingTool> = None;
        let mut running_kind: Option<String> = None;
        for (idx, item) in self.items.iter().enumerate() {
            let ThreadItem::ToolCall(snap) = item else {
                continue;
            };
            match snap.status {
                super::runtime::ToolCallStatusKind::WaitingForConfirmation => {
                    awaiting = Some(AwaitingTool {
                        item_idx: idx,
                        title: snap.title.clone(),
                    });
                }
                super::runtime::ToolCallStatusKind::InProgress
                | super::runtime::ToolCallStatusKind::Pending => {
                    running_kind = Some(verbose_tool_kind_label(snap.kind).to_string());
                }
                _ => {}
            }
        }
        let (is_streaming, queued, elapsed, used_tokens) = match self.composer.as_ref() {
            Some(c) => {
                let c = c.read(cx);
                let elapsed = c
                    .turn_started_at()
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let used = c.last_usage().map(|(used, _)| used);
                (c.is_streaming(), c.pending_prompts_len(), elapsed, used)
            }
            None => (false, 0, 0, None),
        };
        if running_kind.is_none() && is_streaming && awaiting.is_none() {
            running_kind = Some("Working".to_string());
        }
        let running = running_kind.map(|kind_label| RunningTool {
            kind_label,
            // US-121 AC #1: gate at 3s -- below that the elapsed suffix
            // is not painted at all (avoids first-frame flicker when a
            // turn finishes well under threshold).
            elapsed_label: if is_streaming && elapsed >= ELAPSED_THRESHOLD_SECS {
                Some(format_elapsed(elapsed))
            } else {
                None
            },
            // US-121 AC #2: gate at 1000 tokens -- below that the token
            // suffix is omitted. Today no shipping ACP wrapper emits
            // UsageUpdate, so `used_tokens` is `None` and this stays
            // omitted; the moment a wrapper opts in, the suffix lights
            // up automatically.
            tokens_label: used_tokens.and_then(|used| {
                if used >= TOKEN_SUFFIX_THRESHOLD {
                    Some(format!("{} tokens", humanize_token_count(used)))
                } else {
                    None
                }
            }),
        });
        let edits_summary = self.compute_edits_review_summary();
        if running.is_none() && awaiting.is_none() && queued == 0 && edits_summary.is_none() {
            return None;
        }
        Some(ActivityBarState {
            running,
            awaiting,
            queued,
            edits_summary,
        })
    }

    /// Walk the thread for completed tool calls that still carry
    /// unreviewed diffs and aggregate the file count + `+added` /
    /// `-removed` line counts. Returns `None` when nothing is pending
    /// review so the activity bar doesn't render an empty section.
    fn compute_edits_review_summary(&self) -> Option<EditsReviewSummary> {
        let mut files = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        for item in &self.items {
            let ThreadItem::ToolCall(snap) = item else {
                continue;
            };
            if !matches!(snap.status, super::runtime::ToolCallStatusKind::Completed) {
                continue;
            }
            if snap.diffs.is_empty() {
                continue;
            }
            if self.reviewed_edits.contains(&snap.id) {
                continue;
            }
            files += snap.diffs.len();
            let (a, r) = super::edit_tool_block::diff_stats(&snap.diffs);
            added += a;
            removed += r;
        }
        if files == 0 {
            None
        } else {
            Some(EditsReviewSummary {
                files,
                added,
                removed,
            })
        }
    }

    pub fn scroll_to_item(&mut self, item_idx: usize, _cx: &mut Context<Self>) {
        if item_idx < self.items.len() {
            self.list_state.scroll_to_reveal_item(item_idx);
            self.should_be_following.set(false);
        }
    }

    pub fn clear_pending_queue(&mut self, cx: &mut Context<Self>) {
        if let Some(composer) = self.composer.clone() {
            composer.update(cx, |c, cx| c.clear_pending_prompts(cx));
            cx.notify();
        }
    }

    pub fn resolve_permission(
        &mut self,
        tool_call_id: String,
        decision: paneflow_acp::PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        // US-124 AC #3: a Reject on a tool call that previously
        // matched an `always_allow` substring pattern auto-promotes
        // that pattern to `always_deny`. Look up the snapshot first
        // so we have the kind + raw input for the match; the picker
        // (if open) is closed below by `clear_pending_permission`.
        if matches!(decision, paneflow_acp::PermissionDecision::Reject) {
            let snap_info = self.items.iter().find_map(|item| match item {
                ThreadItem::ToolCall(snap) if snap.id == tool_call_id => {
                    Some((snap.kind, snap.raw_input_pretty.clone().unwrap_or_default()))
                }
                _ => None,
            });
            if let Some((kind, raw)) = snap_info
                && let Some(promoted) =
                    super::panel_config::auto_promote_to_deny(kind, raw.as_str())
            {
                log::info!(
                    "tool_permissions: auto-promoted `{promoted}` from always_allow to always_deny (US-124 AC #3)"
                );
            }
        }
        // US-124 AC #6: any open pattern picker should dismiss with
        // the resolve, regardless of which button drove it.
        self.close_permission_pickers(cx);
        self.clear_pending_permission(&tool_call_id, decision, cx);
        if let Some(composer) = self.composer.clone() {
            composer.update(cx, |c, _| {
                c.send_permission_decision(tool_call_id, decision)
            });
        }
    }

    /// US-124: commit an "Allow Always" decision with an optional
    /// substring pattern. `None` (or empty) writes the bare any-input
    /// rule the existing v1 flow used; `Some(pattern)` appends a
    /// specific pattern so only future calls whose raw input contains
    /// it auto-allow. Persists synchronously via
    /// [`panel_config::record_tool_permission_with_pattern`] and then
    /// resolves the permission as `AllowAlways`.
    pub fn resolve_permission_with_pattern(
        &mut self,
        tool_call_id: String,
        pattern: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let kind = self.items.iter().find_map(|item| match item {
            ThreadItem::ToolCall(snap) if snap.id == tool_call_id => Some(snap.kind),
            _ => None,
        });
        if let Some(kind) = kind {
            super::panel_config::record_tool_permission_with_pattern(kind, pattern);
        }
        self.resolve_permission(
            tool_call_id,
            paneflow_acp::PermissionDecision::AllowAlways,
            cx,
        );
    }

    pub fn send_user_message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.engage_follow();
        self.append_message(Message::user_text(text), cx);
    }

    pub fn send_user_message_blocks(&mut self, content: Vec<ContentBlock>, cx: &mut Context<Self>) {
        self.engage_follow();
        self.append_message(
            Message::new(paneflow_threads::MessageRole::User, content),
            cx,
        );
    }

    // -----------------------------------------------------------------
    // US-020: edit prior message + fork machinery
    // -----------------------------------------------------------------

    pub fn start_edit(&mut self, message_idx: usize, w: &mut Window, cx: &mut Context<Self>) {
        if self.is_streaming() {
            return;
        }
        let Some(ThreadItem::UserMessage(um)) = self.items.get(message_idx) else {
            return;
        };
        if um.msg.role != MessageRole::User {
            return;
        }
        let original_text = join_text_blocks(&um.msg.content);
        let initial = original_text.clone();
        let text_area = cx.new(|cx| {
            let mut ta = crate::widgets::text_area::TextArea::new("Edit your message", cx);
            ta.set_value(initial, cx);
            ta
        });
        // Focus the TextArea immediately so the user can type / select
        // without an extra click — matches the composer's "focused on
        // mount" behavior.
        let focus_handle = text_area.read(cx).focus_handle_ref().clone();
        focus_handle.focus(w, cx);
        self.editing = Some(EditState {
            message_idx,
            text_area,
            original_text,
        });
        cx.notify();
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            cx.notify();
        }
    }

    pub fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.editing.as_ref() else {
            return;
        };
        let new_text = state.text_area.read(cx).value();
        if new_text.trim().is_empty() {
            return;
        }
        let payload = ForkRequested {
            message_idx: state.message_idx,
            new_text,
        };
        self.editing = None;
        cx.notify();
        cx.emit(payload);
    }

    pub fn build_fork_messages(&self, message_idx: usize, new_text: &str) -> Option<Vec<Message>> {
        build_fork_messages_from_items(&self.items, message_idx, new_text)
    }

    /// Drop every item from `message_idx` onwards (the edited user
    /// message and everything the agent generated after it), reset the
    /// list state to the remaining length, and persist the truncated
    /// snapshot to `threads.db`. The caller is expected to immediately
    /// dispatch the new user message via the composer, which will
    /// append it as a fresh item and start a new turn.
    pub fn truncate_for_edit(&mut self, message_idx: usize, cx: &mut Context<Self>) {
        if message_idx >= self.items.len() {
            return;
        }
        self.flush_streaming(cx);
        self.items.truncate(message_idx);
        self.streaming_message_idx = None;
        // Drop collapsed-thought state for items past the truncation
        // point so a future render doesn't index into removed slots.
        // The tool-label markdown cache is keyed by tool call id (not
        // by index), so it can stay — orphaned entries are harmless.
        let new_len = self.items.len();
        self.collapsed_thoughts
            .retain(|(msg_idx, _)| *msg_idx < new_len);
        self.list_state.reset(new_len);
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    pub fn is_editing(&self, message_idx: usize) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|s| s.message_idx == message_idx)
    }

    pub fn composer(&self) -> Option<Entity<Composer>> {
        self.composer.clone()
    }

    /// True when the timeline has no items at all — used by the
    /// composer to detect "this prompt is the very first one in the
    /// thread" and trigger the client-side title auto-derive.
    pub fn items_is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Text of the first user message in the timeline, or `None` when
    /// no user message exists yet. Used by the background title
    /// summarizer ([`crate::agents::title_summarizer`]) to build the
    /// conversation context it ships to the transient agent session.
    pub fn first_user_text(&self) -> Option<String> {
        self.items.iter().find_map(|it| match it {
            ThreadItem::UserMessage(um) if um.msg.role == MessageRole::User => {
                let text = join_text_blocks(&um.msg.content);
                if text.is_empty() { None } else { Some(text) }
            }
            _ => None,
        })
    }

    /// Concatenated text of the first assistant turn in the timeline,
    /// or `None` when the assistant hasn't responded yet. Thought
    /// chunks are skipped (the summarizer should see the final answer,
    /// not the reasoning trace). Capped to a reasonable length so the
    /// transient session prompt stays small.
    pub fn first_assistant_text(&self) -> Option<String> {
        const MAX_ASSISTANT_TEXT: usize = 2000;
        let am = self.items.iter().find_map(|it| match it {
            ThreadItem::AssistantMessage(am) => Some(am),
            _ => None,
        })?;
        let mut out = String::new();
        for chunk in &am.chunks {
            if let AssistantMessageChunk::Text { text, .. } = chunk {
                out.push_str(text);
                if out.len() >= MAX_ASSISTANT_TEXT {
                    break;
                }
            }
        }
        if out.is_empty() {
            None
        } else {
            if out.len() > MAX_ASSISTANT_TEXT {
                out.truncate(MAX_ASSISTANT_TEXT);
                out.push_str("...");
            }
            Some(out)
        }
    }

    pub fn clear_local_display(&mut self, cx: &mut Context<Self>) {
        self.flush_streaming(cx);
        self.items.clear();
        self.collapsed_thoughts.clear();
        self.tool_label_markdown.clear();
        self.streaming_message_idx = None;
        self.list_state.reset(0);
        cx.notify();
    }

    /// Persist the full timeline snapshot to `threads.db`.
    pub fn persist_snapshot_now(&self, cx: &gpui::App) {
        let (Some(store), Some(id)) = (self.store.as_ref(), self.store_id.as_ref()) else {
            return;
        };
        let items = self.collect_persisted_items(cx);
        if let Err(err) = store.save_items(id, &items) {
            log::warn!("ThreadView: persist_snapshot_now failed: {err}");
        }
    }

    fn collect_persisted_items(&self, _cx: &gpui::App) -> Vec<PersistedThreadItem> {
        self.items
            .iter()
            .filter_map(|it| match it {
                ThreadItem::UserMessage(um) => Some(PersistedThreadItem::Message(um.msg.clone())),
                ThreadItem::AssistantMessage(am) => {
                    if am.chunks.is_empty() {
                        None
                    } else {
                        let chunks = am
                            .chunks
                            .iter()
                            .map(|c| match c {
                                AssistantMessageChunk::Text { text, .. } => {
                                    PersistedAssistantChunk::Text { text: text.clone() }
                                }
                                AssistantMessageChunk::Thought {
                                    text, signature, ..
                                } => PersistedAssistantChunk::Thought {
                                    text: text.clone(),
                                    signature: signature.clone(),
                                },
                            })
                            .collect();
                        Some(PersistedThreadItem::Assistant(PersistedAssistant {
                            chunks,
                        }))
                    }
                }
                ThreadItem::ToolCall(snap) => {
                    let mut persisted = snap.to_persisted();
                    // Carry the view-side review state into the blob
                    // so the activity-bar footer stays dismissed
                    // across reloads. `to_persisted` defaults it to
                    // false because the snapshot itself has no field
                    // for it -- review state lives on the view.
                    persisted.reviewed = self.reviewed_edits.contains(&snap.id);
                    Some(PersistedThreadItem::Tool(persisted))
                }
            })
            .collect()
    }

    /// US-019 (`/export` slash command): serialise the current
    /// timeline as Markdown.
    pub fn export_markdown(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            match item {
                ThreadItem::UserMessage(um) => {
                    let header = match um.msg.role {
                        MessageRole::User => "## User",
                        MessageRole::System => "## System",
                        // unreachable: assistant role would land in
                        // AssistantMessage instead.
                        MessageRole::Assistant => "## Assistant",
                    };
                    out.push_str(header);
                    out.push_str("\n\n");
                    out.push_str(&join_text_blocks(&um.msg.content));
                    out.push_str("\n\n");
                }
                ThreadItem::AssistantMessage(am) => {
                    out.push_str("## Assistant\n\n");
                    for chunk in &am.chunks {
                        match chunk {
                            AssistantMessageChunk::Text { text, .. } => {
                                out.push_str(text);
                                out.push_str("\n\n");
                            }
                            AssistantMessageChunk::Thought { text, .. } => {
                                out.push_str("<thinking>\n");
                                out.push_str(text);
                                out.push_str("\n</thinking>\n\n");
                            }
                        }
                    }
                }
                ThreadItem::ToolCall(snap) => {
                    out.push_str("### Tool: ");
                    out.push_str(&snap.title);
                    out.push_str("\n\n");
                    if !snap.content_text.is_empty() {
                        out.push_str("```\n");
                        out.push_str(&snap.content_text);
                        out.push_str("\n```\n\n");
                    }
                }
            }
        }
        out
    }

    pub fn engage_follow(&self) {
        self.should_be_following.set(true);
    }

    /// US-015: open an assistant message and start the paced reveal
    /// loop. If a previous turn is still streaming, it is flushed
    /// first.
    pub fn begin_assistant_stream(&mut self, cx: &mut Context<Self>) {
        if self.streaming_message_idx.is_some() {
            self.flush_streaming(cx);
        }
        // Re-use the trailing assistant message if there is one
        // (e.g. a Thought chunk landed first and we want the text to
        // append into the same turn). Otherwise open a fresh one.
        let idx = self.ensure_open_assistant_message(cx);
        if let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(idx) {
            // Only add an empty Text seed if the trailing chunk
            // isn't already a Text chunk that the paced reveal can
            // append into.
            let needs_seed = !matches!(am.chunks.last(), Some(AssistantMessageChunk::Text { .. }));
            if needs_seed {
                let markdown = Self::make_markdown("", cx);
                am.chunks.push(AssistantMessageChunk::Text {
                    text: String::new(),
                    markdown,
                });
            }
        }
        self.streaming_message_idx = Some(idx);
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        cx.notify();

        self._streaming_task = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                loop {
                    smol::Timer::after(STREAMING_TICK).await;
                    let outcome = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            if view.streaming_message_idx.is_none() {
                                return false;
                            }
                            let revealed = view.streaming_buffer.tick();
                            if !revealed.is_empty() {
                                view.append_to_streaming_text(&revealed, cx);
                            }
                            true
                        })
                    });
                    match outcome {
                        Ok(true) => continue,
                        _ => break,
                    }
                }
            },
        ));
    }

    pub fn push_streaming_chunk(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.streaming_buffer.push(chunk);
    }

    pub fn flush_streaming(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.streaming_message_idx.take() else {
            return;
        };
        let remaining = self.streaming_buffer.flush();
        if !remaining.is_empty() {
            self.append_to_streaming_text_at(idx, &remaining, cx);
        }
        self.persist_snapshot_now(cx);
        self._streaming_task = None;
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming_message_idx.is_some()
    }

    /// Append text to the in-progress assistant message's trailing
    /// `Text` chunk. If the trailing chunk is a `Thought` (a thinking
    /// burst landed mid-stream), open a new `Text` chunk after it.
    fn append_to_streaming_text(&mut self, chunk: &str, cx: &mut Context<Self>) {
        let Some(idx) = self.streaming_message_idx else {
            return;
        };
        self.append_to_streaming_text_at(idx, chunk, cx);
    }

    fn append_to_streaming_text_at(&mut self, idx: usize, chunk: &str, cx: &mut Context<Self>) {
        let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(idx) else {
            return;
        };
        match am.chunks.last_mut() {
            Some(AssistantMessageChunk::Text { text, markdown }) => {
                text.push_str(chunk);
                markdown.update(cx, |m, cx| m.append(chunk, cx));
            }
            _ => {
                let md = Self::make_markdown(chunk, cx);
                am.chunks.push(AssistantMessageChunk::Text {
                    text: chunk.to_string(),
                    markdown: md,
                });
            }
        }
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        cx.notify();
    }

    pub fn is_following(&self) -> bool {
        self.should_be_following.get()
    }

    pub fn message_count(&self) -> usize {
        self.items.len()
    }

    // -----------------------------------------------------------------
    // Assistant-message bookkeeping
    // -----------------------------------------------------------------

    /// Return the index of the currently-open assistant message,
    /// creating a new one when the last item is not an open
    /// assistant message.
    /// Return the index of the assistant message at the back of the
    /// timeline, creating a new (empty) one when the last item is a
    /// user message or a tool call. The "open" assistant message is
    /// the one new chunks (Text + Thought) append into.
    fn ensure_open_assistant_message(&mut self, cx: &mut Context<Self>) -> usize {
        match self.items.last() {
            Some(ThreadItem::AssistantMessage(_)) => self.items.len() - 1,
            _ => {
                let prev_count = self.items.len();
                self.items
                    .push(ThreadItem::AssistantMessage(AssistantMessage {
                        chunks: Vec::new(),
                    }));
                self.list_state.splice(prev_count..prev_count, 1);
                if self.should_be_following.get() {
                    self.list_state.scroll_to_end();
                }
                cx.notify();
                prev_count
            }
        }
    }

    /// Stop appending to the trailing assistant message. The message
    /// itself stays in the timeline (Zed semantics: a tool call or
    /// user message naturally interrupts the assistant turn, but the
    /// chunks already emitted remain visible).
    fn close_open_assistant_message(&mut self) {
        self.streaming_message_idx = None;
    }

    // -----------------------------------------------------------------
    // Render
    // -----------------------------------------------------------------

    fn render_item(&mut self, ix: usize, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        // Virtual extra item: when the composer is streaming, the
        // list reserves a slot at index `items.len()` for the inline
        // pixel spinner. Mirrors Zed's `render_entries` branch at
        // `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:4848`
        // (`generating_indicator_in_list`).
        if ix == self.items.len() && self.generating_indicator_active {
            return self.render_generating_spinner(ui, cx);
        }
        let Some(item) = self.items.get(ix) else {
            return div().into_any_element();
        };
        match item {
            ThreadItem::UserMessage(um) => {
                if self.is_editing(ix)
                    && let Some(state) = self.editing.as_ref()
                {
                    return render_user_edit_row(state, ui, cx);
                }
                let role = um.msg.role;
                let md_entity = um.markdown.clone();
                let bubble = super::message_render::render_message_body(
                    role,
                    &um.msg.content,
                    md_entity,
                    ui,
                );
                if role == MessageRole::User {
                    let is_streaming = self.is_streaming();
                    render_user_message_with_edit_affordance(ix, bubble, is_streaming, ui, cx)
                } else {
                    bubble
                }
            }
            ThreadItem::AssistantMessage(am) => {
                let is_last = ix + 1 == self.items.len();
                self.render_assistant_message(ix, am, is_last, ui, w, cx)
            }
            ThreadItem::ToolCall(snap) => {
                let snap = snap.clone();
                if snap.kind == ToolKindKind::Edit
                    || !snap.diffs.is_empty()
                    || snap.kind == ToolKindKind::Delete
                    || snap.kind == ToolKindKind::Move
                {
                    // Card layout for diff-bearing tools. Keep All /
                    // Reject All buttons live in the activity bar
                    // (above the composer) so the cards stay focused
                    // on the file + diff preview.
                    let id_for_toggle = snap.id.clone();
                    let on_toggle = cx.listener(move |this, _ev, _w, cx| {
                        this.toggle_tool_call_expanded(&id_for_toggle, cx);
                    });
                    let scroll_handle = self
                        .diff_scroll_handles
                        .entry(snap.id.clone())
                        .or_default()
                        .clone();
                    super::edit_tool_block::render_edit_tool_block(
                        snap,
                        ui,
                        on_toggle,
                        scroll_handle,
                    )
                } else {
                    // Inline single-line row for Read / Search / Exec / Fetch / Think / Other.
                    let id_for_toggle = snap.id.clone();
                    let id_for_perm = snap.id.clone();
                    let label_md = self.tool_label_markdown.get(&snap.id).cloned();
                    let entity_weak_toggle = cx.entity().downgrade();
                    let on_toggle =
                        move |_ev: &ClickEvent, _w: &mut gpui::Window, cx: &mut gpui::App| {
                            if let Some(entity) = entity_weak_toggle.upgrade() {
                                let id = id_for_toggle.clone();
                                entity.update(cx, |this, cx| {
                                    this.toggle_tool_call_expanded(&id, cx);
                                });
                            }
                        };
                    let entity_weak = cx.entity().downgrade();
                    let id_for_picker = snap.id.clone();
                    let id_for_pattern = snap.id.clone();
                    let on_permission = {
                        let entity_weak = entity_weak.clone();
                        let id_for_perm = id_for_perm.clone();
                        move |d: paneflow_acp::PermissionDecision,
                              _w: &mut gpui::Window,
                              cx: &mut gpui::App| {
                            if let Some(entity) = entity_weak.upgrade() {
                                let id = id_for_perm.clone();
                                entity.update(cx, |this, cx| {
                                    this.resolve_permission(id, d, cx);
                                });
                            }
                        }
                    };
                    // US-124: picker toggle + apply-pattern callbacks
                    // bridge ToolCallSnapshot view-state and the
                    // persistence layer through ThreadView's helpers.
                    let on_toggle_picker = {
                        let entity_weak = entity_weak.clone();
                        move |_w: &mut gpui::Window, cx: &mut gpui::App| {
                            if let Some(entity) = entity_weak.upgrade() {
                                let id = id_for_picker.clone();
                                entity.update(cx, |this, cx| {
                                    this.toggle_permission_picker(&id, cx);
                                });
                            }
                        }
                    };
                    let on_apply_pattern = {
                        let entity_weak = entity_weak.clone();
                        move |pattern: Option<String>, _w: &mut gpui::Window, cx: &mut gpui::App| {
                            if let Some(entity) = entity_weak.upgrade() {
                                let id = id_for_pattern.clone();
                                entity.update(cx, |this, cx| {
                                    this.resolve_permission_with_pattern(id, pattern, cx);
                                });
                            }
                        }
                    };
                    super::inline_tool_call::render_inline_tool_call(
                        ix,
                        snap,
                        label_md,
                        ui,
                        w,
                        cx,
                        on_toggle,
                        on_permission,
                        on_toggle_picker,
                        on_apply_pattern,
                    )
                }
            }
        }
    }

    fn render_assistant_message(
        &self,
        entry_ix: usize,
        am: &AssistantMessage,
        is_last: bool,
        ui: crate::theme::UiColors,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Port of Zed `thread_view.rs:5057-5117` AssistantMessage match
        // arm (US-008 in `prd-agent-ui-visual-parity-2026-Q3.md`):
        //   v_flex().w_full().gap_3() chunks container
        //   outer v_flex().px_5().py_1p5().when(is_last, |t| t.pb_4())
        //   .text_ui(cx) — substituted with our 14px body size to mirror
        //   `MarkdownFont::Agent`.
        // Blank-only chunks are skipped (`is_blank` guard) to avoid
        // emitting empty outer padding while a turn warms up.
        let is_blank = am.chunks.iter().all(|c| match c {
            AssistantMessageChunk::Text { text, .. }
            | AssistantMessageChunk::Thought { text, .. } => text.trim().is_empty(),
        });
        if is_blank {
            return div().into_any_element();
        }
        // Inter-chunk gap: Zed uses `gap_3` (12px) at
        // `crates/agent_ui/src/conversation_view/thread_view.rs:5066-5068`
        // between Text/Thought chunks of one turn -- but the markdown
        // widget's auto-`mb_2()` paragraph margin
        // (`crates/markdown/src/markdown.rs:1164-1166`) compounds with
        // it inside Zed's longer responses, giving the airier feel.
        // Paneflow's chunks are usually one paragraph each, so the
        // mb_2 compounding rarely fires and the result reads as
        // tighter than Zed at the same gap value. Bumping to 20px
        // matches the visual breathing room of a multi-paragraph Zed
        // turn while keeping single-chunk turns from feeling sparse.
        let mut body = div().flex().flex_col().w_full().gap(px(20.));
        for (chunk_ix, chunk) in am.chunks.iter().enumerate() {
            match chunk {
                AssistantMessageChunk::Text { text, markdown } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    body = body.child(super::message_render::render_assistant_body_md(
                        markdown.clone(),
                        ui,
                    ));
                }
                AssistantMessageChunk::Thought { text, markdown, .. } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    let key = (entry_ix, chunk_ix);
                    // Default to expanded; the set tracks user-collapsed
                    // keys (see `toggle_thinking_expanded` docstring).
                    let expanded = !self.collapsed_thoughts.contains(&key);
                    let entity_weak = cx.entity().downgrade();
                    let on_toggle =
                        move |_ev: &ClickEvent, _w: &mut gpui::Window, cx: &mut gpui::App| {
                            if let Some(entity) = entity_weak.upgrade() {
                                entity.update(cx, |this, cx| {
                                    this.toggle_thinking_expanded(key, cx);
                                });
                            }
                        };
                    body = body.child(super::inline_thinking::render_inline_thinking(
                        key,
                        markdown.clone(),
                        expanded,
                        // No `ThinkingBlockDisplay::Preview` mode wired
                        // yet (Paneflow only tracks a single expanded
                        // bit); pass false so the constrained max_h +
                        // gradient overlay is no-op until a future story
                        // adds the display-mode state.
                        false,
                        ui,
                        w,
                        cx,
                        on_toggle,
                    ));
                }
            }
        }
        // Outer wrapper: Zed `py_1p5()` (6px) +
        // `pb_4()` on the last turn at
        // `crates/agent_ui/src/conversation_view/thread_view.rs:5113-5114`.
        // Bumped to `py(10)` here so the turn boundary reads as a
        // distinct vertical region in Paneflow's narrower panel
        // widths. Last-turn bottom padding bumped to 24px so the
        // composer doesn't crowd the final paragraph.
        let mut outer = div()
            .flex()
            .flex_col()
            .w_full()
            .px(px(20.))
            .py(px(10.))
            .text_size(px(14.));
        if is_last {
            outer = outer.pb(px(24.));
        }
        outer.child(body).into_any_element()
    }
}

#[derive(Debug, Clone)]
struct ActivityBarState {
    running: Option<RunningTool>,
    awaiting: Option<AwaitingTool>,
    queued: usize,
    /// Zed-parity port of `render_edits_summary`: aggregate of every
    /// completed but unreviewed edit tool call in the thread. `None`
    /// when there is nothing to review (no diffs, or every diff has
    /// already been kept / rejected). When `Some`, the activity bar
    /// renders the "Edits · N files · +X −Y" cluster + Reject All /
    /// Keep All buttons so the user can resolve every pending edit
    /// without scrolling back into the thread.
    edits_summary: Option<EditsReviewSummary>,
}

#[derive(Debug, Clone)]
struct EditsReviewSummary {
    files: usize,
    added: usize,
    removed: usize,
}

#[derive(Debug, Clone)]
struct RunningTool {
    kind_label: String,
    /// US-121 AC #1: `Some("3s")` / `Some("12s")` / `Some("1m02")` once
    /// the elapsed time crosses the 3-second threshold; `None` below it
    /// so the suffix is not painted at all.
    elapsed_label: Option<String>,
    /// US-121 AC #2: `Some("1.2k tokens")` once the cumulative count
    /// crosses 1000; `None` below it. Reads the latest
    /// `Composer::last_usage` snapshot.
    tokens_label: Option<String>,
}

#[derive(Debug, Clone)]
struct AwaitingTool {
    item_idx: usize,
    title: String,
}

fn verbose_tool_kind_label(kind: super::runtime::ToolKindKind) -> &'static str {
    use super::runtime::ToolKindKind;
    match kind {
        ToolKindKind::Read => "Reading file",
        ToolKindKind::Edit => "Editing file",
        ToolKindKind::Delete => "Deleting file",
        ToolKindKind::Move => "Moving file",
        ToolKindKind::Search => "Searching",
        ToolKindKind::Execute => "Running command",
        ToolKindKind::Think => "Thinking",
        ToolKindKind::Fetch => "Fetching",
        ToolKindKind::SwitchMode => "Switching mode",
        ToolKindKind::Other => "Working",
    }
}

fn apply_patch(snap: &mut ToolCallSnapshot, patch: ToolCallUpdate) {
    let mut needs_reenrich = false;
    if let Some(title) = patch.title {
        snap.title = title;
        needs_reenrich = true;
    }
    if let Some(kind) = patch.kind {
        snap.kind = kind;
    }
    if let Some(status) = patch.status {
        snap.status = status;
    }
    if let Some(raw_in) = patch.raw_input_pretty {
        snap.raw_input_pretty = Some(raw_in);
    }
    if let Some(raw_out) = patch.raw_output_pretty {
        snap.raw_output_pretty = Some(raw_out);
    }
    if let Some(text) = patch.content_text {
        snap.content_text = text;
    }
    if let Some(diffs) = patch.diffs {
        // The default `expanded` value is set once in
        // `snapshot_from_tool_call` (true for Edit-kind / diff-bearing
        // tool calls). After that, never touch it from a patch: the
        // user's chevron click is the source of truth, and re-opening
        // the card on every late ACP update would make the collapse
        // chord feel broken.
        snap.diffs = diffs;
    }
    if let Some(locations) = patch.locations {
        snap.locations = locations;
        needs_reenrich = true;
    }
    // Re-run title enrichment when either field changed: the agent
    // sometimes sends a bare "Read File" title in the initial
    // notification then a `locations` update in a follow-up patch,
    // and we want the path chip to appear once both are known.
    // Idempotent -- the helper bails out when the title already
    // contains the basename.
    if needs_reenrich {
        snap.title = super::runtime::enrich_title_with_location(
            std::mem::take(&mut snap.title),
            snap.kind,
            &snap.locations,
        );
    }
}

impl Render for ThreadView {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let list_body: AnyElement = if self.items.is_empty() {
            empty_state()
        } else {
            list(
                self.list_state.clone(),
                cx.processor(
                    |this: &mut Self, ix: usize, w: &mut Window, cx: &mut Context<Self>| {
                        this.render_item(ix, w, cx)
                    },
                ),
            )
            .with_sizing_behavior(ListSizingBehavior::Auto)
            .size_full()
            .into_any_element()
        };

        let composer_el: AnyElement = match self.composer.clone() {
            Some(c) => c.into_any_element(),
            None => div().into_any_element(),
        };

        // Keep the list_state's virtual-item count in sync with
        // `generating_indicator_active`. Each render checks whether
        // the composer is streaming; if the bool differs from the
        // sticky one we last spliced for, splice the delta and flip
        // the flag. The spinner is rendered at index ==
        // self.items.len() inside `render_item` (see the matching
        // `if ix == self.items.len() && self.generating_indicator_active`
        // branch). Mirrors Zed `render_entries` at
        // `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:4848`
        // (`generating_indicator_in_list`).
        let composer_streaming = self
            .composer
            .as_ref()
            .map(|c| c.read(cx).is_streaming())
            .unwrap_or(false);
        if composer_streaming != self.generating_indicator_active {
            let len = self.items.len();
            if composer_streaming {
                // turning ON: reserve one extra virtual slot at the tail
                self.list_state.splice(len..len, 1);
            } else {
                // turning OFF: drop the virtual slot
                self.list_state.splice(len..len + 1, 0);
            }
            self.generating_indicator_active = composer_streaming;
            if self.should_be_following.get() {
                self.list_state.scroll_to_end();
            }
        }

        let activity_bar: Option<AnyElement> = self
            .compute_activity_state(cx)
            .map(|state| self.render_activity_bar(state, cx));

        let theme = crate::theme::active_theme();
        let max_w_px = crate::agents::panel_config::active_max_content_width() as f32;
        let mut inner = div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(max_w_px))
            .mx_auto()
            .child(div().flex_1().min_h(px(0.)).child(list_body));
        if let Some(bar) = activity_bar {
            inner = inner.child(bar);
        }
        let inner = inner.child(composer_el);
        let _ = ui;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.title_bar_background)
            .on_action(cx.listener(Self::keep_all_edits))
            .on_action(cx.listener(Self::reject_all_edits))
            .child(inner)
            .into_any_element()
    }
}

/// Revert one [`DiffSnapshot`] on disk. Best-effort: errors are
/// logged at `warn` and swallowed so a single failing path doesn't
/// abort a multi-file reject. Behaviour mirrors Zed's
/// `reject_edits_in_ranges`:
/// - `old_text == Some(content)` with non-empty content -> the file
///   existed before the edit; restore it via `fs::write`.
/// - `old_text == None` OR `Some("")` -> the file was newly created by
///   the agent; remove it via `fs::remove_file` (treats "didn't exist
///   then" as the ground truth even if the agent wrote an empty
///   payload).
fn revert_one_diff(diff: &super::runtime::DiffSnapshot) {
    use std::fs;
    let path = &diff.path;
    match diff.old_text.as_deref() {
        Some(prev) if !prev.is_empty() => {
            if let Err(err) = fs::write(path, prev) {
                log::warn!(
                    "agents::thread_view: reject_all_edits failed to restore {}: {err}",
                    path.display()
                );
            }
        }
        _ => {
            if let Err(err) = fs::remove_file(path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!(
                    "agents::thread_view: reject_all_edits failed to remove {}: {err}",
                    path.display()
                );
            }
        }
    }
}

const MAX_CHAT_WIDTH: f32 = 760.0;

/// US-121 AC #1: Paneflow divergence from Zed's `STOPWATCH_THRESHOLD =
/// 30s`. A 3-second gate is more reactive: a turn that lingers past 3s
/// already feels "long" on a developer-grade laptop, and seeing the
/// elapsed counter appear gives the user a stopwatch they can use to
/// judge whether to cancel.
const ELAPSED_THRESHOLD_SECS: u64 = 3;

/// US-121 AC #2: token-throughput suffix appears once cumulative tokens
/// cross this gate. Matches Zed's behavior (anything below feels like
/// noise compared to the elapsed time).
const TOKEN_SUFFIX_THRESHOLD: u64 = 1000;

/// US-120 / US-121 helper: format a token count as a compact label.
/// - `< 1_000`: `"123"` (rare path -- call sites gate above threshold,
///   but the function is total).
/// - `< 10_000`: `"1.2k"` (one decimal, dropped once we'd cross 10k).
/// - `< 1_000_000`: `"47k"` (integer).
/// - `>= 1_000_000`: `"1.2M"` then `"12M"` (parallel to the k branch).
///
/// Pure function so the unit tests cover the threshold boundaries
/// without dragging GPUI into the test binary.
pub(crate) fn humanize_token_count(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        // 1234 -> "1.2k", 9999 -> "10.0k"? -- guard the rounding so
        // 9999 -> "10k" (single integer, dropping the decimal once we
        // hit the next decade).
        let tenths = (count + 50) / 100; // round to nearest tenth-of-k
        let whole = tenths / 10;
        let frac = tenths % 10;
        if whole >= 10 {
            return format!("{whole}k");
        }
        if frac == 0 {
            return format!("{whole}.0k");
        }
        return format!("{whole}.{frac}k");
    }
    if count < 1_000_000 {
        let rounded = (count + 500) / 1_000;
        return format!("{rounded}k");
    }
    if count < 10_000_000 {
        let tenths = (count + 50_000) / 100_000;
        let whole = tenths / 10;
        let frac = tenths % 10;
        if frac == 0 {
            return format!("{whole}.0M");
        }
        return format!("{whole}.{frac}M");
    }
    let rounded = (count + 500_000) / 1_000_000;
    format!("{rounded}M")
}

/// US-121 helper: format elapsed seconds for the activity-bar suffix.
/// Below 60s -> `"<n>s"`. From 60s onward -> `"<m>m<ss>"` (zero-padded
/// seconds, no separator). No `h` suffix per AC #1 -- hours stay in
/// `Nm` even if exceptionally long (a 65-minute turn renders as
/// `65m07`, not `1h05m07`).
fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let minutes = secs / 60;
    let leftover = secs % 60;
    format!("{minutes}m{leftover:02}")
}

fn is_at_bottom(event: &ListScrollEvent) -> bool {
    if event.count == 0 {
        return true;
    }
    event.visible_range.end >= event.count
}

fn join_text_blocks(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t.text);
            }
            ContentBlock::Image(_) => out.push_str("\n[image]"),
            ContentBlock::Audio(_) => out.push_str("\n[audio]"),
            ContentBlock::ResourceLink(_) => out.push_str("\n[resource link]"),
            ContentBlock::Resource(_) => out.push_str("\n[resource]"),
            _ => out.push_str("\n[unknown content block]"),
        }
    }
    out
}

/// US-020: pure helper -- given the live timeline `items`, build the
/// truncated message list for a fork.
fn build_fork_messages_from_items(
    items: &[ThreadItem],
    message_idx: usize,
    new_text: &str,
) -> Option<Vec<Message>> {
    if message_idx >= items.len() {
        return None;
    }
    let mut out: Vec<Message> = Vec::new();
    for item in &items[..=message_idx] {
        match item {
            ThreadItem::UserMessage(um) => out.push(um.msg.clone()),
            ThreadItem::AssistantMessage(am) => {
                let text = am
                    .chunks
                    .iter()
                    .filter_map(|c| match c {
                        AssistantMessageChunk::Text { text, .. } => Some(text.clone()),
                        AssistantMessageChunk::Thought { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    out.push(Message::assistant_text(text));
                }
            }
            ThreadItem::ToolCall(_) => {}
        }
    }
    let last = out.last_mut()?;
    if last.role != MessageRole::User {
        return None;
    }
    last.content = vec![ContentBlock::Text(
        agent_client_protocol::schema::TextContent::new(new_text),
    )];
    Some(out)
}

fn render_user_message_with_edit_affordance(
    message_idx: usize,
    bubble: AnyElement,
    is_streaming: bool,
    ui: crate::theme::UiColors,
    cx: &mut gpui::Context<ThreadView>,
) -> AnyElement {
    // Mirror Zed `thread_view.rs:4977-5022` — edit-mode action buttons
    // live in a small bordered pill anchored absolute top_neg_3p5
    // right_3 (i.e. -14px top, 12px right) so they hover just above
    // the bubble's top-right corner. Visible on hover via the parent
    // group; greyed out while a turn is streaming.
    let group_name: gpui::SharedString = format!("user-msg-{message_idx}").into();
    let btn_id: gpui::SharedString = format!("user-msg-edit-{message_idx}").into();
    let icon = gpui::svg()
        .size(px(12.))
        .path("icons/tool_pencil.svg")
        .text_color(if is_streaming { ui.muted } else { ui.text });
    let inner_btn = div()
        .id(btn_id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.))
        .h(px(22.))
        .text_color(if is_streaming { ui.muted } else { ui.text })
        .child(icon);
    let inner_btn = if is_streaming {
        inner_btn.into_any_element()
    } else {
        inner_btn
            .cursor_pointer()
            .hover(|s| s.bg(ui.subtle))
            .on_click(cx.listener(move |this, _ev: &gpui::ClickEvent, w, cx| {
                this.start_edit(message_idx, w, cx);
            }))
            .into_any_element()
    };
    // Pill sits INSIDE the bubble at the top-right corner — no
    // overflow above, no clip-rect surprises.
    let pill = div()
        .absolute()
        .top(px(6.))
        .right(px(6.))
        .flex()
        .flex_row()
        .gap(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(ui.border)
        .bg(ui.base)
        .overflow_hidden()
        .opacity(0.0)
        .group_hover(group_name.clone(), |s| {
            s.opacity(if is_streaming { 0.4 } else { 1.0 })
        })
        .child(inner_btn);
    div()
        .relative()
        .group(group_name)
        .child(bubble)
        .child(pill)
        .into_any_element()
}

fn render_user_edit_row(
    state: &EditState,
    ui: crate::theme::UiColors,
    cx: &mut gpui::Context<ThreadView>,
) -> AnyElement {
    // Edit-mode bubble. Matches the composer card so the user gets the
    // same visual + interaction model: `ui.surface` bg, neutral border,
    // `rounded(14)` corners, `p(8)` inset around the TextArea — which
    // already handles selection / cut / copy / delete / paste natively.
    // Cancel + Save sit in a floating pill at the top-right corner.
    let editor = state.text_area.clone();
    let cancel_btn = div()
        .id("thread-edit-cancel")
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.))
        .h(px(22.))
        .cursor_pointer()
        .hover(|s| s.bg(ui.subtle))
        .child(
            gpui::svg()
                .size(px(12.))
                .path("icons/close.svg")
                .text_color(ui.muted),
        )
        .on_click(cx.listener(|this, _ev: &gpui::ClickEvent, _w, cx| {
            this.cancel_edit(cx);
        }));
    let save_btn = div()
        .id("thread-edit-save")
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.))
        .h(px(22.))
        .cursor_pointer()
        .hover(|s| s.bg(ui.subtle))
        .child(
            gpui::svg()
                .size(px(12.))
                .path("icons/check.svg")
                .text_color(ui.text),
        )
        .on_click(cx.listener(|this, _ev: &gpui::ClickEvent, _w, cx| {
            this.commit_edit(cx);
        }));
    // Action pill sits INSIDE the bubble at the top-right corner.
    let action_pill = div()
        .absolute()
        .top(px(6.))
        .right(px(6.))
        .flex()
        .flex_row()
        .gap(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(ui.border)
        .bg(ui.base)
        .overflow_hidden()
        .child(cancel_btn)
        .child(save_btn);
    div()
        .pt(px(8.))
        .pb(px(12.))
        .px(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
            // Esc — cancel the edit (matches the composer's Esc-on-popup
            // behavior). The TextArea swallows printable keys, but Esc
            // bubbles up to this wrapper.
            if ev.keystroke.key == "escape" {
                this.cancel_edit(cx);
            }
        }))
        .child(
            div()
                .id("thread-edit-card")
                .relative()
                .p(px(8.))
                .rounded(px(14.))
                .bg(ui.surface)
                .border_1()
                .border_color(ui.border)
                .text_size(px(12.))
                .text_color(ui.text)
                // I-beam cursor over the card + click anywhere on
                // the padding to focus the TextArea, matching the
                // composer's `cursor_text + on_click → focus` pattern
                // (composer.rs:3656-3660).
                .cursor_text()
                .on_click(cx.listener(|this, _ev: &gpui::ClickEvent, w, cx| {
                    if let Some(state) = this.editing.as_ref() {
                        let handle = state.text_area.read(cx).focus_handle_ref().clone();
                        handle.focus(w, cx);
                    }
                }))
                .child(editor)
                .child(action_pill),
        )
        .into_any_element()
}

fn empty_state() -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(px(13.))
                .text_color(rgb(0x808080))
                .child("No messages yet."),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_at_bottom_returns_true_when_visible_extends_past_last_index() {
        let event = ListScrollEvent {
            visible_range: 5..10,
            count: 10,
            is_scrolled: true,
            is_following_tail: false,
        };
        assert!(is_at_bottom(&event));
    }

    #[test]
    fn is_at_bottom_returns_false_when_user_scrolled_up() {
        let event = ListScrollEvent {
            visible_range: 0..5,
            count: 10,
            is_scrolled: true,
            is_following_tail: false,
        };
        assert!(!is_at_bottom(&event));
    }

    #[test]
    fn is_at_bottom_handles_empty_list_gracefully() {
        let event = ListScrollEvent {
            visible_range: 0..0,
            count: 0,
            is_scrolled: false,
            is_following_tail: true,
        };
        assert!(is_at_bottom(&event));
    }

    #[test]
    fn join_text_blocks_concatenates_text_with_newlines() {
        let msg_a = Message::user_text("line 1");
        let msg_b = Message::user_text("line 2");
        let mut combined: Vec<ContentBlock> = Vec::new();
        combined.extend(msg_a.content);
        combined.extend(msg_b.content);
        assert_eq!(join_text_blocks(&combined), "line 1\nline 2");
    }

    // ----------------------------------------------------------------
    // US-121: helpers for the activity-bar elapsed + token suffixes.
    // The threshold gates themselves are exercised through
    // compute_activity_state in integration; these tests lock the
    // formatting contract.
    // ----------------------------------------------------------------

    #[test]
    fn format_elapsed_below_minute_renders_as_seconds() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(3), "3s");
        assert_eq!(format_elapsed(12), "12s");
        assert_eq!(format_elapsed(59), "59s");
    }

    #[test]
    fn format_elapsed_at_and_past_minute_renders_as_mss() {
        assert_eq!(format_elapsed(60), "1m00");
        assert_eq!(format_elapsed(62), "1m02");
        assert_eq!(format_elapsed(125), "2m05");
        // No `h` suffix per AC #1 -- 65 minutes stays in `Nm` shape.
        assert_eq!(format_elapsed(65 * 60 + 7), "65m07");
    }

    #[test]
    fn humanize_token_count_below_threshold_is_raw_integer() {
        // Used by the rare path where a caller wants the count even
        // below the activity-bar gate. compute_activity_state itself
        // never calls humanize for count < TOKEN_SUFFIX_THRESHOLD.
        assert_eq!(humanize_token_count(0), "0");
        assert_eq!(humanize_token_count(999), "999");
    }

    #[test]
    fn humanize_token_count_at_one_thousand_rounds_to_one_decimal_k() {
        // 1000 -> "1.0k"; 1234 -> "1.2k"; 1250 -> "1.3k" (banker would
        // round to 1.2; we use +50 / 100 floor so 1250 lands on 1.3).
        assert_eq!(humanize_token_count(1_000), "1.0k");
        assert_eq!(humanize_token_count(1_234), "1.2k");
        assert_eq!(humanize_token_count(1_250), "1.3k");
    }

    #[test]
    fn humanize_token_count_above_ten_thousand_drops_decimal() {
        assert_eq!(humanize_token_count(12_345), "12k");
        assert_eq!(humanize_token_count(47_000), "47k");
        // 9999 lands in the `< 10_000` branch but rounds up to 10.0k,
        // which the inner guard collapses to "10k" so we never emit a
        // mismatched-precision label adjacent to a 10k+ count.
        assert_eq!(humanize_token_count(9_999), "10k");
    }

    #[test]
    fn humanize_token_count_million_scale() {
        assert_eq!(humanize_token_count(1_000_000), "1.0M");
        assert_eq!(humanize_token_count(1_234_567), "1.2M");
        assert_eq!(humanize_token_count(12_345_678), "12M");
    }
}
