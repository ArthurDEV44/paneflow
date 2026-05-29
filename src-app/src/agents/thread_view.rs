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

/// Default tick cadence for the streaming pipeline (60 Hz). Used
/// while the open assistant message body is below
/// [`STREAMING_ADAPT_MEDIUM`].
const STREAMING_TICK_FAST: Duration = Duration::from_millis(16);
/// Tick cadence once the open message crosses [`STREAMING_ADAPT_MEDIUM`].
/// `markdown::Markdown::append` reconstructs the full source string
/// on every call (concat into a new SharedString); past ~4 KB the per-
/// call cost starts to matter at 60 Hz. Dropping to ~20 Hz keeps
/// streaming visibly smooth (text reading is paragraph-paced, not
/// char-by-char) while quartering the append CPU.
const STREAMING_TICK_MEDIUM: Duration = Duration::from_millis(50);
/// Tick cadence once the open message crosses [`STREAMING_ADAPT_LONG`].
/// At ~16 KB+ the append cost dominates the main thread budget; ~7 Hz
/// keeps the integral over a long response bounded without a visible
/// "frozen" period.
const STREAMING_TICK_SLOW: Duration = Duration::from_millis(150);
/// Char-count threshold at which the streaming tick transitions
/// from FAST to MEDIUM. Picked from the audit profile: below this,
/// 60 Hz is cheap enough to keep.
const STREAMING_ADAPT_MEDIUM: usize = 4_096;
/// Char-count threshold at which the streaming tick transitions
/// from MEDIUM to SLOW.
const STREAMING_ADAPT_LONG: usize = 16_384;
/// Throttle window for streaming-hot persist writes. Keeps SQLite I/O
/// off the GPUI main thread's 16 ms budget by batching chunks.
const PERSIST_THROTTLE: Duration = Duration::from_millis(500);

/// Pick the streaming tick cadence appropriate for the current open
/// assistant message length. Reduces `markdown::Markdown::append`
/// frequency as the source string grows so the cumulative O(n^2)
/// concat cost stays bounded. Returning a `Duration` (not a multiplier)
/// keeps the constants readable side-by-side at the top of the module.
/// Compute the tool-call positions a from-scratch scan of `items`
/// would produce. Single source of truth shared by the production
/// debug-assert (see [`ThreadView::assert_tool_call_indices_consistent`])
/// and the unit tests that pin the invariant down. Pure -- no GPUI
/// context required.
fn expected_tool_call_indices(items: &[ThreadItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| matches!(item, ThreadItem::ToolCall(_)).then_some(i))
        .collect()
}

fn adaptive_streaming_tick(open_message_chars: usize) -> Duration {
    if open_message_chars >= STREAMING_ADAPT_LONG {
        STREAMING_TICK_SLOW
    } else if open_message_chars >= STREAMING_ADAPT_MEDIUM {
        STREAMING_TICK_MEDIUM
    } else {
        STREAMING_TICK_FAST
    }
}

/// One row in the thread's scrollable timeline. Mirrors Zed's
/// `AgentThreadEntry` (acp_thread.rs:178): user messages, assistant
/// turns (with internal text+thought chunks), and top-level tool
/// calls live as separate variants — no grouping card.
pub enum ThreadItem {
    UserMessage(UserMessage),
    AssistantMessage(AssistantMessage),
    /// Tool calls are reference-counted so the render path can hand
    /// a cheap pointer copy to downstream renderers instead of doing
    /// a full struct + string + diff-vec clone per visible item per
    /// frame. Mutations use `Arc::make_mut`, which clones only when
    /// the Arc is shared -- inside `items` it is uniquely held so
    /// `make_mut` is in-place for the common case.
    ToolCall(Arc<ToolCallSnapshot>),
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
    /// Paced reveal for thinking chunks. Parallel to
    /// [`Self::streaming_buffer`] (text). Thinking bursts often arrive
    /// as 100+ rapid tokens; routing them through a buffer prevents
    /// the same per-token `cx.notify()` + `Markdown::append` storm
    /// that hits the text path on long responses (audit P1-2).
    thinking_buffer: StreamingBuffer,
    /// Index in `items` of the currently-open assistant message, or
    /// `None` when no streaming turn is active. The currently-open
    /// assistant message is the only one new `Text` / `Thought` chunks
    /// append to; user messages and tool calls close it.
    streaming_message_idx: Option<usize>,
    _streaming_task: Option<Task<()>>,
    /// Deadline at which the next throttled persist should fire. Set by
    /// `schedule_persist` (streaming-hot paths) and consumed by the
    /// streaming task tick. `flush_streaming` (turn end) forces a final
    /// persist and clears the deadline so a partially-armed deadline
    /// can never miss the final snapshot.
    persist_deadline: Option<std::time::Instant>,
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

    /// Reverse-index: `tool_call_id -> position in self.items`. Hot path
    /// optimisation so `update_tool_call`, `toggle_tool_call_expanded`
    /// and permission handlers do not scan every item on each ACP patch
    /// (Zed parity for the `tool_calls` HashMap on `AcpThread`). Kept in
    /// sync at every items-mutation point: pushes append, truncates
    /// retain entries below the cut, clears reset.
    tool_call_index: HashMap<String, usize>,

    /// Items-order list of indices that hold a `ToolCall`. Kept in
    /// sync with [`Self::tool_call_index`] at every mutation point.
    /// `compute_activity_state` iterates this instead of the full
    /// `items` Vec so its O(N) scan only walks the tool-call subset
    /// rather than every user message + assistant chunk row -- a 2-5x
    /// speedup on typical threads where most items are NOT tool calls
    /// (audit P0-5: the previous full-vec scan ran on every
    /// `cx.notify()` and grew with timeline length).
    tool_call_ordered_indices: Vec<usize>,

    /// US-022 (cli-hardening-followup-2026-Q3): per-item persisted-
    /// form cache parallel to `self.items`. Each slot is `Some` when
    /// the cached entry is up-to-date with the corresponding item;
    /// `None` when the slot is dirty and must be rebuilt on the next
    /// `collect_persisted_items` call.
    ///
    /// Invariants:
    /// - `persist_cache_items.len() == self.items.len()` at every
    ///   public mutation boundary; the helpers
    ///   [`Self::cache_push_slot`] / [`Self::cache_truncate`] /
    ///   [`Self::cache_clear`] keep them in sync, and any in-place
    ///   item mutation must call [`Self::mark_persist_dirty`] (or
    ///   [`Self::invalidate_persist_cache_all`] for sweeping
    ///   changes) before the next persist.
    /// - The hot streaming path (`push_streaming_chunk`,
    ///   `update_tool_call`, `push_thinking_chunk`,
    ///   `append_message`) is instrumented so that on a 200-item
    ///   thread with 2 streaming chunks per persist tick, only 2
    ///   slots get re-serialised instead of all 200. Mutation
    ///   sites that are not on the hot path call
    ///   `invalidate_persist_cache_all` for safety (correctness
    ///   over saving allocations on rare paths).
    persist_cache_items: Vec<Option<PersistedThreadItem>>,

    /// User-forced expand state for inline tool-call bursts, keyed by
    /// the index of the burst's first item in `items`. Absence ==
    /// follow the auto policy (open while any tool in the burst is
    /// non-terminal, closed once every tool is terminal). A
    /// `WaitingForConfirmation` tool always wins regardless of the
    /// override -- the user can't dismiss a permission prompt by
    /// collapsing the group. Not persisted: reload always starts with
    /// the auto policy so a re-opened thread doesn't inherit stale UI
    /// state from the previous session.
    tool_group_user_open: HashMap<usize, bool>,

    /// Key of the in-progress streaming Thought chunk, if any. Set by
    /// `push_thinking_chunk` when a new Thought is opened, cleared by
    /// `finalize_thinking` (after auto-collapsing) or by the user
    /// explicitly toggling. Used to auto-collapse the thinking block
    /// at the end of the burst, but only when the user has not opened
    /// it themselves — mirrors Zed
    /// `agent_ui/src/conversation_view/thread_view.rs::auto_expand_streaming_thought`.
    streaming_thinking_key: Option<(usize, usize)>,
    /// Handle to the in-flight background title-summarization task.
    /// Mirrors Zed `crates/agent/src/thread.rs:962`
    /// (`pending_title_generation`) — storing the `Task<()>` here gives
    /// us a cheap way to cancel a previous in-flight summarizer when a
    /// fresh trigger arrives (the `Task` is dropped, the inner future
    /// stops being polled, and the spawned `claude -p` subprocess gets
    /// reaped by the OS). Without this guard two `TurnEnded` events
    /// arriving rapidly (retry, reconnect) would race two `claude -p`
    /// children whose last-write-wins outcome is undefined.
    pending_title_generation: Option<Task<()>>,
    /// Set when the last summarization run failed (subprocess error,
    /// timeout, or empty output). Visible via
    /// [`Self::title_generation_failed`] so future UI can offer a
    /// retry button and tests can assert on the failure path instead
    /// of relying on log scraping. Zed parity:
    /// `crates/agent/src/thread.rs:963` `title_generation_failed`.
    title_generation_failed: bool,

    /// Theme snapshot captured at the top of `render`; cleared on next
    /// render. `render_item` (driven by the virtualized List) reads
    /// this instead of re-locking the global theme cache. Saves
    /// O(visible_items) mutex acquisitions per frame.
    _theme_snapshot: Option<crate::theme::TerminalTheme>,
    /// UI palette snapshot captured at the top of `render`, derived
    /// from `_theme_snapshot`. Same rationale: reused by `render_item`.
    _ui_snapshot: Option<crate::theme::UiColors>,
    /// Composer cwd snapshot captured at the top of `render`; cleared
    /// on next render. `render_item` (driven by the virtualized List)
    /// reads this instead of doing
    /// `composer.read(cx).cwd().to_path_buf()` per visible message --
    /// the cwd does not change inside a single render pass (audit P2-4).
    /// Stored as `Arc<Path>` so per-item clones are a refcount bump
    /// rather than a fresh `PathBuf` allocation. The link-handler
    /// closure needs `'static` ownership of the path so a borrowed
    /// `&Path` cannot replace this entirely (review follow-up).
    _cwd_snapshot: Option<std::sync::Arc<std::path::Path>>,
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
                    items.push(ThreadItem::ToolCall(Arc::new(
                        ToolCallSnapshot::from_persisted(p),
                    )));
                }
            }
        }

        // Reverse-index of every tool-call id → its position in `items`
        // so update_tool_call / toggle_tool_call_expanded / permission
        // handlers run in O(1) instead of scanning the whole timeline.
        let mut tool_call_index: HashMap<String, usize> = HashMap::new();
        let mut tool_call_ordered_indices: Vec<usize> = Vec::new();
        for (idx, item) in items.iter().enumerate() {
            if let ThreadItem::ToolCall(snap) = item {
                tool_call_index.insert(snap.id.clone(), idx);
                tool_call_ordered_indices.push(idx);
            }
        }

        let count = items.len();
        let list_state = ListState::new(count, ListAlignment::Top, px(400.));
        let should_be_following = Rc::new(Cell::new(true));
        {
            let flag = Rc::clone(&should_be_following);
            // Re-entrancy guard: this closure fires inside `ListState`'s
            // internal scroll dispatch. Calling back into the same
            // `list_state` (e.g. `logical_scroll_top()`, `splice()`,
            // `scroll_to_end()`) double-borrows the inner RefCell and
            // panics — Zed documents the same hazard at
            // `crates/agent_ui/src/conversation_view/thread_view.rs:926`.
            // Keep the body restricted to data carried by the event; for
            // any list_state mutation defer via `cx.defer`.
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

        let initial_persist_cache_len = items.len();
        Self {
            store_id,
            store,
            items,
            list_state,
            should_be_following,
            streaming_buffer: StreamingBuffer::new(PacingConfig::default()),
            thinking_buffer: StreamingBuffer::new(PacingConfig::default()),
            streaming_message_idx: None,
            _streaming_task: None,
            composer,
            editing: None,
            collapsed_thoughts: HashSet::new(),
            tool_label_markdown: HashMap::new(),
            generating_indicator_active: false,
            reviewed_edits,
            diff_scroll_handles: HashMap::new(),
            tool_call_index,
            tool_call_ordered_indices,
            // US-022: cache starts empty-but-correctly-sized; first
            // `collect_persisted_items` fills every slot. Subsequent
            // calls only rebuild the slots flagged dirty by
            // `mark_persist_dirty`.
            persist_cache_items: vec![None; initial_persist_cache_len],
            tool_group_user_open: HashMap::new(),
            persist_deadline: None,
            streaming_thinking_key: None,
            pending_title_generation: None,
            title_generation_failed: false,
            _theme_snapshot: None,
            _ui_snapshot: None,
            _cwd_snapshot: None,
        }
    }

    /// Spawn a background title summarizer for this thread, replacing
    /// any in-flight one. The previous `Task` is dropped before the
    /// new spawn so two `claude -p` subprocesses never coexist for
    /// the same thread (Zed parity guard at `thread.rs:962`).
    ///
    /// Returns `true` when a new task was armed, `false` when the
    /// summarizer chose to no-op (e.g. ACP agent that pushes its own
    /// title, or `claude` CLI missing).
    pub fn start_title_summarization(
        &mut self,
        req: super::title_summarizer::SummarizeRequest,
        cx: &mut Context<Self>,
    ) -> bool {
        // Drop the previous task explicitly so cancellation order is
        // deterministic (drop runs the `Task`'s destructor before the
        // new one is moved into the slot).
        self.pending_title_generation = None;
        // A new run wipes the prior failure state; only the latest
        // outcome should be user-visible.
        self.title_generation_failed = false;
        match super::title_summarizer::summarize_thread_title_task(req, cx) {
            Some(task) => {
                self.pending_title_generation = Some(task);
                true
            }
            None => false,
        }
    }

    /// Called from the background summarizer when the run completes
    /// successfully. Clears the pending-task slot so callers can use
    /// [`Self::is_title_generation_pending`] as a "real" busy signal.
    pub fn note_title_generation_succeeded(&mut self) {
        self.pending_title_generation = None;
        self.title_generation_failed = false;
    }

    /// Called from the background summarizer on any failure path.
    /// Sets [`Self::title_generation_failed`] and notifies so any
    /// retry-affordance UI can light up.
    pub fn note_title_generation_failed(&mut self, cx: &mut Context<Self>) {
        self.pending_title_generation = None;
        self.title_generation_failed = true;
        cx.notify();
    }

    /// Whether a background title summarizer is currently in flight.
    pub fn is_title_generation_pending(&self) -> bool {
        self.pending_title_generation.is_some()
    }

    /// Whether the most recent title summarization run ended in failure.
    pub fn title_generation_failed(&self) -> bool {
        self.title_generation_failed
    }

    /// Throttled persist. Arms a deadline so the next streaming-task
    /// tick (every 16 ms) writes to SQLite at most every
    /// [`PERSIST_THROTTLE`]. Mirrors Zed's `schedule_save` pattern at
    /// `crates/agent_ui/src/conversation_view/thread_view.rs:945-958`.
    fn schedule_persist(&mut self) {
        if self.persist_deadline.is_none() {
            self.persist_deadline = Some(std::time::Instant::now() + PERSIST_THROTTLE);
        }
    }

    /// Fire the throttled persist if its deadline has elapsed. Called
    /// from the streaming task tick.
    fn tick_persist(&mut self, cx: &gpui::App) {
        let Some(deadline) = self.persist_deadline else {
            return;
        };
        if std::time::Instant::now() < deadline {
            return;
        }
        self.persist_deadline = None;
        self.persist_snapshot_now(cx);
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
                // Drain any text-in-flight before closing the assistant
                // turn — same ordering guarantee as `add_tool_call`. A
                // user message interrupting an assistant stream is rare
                // (typically the composer is locked) but the same data-
                // loss path applies to it.
                self.drain_streaming_buffer_into_open(cx);
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
        log::debug!(
            target: "agents::stream",
            "add_tool_call id={} pending_chars={} streaming_idx={:?}",
            snapshot.id,
            self.streaming_buffer.pending_chars(),
            self.streaming_message_idx,
        );
        // CRITICAL: drain the paced streaming buffer into the still-open
        // assistant message BEFORE closing it. Otherwise chars queued
        // between the last 16 ms tick and this tool_call get stranded in
        // the buffer; the streaming task observes `idx=None` and bails,
        // and the next `begin_assistant_stream` reveals them into a
        // *new* AssistantMessage AFTER the tool — producing the visible
        // "Laiss" + tool_calls + "e-moi scanner..." split (Claude Code
        // thread regression reported 2026-05-26).
        self.drain_streaming_buffer_into_open(cx);
        self.close_open_assistant_message();
        let id = snapshot.id.clone();
        let title = snapshot.title.clone();
        let prev_count = self.items.len();
        self.items.push(ThreadItem::ToolCall(Arc::new(snapshot)));
        // US-022: keep cache parallel to items.
        self.persist_cache_items.push(None);
        self.tool_call_index.insert(id.clone(), prev_count);
        self.tool_call_ordered_indices.push(prev_count);
        #[cfg(debug_assertions)]
        self.assert_tool_call_indices_consistent();
        self.list_state.splice(prev_count..prev_count, 1);
        let md = Self::make_markdown(&title, cx);
        self.tool_label_markdown.insert(id, md);
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        // Throttled persist (streaming hot path): a busy turn can emit
        // dozens of tool calls in a second. The final state is forced
        // to disk on `flush_streaming` / `sweep_*_tools_*`.
        self.schedule_persist();
        cx.notify();
    }

    /// Append a thinking chunk to the currently-open assistant
    /// message. Opens a new assistant turn if none is active.
    pub fn push_thinking_chunk(&mut self, chunk: &str, cx: &mut Context<Self>) {
        if chunk.is_empty() {
            return;
        }
        log::trace!(
            target: "agents::stream",
            "push_thinking_chunk len={} pending_chars={} streaming_idx={:?}",
            chunk.len(),
            self.streaming_buffer.pending_chars(),
            self.streaming_message_idx,
        );
        // Same drain rationale as `add_tool_call`: a Thought arriving
        // between two text ticks would otherwise let the queued chars
        // reveal AFTER the Thought, producing a reversed Thought→Text
        // order inside the same assistant turn.
        self.drain_streaming_buffer_into_open(cx);
        let idx = self.ensure_open_assistant_message(cx);
        // Make sure there is an open Thought chunk for the streaming
        // task to append into. The actual chars are queued in
        // `thinking_buffer` and revealed by the streaming tick -- this
        // path only handles the chunk-creation bookkeeping (audit
        // P1-2: avoid the per-push `markdown.append` + `cx.notify`
        // storm on rapid thinking bursts).
        if let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(idx) {
            let needs_new_thought = !matches!(
                am.chunks.last(),
                Some(AssistantMessageChunk::Thought { .. }),
            );
            if needs_new_thought {
                let md = Self::make_markdown("", cx);
                let chunk_idx = am.chunks.len();
                am.chunks.push(AssistantMessageChunk::Thought {
                    text: String::new(),
                    markdown: md,
                    signature: None,
                });
                // Track this thought as the current "auto-expandable"
                // burst so `finalize_thinking` can auto-collapse it
                // when the burst ends -- unless the user has
                // explicitly touched it in the meantime.
                self.streaming_thinking_key = Some((idx, chunk_idx));
            }
        }
        self.thinking_buffer.push(chunk);
        // Throttled persist (streaming hot path) -- thinking bursts emit
        // many small chunks. Final flush happens at turn end. No
        // `cx.notify` here: the streaming task tick coalesces reveals
        // and notifies once per frame budget.
        self.schedule_persist();
    }

    /// Close the currently-open assistant turn. After this returns,
    /// the next text / thought chunk opens a fresh turn. Idempotent.
    pub fn finalize_thinking(&mut self, cx: &mut Context<Self>) {
        self.close_open_assistant_message();
        // Auto-collapse the streaming thinking burst at the end of the
        // turn so the body stays readable. The user can still expand
        // it via the disclosure chevron; their explicit toggle clears
        // `streaming_thinking_key` so this branch is a no-op next time.
        if let Some(key) = self.streaming_thinking_key.take() {
            self.collapsed_thoughts.insert(key);
        }
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
            // A refusal is also a hard stop: the model declined to keep
            // going, so any tool call still `Pending`/`InProgress` will
            // never complete. Leaving them in that state would freeze
            // the activity-bar spinner ("Reading file…") on reload.
            // Treat as Cancelled, same as an explicit `Cancelled`.
            StopReasonKind::Cancelled | StopReasonKind::Refusal | StopReasonKind::Other => {
                ToolCallStatusKind::Canceled
            }
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
                let snap = Arc::make_mut(snap);
                snap.status = target;
                // The permission picker is bound to a live `pending`
                // tool call; once we transition it to a terminal state
                // the popover is no longer interactable. Clear so a
                // reload of the thread does not re-open it from disk.
                snap.permission_picker_open = false;
                snap.permission_options.clear();
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
        // US-004 (cli-hardening-followup-2026-Q3): collect ids whose
        // diff-empty payload is now terminal so we can drop their
        // label / scroll bookkeeping AFTER the mutating pass
        // releases the `&mut self.items` borrow.
        let mut swept: Vec<(String, bool)> = Vec::new();
        for item in &mut self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(
                    snap.status,
                    ToolCallStatusKind::Pending
                        | ToolCallStatusKind::InProgress
                        | ToolCallStatusKind::WaitingForConfirmation
                )
            {
                let id = snap.id.clone();
                let has_diffs = !snap.diffs.is_empty();
                let snap = Arc::make_mut(snap);
                snap.status = ToolCallStatusKind::Failed;
                snap.permission_picker_open = false;
                snap.permission_options.clear();
                touched = true;
                swept.push((id, has_diffs));
            }
        }
        for (id, has_diffs) in swept {
            apply_terminal_tool_ui_prune(
                &mut self.tool_label_markdown,
                &mut self.diff_scroll_handles,
                &id,
                true,
                has_diffs,
            );
        }
        if touched {
            // US-022: sweep changed many tool-call snapshots in one
            // pass; the safest move is to drop the whole cache.
            self.invalidate_persist_cache_all();
            self.persist_snapshot_now(cx);
            cx.notify();
        }
        touched
    }

    /// Apply a `ToolCallUpdate` patch. Silently no-ops on unknown
    /// ids (defensive: ACP servers occasionally emit updates for
    /// already-finalised calls).
    pub fn update_tool_call(&mut self, patch: ToolCallUpdate, cx: &mut Context<Self>) {
        use super::runtime::ToolCallStatusKind;
        let new_title = patch.title.clone();
        // O(1) lookup via the reverse index. Guard against the index
        // pointing at a removed/replaced slot (truncate_for_edit / a
        // future swap) by validating the item is still the expected
        // ToolCall with the matching id before applying.
        let post = if let Some(&idx) = self.tool_call_index.get(&patch.id)
            && let Some(ThreadItem::ToolCall(snap)) = self.items.get_mut(idx)
            && snap.id == patch.id
        {
            let id = snap.id.clone();
            apply_patch(Arc::make_mut(snap), patch);
            // Capture the post-patch terminal/diffs state so we can
            // release the inner borrow on `self.items` before
            // touching `self.tool_label_markdown` /
            // `self.diff_scroll_handles`.
            let is_terminal = matches!(
                snap.status,
                ToolCallStatusKind::Completed | ToolCallStatusKind::Failed
            );
            let has_diffs = !snap.diffs.is_empty();
            Some((id, is_terminal, has_diffs))
        } else {
            None
        };
        if let Some((id, is_terminal, has_diffs)) = post {
            // US-004 (cli-hardening-followup-2026-Q3): drop UI
            // bookkeeping once the call hits a terminal status.
            // - non-edit calls (no diffs): drop both the label
            //   markdown and the scroll handle. They never trigger
            //   Keep All / Reject All so US-015's review-purge path
            //   could never reach them; on a 200-tool-call session
            //   that's 200 retained `Entity<Markdown>` entries.
            // - edit calls (has diffs): keep the label markdown so
            //   the header path stays styled on re-expansion; drop
            //   the live scroll handle (it is recreated lazily by
            //   the render pass).
            // Re-inserting the title further down is skipped on
            // non-edit terminals so we don't undo our own work.
            let skip_title_insert = is_terminal && !has_diffs;
            if let Some(title) = new_title
                && !skip_title_insert
            {
                let md = Self::make_markdown(&title, cx);
                self.tool_label_markdown.insert(id.clone(), md);
            }
            apply_terminal_tool_ui_prune(
                &mut self.tool_label_markdown,
                &mut self.diff_scroll_handles,
                &id,
                is_terminal,
                has_diffs,
            );
            // US-022: tool-call snapshot changed; mark its cache
            // slot dirty so the next persist re-serialises only it.
            if let Some(&idx) = self.tool_call_index.get(&id) {
                self.mark_persist_dirty(idx);
            }
            // Throttled — `update_tool_call` is the hottest path on a
            // file-heavy turn (every diff chunk is a patch).
            self.schedule_persist();
            cx.notify();
        }
    }

    /// Toggle the expand/collapse state of one tool-call row.
    pub fn toggle_tool_call_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(&idx) = self.tool_call_index.get(id)
            && let Some(ThreadItem::ToolCall(snap)) = self.items.get_mut(idx)
            && snap.id == id
        {
            let snap = Arc::make_mut(snap);
            snap.expanded = !snap.expanded;
            cx.notify();
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
        let mut to_purge: Vec<String> = Vec::new();
        for item in &self.items {
            if let ThreadItem::ToolCall(snap) = item
                && matches!(snap.status, super::runtime::ToolCallStatusKind::Completed)
                && !snap.diffs.is_empty()
                && !self.reviewed_edits.contains(&snap.id)
                && self.reviewed_edits.insert(snap.id.clone())
            {
                to_purge.push(snap.id.clone());
            }
        }
        if !to_purge.is_empty() {
            // US-015: drop the per-tool-call header markdown + scroll
            // handle now that the card is final. Reduces long-session
            // GPUI entity bookkeeping.
            for id in &to_purge {
                self.purge_reviewed_tool_ui_state(id);
            }
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
            self.reviewed_edits.insert(id.clone());
            self.purge_reviewed_tool_ui_state(&id);
        }
        self.persist_snapshot_now(cx);
        cx.notify();
    }

    /// US-015 (audit P2-2): drop per-tool-call UI state once the user
    /// has reviewed the edit. The header `Markdown` entity and the
    /// diff-body `ScrollHandle` only matter while the card is
    /// actionable -- post-review the card stays visible but its
    /// header is a frozen string and the diff body is collapsed by
    /// default. Both entries are recreated lazily on demand if the
    /// user re-expands or the timeline truncates back over the
    /// reviewed point.
    ///
    /// US-001 (cli-hardening-followup-2026-Q3): on top of the
    /// per-card UI state, this function now also frees
    /// `DiffSnapshot.old_text` on every diff of the reviewed
    /// snapshot. Heavy refactor sessions (500 turns x 50 KB files x
    /// 20 edits each) retain 10-100 MB of pre-edit file content
    /// otherwise -- the `Arc<ToolCallSnapshot>` keeps every byte
    /// live for the full thread lifetime. The original line count is
    /// recorded in `DiffSnapshot.cleared_diff_lines` so the renderer
    /// at `edit_tool_block.rs::render_diff` can show
    /// `[diff body cleared after review, ~N lines]` on re-expansion
    /// instead of computing a `[]` vs `new_text` diff (which would
    /// mark every line as added).
    fn purge_reviewed_tool_ui_state(&mut self, id: &str) {
        self.tool_label_markdown.remove(id);
        self.diff_scroll_handles.remove(id);
        if let Some(&idx) = self.tool_call_index.get(id)
            && let Some(ThreadItem::ToolCall(snap)) = self.items.get_mut(idx)
        {
            // Only mutate if at least one diff still holds
            // `old_text` -- avoids an Arc clone when the snapshot was
            // already cleared (idempotent call from
            // `keep_all_edits` / `reject_all_edits`).
            let needs_clear = snap
                .diffs
                .iter()
                .any(|d| d.old_text.is_some() && d.cleared_diff_lines.is_none());
            if needs_clear {
                clear_reviewed_diff_bodies(Arc::make_mut(snap));
                // US-022: snapshot changed; refresh just that slot.
                self.mark_persist_dirty(idx);
            } else {
                // US-022 (correctness): even when no diff bodies
                // need clearing, the `reviewed_edits` set may have
                // changed for this id between cache write and now.
                // Force the slot to rebuild so `reviewed` flips
                // propagate to disk on the next persist.
                self.mark_persist_dirty(idx);
            }
        }
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
                    let snap = Arc::make_mut(snap);
                    snap.permission_picker_open = !snap.permission_picker_open;
                } else if snap.permission_picker_open {
                    Arc::make_mut(snap).permission_picker_open = false;
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
                Arc::make_mut(snap).permission_picker_open = false;
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
        // User explicitly took control of this block; cancel any
        // pending auto-collapse so a turn-end won't re-close what they
        // chose to open.
        if self.streaming_thinking_key == Some(key) {
            self.streaming_thinking_key = None;
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
        if let Some(&idx) = self.tool_call_index.get(tool_call_id)
            && let Some(ThreadItem::ToolCall(snap)) = self.items.get_mut(idx)
            && snap.id == tool_call_id
        {
            let snap = Arc::make_mut(snap);
            snap.status = super::runtime::ToolCallStatusKind::WaitingForConfirmation;
            snap.permission_options = options;
            self.persist_snapshot_now(cx);
            cx.notify();
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
        if let Some(&idx) = self.tool_call_index.get(tool_call_id)
            && let Some(ThreadItem::ToolCall(snap)) = self.items.get_mut(idx)
            && snap.id == tool_call_id
        {
            let snap = Arc::make_mut(snap);
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
            // Use the Zed `SpinnerLabel::dots()` Braille animation for
            // consistency with the inline-pixel spinner — vector rotation
            // on a `loader-circle.svg` can introduce sub-pixel artefacts
            // on HiDPI Wayland, the Braille spinner is pixel-perfect.
            use ui::{LabelCommon, LabelSize, SpinnerLabel};
            let section = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .text_color(ui.text)
                .child(SpinnerLabel::dots().size(LabelSize::Small))
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
                        inset: false,
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

    /// Invariant guard for [`Self::tool_call_ordered_indices`] and
    /// [`Self::tool_call_index`]. Both maps are maintained by hand at
    /// 4 mutation sites (push_tool_call, truncate_for_edit,
    /// clear_local_display, post-construction); a future addition to
    /// items mutation that forgets the maintenance call would silently
    /// rot [`Self::compute_activity_state`]. The check is debug-only:
    /// in release it compiles to nothing.
    #[cfg(debug_assertions)]
    fn assert_tool_call_indices_consistent(&self) {
        let expected = expected_tool_call_indices(&self.items);
        debug_assert_eq!(
            self.tool_call_ordered_indices, expected,
            "tool_call_ordered_indices drifted from items: expected {expected:?}, got {:?}",
            self.tool_call_ordered_indices,
        );
        for &idx in &self.tool_call_ordered_indices {
            if let Some(ThreadItem::ToolCall(snap)) = self.items.get(idx) {
                debug_assert_eq!(
                    self.tool_call_index.get(&snap.id),
                    Some(&idx),
                    "tool_call_index for id {} should be {idx}, got {:?}",
                    snap.id,
                    self.tool_call_index.get(&snap.id),
                );
            }
        }
    }

    fn compute_activity_state(&self, cx: &gpui::App) -> Option<ActivityBarState> {
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
        // Fused scan over only the tool-call indices, not the full
        // `items` Vec: a 500-item thread with 200 tool calls now
        // scans 200 positions per `cx.notify()` instead of 500.
        // `tool_call_ordered_indices` is maintained in items-order
        // alongside `tool_call_index` so the awaiting/running
        // "last match wins" semantics are unchanged. (Audit P0-5.)
        let mut awaiting: Option<AwaitingTool> = None;
        let mut running_kind: Option<String> = None;
        let mut files = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        for &idx in &self.tool_call_ordered_indices {
            let Some(ThreadItem::ToolCall(snap)) = self.items.get(idx) else {
                continue;
            };
            match snap.status {
                super::runtime::ToolCallStatusKind::WaitingForConfirmation => {
                    awaiting = Some(AwaitingTool {
                        item_idx: idx,
                        title: snap.title.clone(),
                    });
                }
                // `Pending`/`InProgress` snapshots survive a Fatal until
                // `sweep_pending_tools_on_fatal` has had a chance to run.
                // If the composer reports not-streaming, treat those as
                // stale and skip them so the activity bar doesn't spin
                // forever after a crash.
                super::runtime::ToolCallStatusKind::InProgress
                | super::runtime::ToolCallStatusKind::Pending
                    if is_streaming =>
                {
                    running_kind = Some(verbose_tool_kind_label(snap.kind).to_string());
                }
                super::runtime::ToolCallStatusKind::Completed
                    if !snap.diffs.is_empty() && !self.reviewed_edits.contains(&snap.id) =>
                {
                    files += snap.diffs.len();
                    let (a, r) = super::edit_tool_block::diff_stats(&snap.diffs);
                    added += a;
                    removed += r;
                }
                _ => {}
            }
        }
        let edits_summary = if files == 0 {
            None
        } else {
            Some(EditsReviewSummary {
                files,
                added,
                removed,
            })
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
        // US-022: keep cache parallel and invalidate (truncate
        // doesn't tell us which trailing slots changed semantically
        // -- safer to drop them so a future restore can't serve a
        // stale entry).
        self.persist_cache_items.truncate(message_idx);
        self.streaming_message_idx = None;
        let new_len = self.items.len();
        // Drop collapsed-thought state for items past the truncation
        // point so a future render doesn't index into removed slots.
        self.collapsed_thoughts
            .retain(|(msg_idx, _)| *msg_idx < new_len);
        // Reap tool-call-keyed caches for IDs that no longer exist.
        // Orphans are not visible (nothing references them in the items
        // list anymore) but they accumulate over long edit-heavy
        // sessions and tie Markdown entities + ScrollHandles alive.
        let surviving_ids: std::collections::HashSet<String> = self
            .items
            .iter()
            .filter_map(|it| match it {
                ThreadItem::ToolCall(snap) => Some(snap.id.clone()),
                _ => None,
            })
            .collect();
        self.tool_label_markdown
            .retain(|id, _| surviving_ids.contains(id));
        self.diff_scroll_handles
            .retain(|id, _| surviving_ids.contains(id));
        self.reviewed_edits.retain(|id| surviving_ids.contains(id));
        // Drop reverse-index entries for tool calls past the truncation
        // point. Indices of surviving items don't shift (we only chop
        // the tail) so no remapping is needed.
        self.tool_call_index
            .retain(|id, _| surviving_ids.contains(id));
        // Keep the ordered-index Vec in sync with the index map.
        // Truncation only chops the tail, so positions of surviving
        // tool calls do not shift.
        self.tool_call_ordered_indices.retain(|&idx| idx < new_len);
        #[cfg(debug_assertions)]
        self.assert_tool_call_indices_consistent();
        // Drop user-override entries for tool-group bursts whose
        // start index now sits past the truncation point.
        self.tool_group_user_open
            .retain(|&start_ix, _| start_ix < new_len);
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
        // US-022: drop cache contents too.
        self.persist_cache_items.clear();
        self.collapsed_thoughts.clear();
        self.tool_label_markdown.clear();
        // Also clear edit-card state so a `/clear`-then-replay path
        // doesn't render edit cards as "already reviewed" against ids
        // from a previous timeline.
        self.reviewed_edits.clear();
        self.diff_scroll_handles.clear();
        self.tool_call_index.clear();
        self.tool_call_ordered_indices.clear();
        #[cfg(debug_assertions)]
        self.assert_tool_call_indices_consistent();
        self.tool_group_user_open.clear();
        self.streaming_message_idx = None;
        self.list_state.reset(0);
        cx.notify();
    }

    /// Persist the full timeline snapshot to `threads.db`.
    ///
    /// The actual SQLite write is dispatched to `cx.background_spawn`
    /// so a slow disk (Snap-confined writes, fsync on NVMe under load,
    /// WAL checkpoint) never stalls the GPUI main thread. The blob is
    /// built synchronously on the main thread (cheap clones from the
    /// in-memory `items` Vec) and then moved into the background task,
    /// matching Zed's pattern at `thread_metadata_store.rs:989` where
    /// every save goes through `cx.background_spawn`. Previously this
    /// function held the `Arc<Mutex<Connection>>` for the duration of
    /// the WAL write inside hot streaming-tick callers like
    /// `flush_streaming` and `append_message`.
    pub fn persist_snapshot_now(&mut self, cx: &gpui::App) {
        // Clone the store + id up front so the immutable borrow of
        // `self` ends before the mutable `sync_persist_cache` call.
        let Some((store, id)) = self
            .store
            .as_ref()
            .zip(self.store_id.as_ref())
            .map(|(s, i)| (s.clone(), i.clone()))
        else {
            return;
        };
        let items = self.collect_persisted_items(cx);
        // US-022 (cli-hardening-followup-2026-Q3): write the
        // freshly-collected forms back into the per-item cache so
        // the NEXT persist tick reuses the still-clean slots.
        // Without this, the cache would stay all-None forever and
        // every tick would re-serialise the whole thread.
        self.sync_persist_cache(&items);
        cx.background_spawn(async move {
            if let Err(err) = store.save_items(&id, &items) {
                log::warn!("ThreadView: persist_snapshot_now failed: {err}");
            }
        })
        .detach();
    }

    fn collect_persisted_items(&self, _cx: &gpui::App) -> Vec<PersistedThreadItem> {
        // US-022 (cli-hardening-followup-2026-Q3): delegate to the
        // pure free helper so the cache-skip behaviour can be unit
        // tested without constructing a full `ThreadView` (no GPUI
        // `Context` required). Per-item cache: a `None` slot means
        // the corresponding item changed since the last persist;
        // rebuild only those slots. On a 200-item thread with 2
        // streaming items per persist tick, this drops the
        // per-tick allocation count from ~200 fresh `String`/`Vec`
        // clones down to ~2.
        //
        // This function still takes `&self` (no `&mut`) so we
        // cannot write back into `persist_cache_items` from here
        // -- the caller (`persist_snapshot_now`) calls
        // `Self::sync_persist_cache` once it has a `&mut self`.
        build_persisted_from_cache(
            &self.items,
            &self.persist_cache_items,
            &self.reviewed_edits,
            &self.tool_call_index,
        )
    }

    /// US-022 (cli-hardening-followup-2026-Q3): write the
    /// just-collected persisted forms back into the per-item cache.
    /// Called from `persist_snapshot_now` once it has a `&mut self`
    /// handle. Safe to call when lengths drift -- the helper resets
    /// the cache vector to match `self.items.len()` first.
    fn sync_persist_cache(&mut self, fresh: &[PersistedThreadItem]) {
        // The fresh slice is built by `collect_persisted_items`
        // which skips assistant items whose `chunks` are empty (it
        // returns `None` for those). So `fresh.len() <= items.len()`.
        // We need to map fresh entries back to item indices for the
        // cache. Walk both in lockstep, skipping empty-assistant
        // items.
        let mut new_cache: Vec<Option<PersistedThreadItem>> = vec![None; self.items.len()];
        let mut fresh_iter = fresh.iter();
        for (idx, item) in self.items.iter().enumerate() {
            if let ThreadItem::AssistantMessage(am) = item
                && am.chunks.is_empty()
            {
                continue;
            }
            if let Some(p) = fresh_iter.next() {
                new_cache[idx] = Some(p.clone());
            }
        }
        self.persist_cache_items = new_cache;
    }

    /// US-022 (cli-hardening-followup-2026-Q3): mark the cache slot
    /// at `idx` as dirty. Called by every in-place mutation site on
    /// the hot streaming path; the next `collect_persisted_items`
    /// call rebuilds only that slot.
    fn mark_persist_dirty(&mut self, idx: usize) {
        if let Some(slot) = self.persist_cache_items.get_mut(idx) {
            *slot = None;
        }
    }

    /// US-022 (cli-hardening-followup-2026-Q3): blow the entire
    /// cache. Called by sweeping mutations (truncate_for_edit,
    /// keep_all_edits, reject_all_edits, sweep_pending_tools_*) so
    /// stale cached forms can never serve a wrong persisted blob.
    /// Correctness over allocation savings on rare paths.
    fn invalidate_persist_cache_all(&mut self) {
        for slot in self.persist_cache_items.iter_mut() {
            *slot = None;
        }
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
        log::debug!(
            target: "agents::stream",
            "begin_assistant_stream prev_idx={:?} pending_chars={} items_len={}",
            self.streaming_message_idx,
            self.streaming_buffer.pending_chars(),
            self.items.len(),
        );
        if self.streaming_message_idx.is_some() {
            self.flush_streaming(cx);
        }
        // Re-use the trailing assistant message if there is one
        // (e.g. a Thought chunk landed first and we want the text to
        // append into the same turn). Otherwise open a fresh one.
        // We no longer seed an empty `Text` chunk here — if the first
        // content for the turn is a tool call (or nothing at all), an
        // empty seed would persist as a phantom row in the timeline
        // and on disk. `append_to_streaming_text_at` already opens a
        // fresh Text chunk on the first real append.
        let idx = self.ensure_open_assistant_message(cx);
        self.streaming_message_idx = Some(idx);
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        cx.notify();

        // Adaptive streaming cadence: a Cell shared between the loop
        // and the per-tick update so the next sleep can shrink/grow
        // based on the open message's current size. Reduces
        // `markdown::Markdown::append` calls on long responses without
        // changing perceived smoothness for short ones. Default starts
        // at FAST -- short prefixes stay snappy.
        let next_tick: Rc<Cell<Duration>> = Rc::new(Cell::new(STREAMING_TICK_FAST));
        let next_tick_for_loop = Rc::clone(&next_tick);
        self._streaming_task = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                loop {
                    smol::Timer::after(next_tick_for_loop.get()).await;
                    let outcome = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            // Drain any scheduled throttled persist
                            // (Zed parity: `schedule_save` batching).
                            view.tick_persist(cx);
                            if view.streaming_message_idx.is_none() {
                                return false;
                            }
                            let revealed = view.streaming_buffer.tick();
                            if !revealed.is_empty() {
                                view.append_to_streaming_text(&revealed, cx);
                            }
                            // US-008: same paced reveal for thinking
                            // chunks. Routing through the buffer
                            // coalesces the per-token `cx.notify` storm
                            // a Sonnet thinking burst would otherwise
                            // produce.
                            let revealed_thought = view.thinking_buffer.tick();
                            if !revealed_thought.is_empty() {
                                view.append_to_streaming_thought(&revealed_thought, cx);
                            }
                            // Adapt the next sleep to the open message
                            // size: past ~4 KB drop to 20 Hz, past
                            // ~16 KB drop to ~7 Hz. Keeps the cumulative
                            // O(n^2) markdown.append cost bounded over
                            // a long response without freezing the UI.
                            next_tick
                                .set(adaptive_streaming_tick(view.open_assistant_message_chars()));
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

    /// Total character count across the currently-open assistant
    /// message's text + thought chunks. Used by the streaming task to
    /// decide its next tick cadence ([`adaptive_streaming_tick`]).
    /// `None`-indexed `streaming_message_idx` returns 0.
    fn open_assistant_message_chars(&self) -> usize {
        let Some(idx) = self.streaming_message_idx else {
            return 0;
        };
        match self.items.get(idx) {
            Some(ThreadItem::AssistantMessage(am)) => am
                .chunks
                .iter()
                .map(|c| match c {
                    AssistantMessageChunk::Text { text, .. } => text.len(),
                    AssistantMessageChunk::Thought { text, .. } => text.len(),
                })
                .sum(),
            _ => 0,
        }
    }

    pub fn push_streaming_chunk(&mut self, chunk: &str, _cx: &mut Context<Self>) {
        if chunk.is_empty() {
            return;
        }
        log::trace!(
            target: "agents::stream",
            "push_streaming_chunk len={} pending_before={} streaming_idx={:?}",
            chunk.len(),
            self.streaming_buffer.pending_chars(),
            self.streaming_message_idx,
        );
        self.streaming_buffer.push(chunk);
        // No immediate tick on push:
        // `markdown::Markdown::append` rebuilds the full `SharedString`
        // source on every call (Zed `crates/markdown/src/markdown.rs:588`
        // -- `self.source = SharedString::new(self.source.to_string() +
        // text)`). Calling that per arriving chunk turns a long
        // response into O(n²) string allocations. The 16 ms streaming
        // tick in `begin_assistant_stream` keeps reveals at frame
        // cadence (at most 60 Hz of markdown.append calls), which is
        // already the user-visible refresh ceiling -- pushing harder
        // burns CPU without changing what the user can see. First-
        // token latency is still one frame because the runtime event
        // task wakes immediately via futures::mpsc and the streaming
        // tick fires within at most 16 ms of the push.
    }

    pub fn flush_streaming(&mut self, cx: &mut Context<Self>) {
        log::debug!(
            target: "agents::stream",
            "flush_streaming idx={:?} pending_chars={}",
            self.streaming_message_idx,
            self.streaming_buffer.pending_chars(),
        );
        // Drain any thinking still queued in the paced buffer BEFORE
        // dropping the open-message handle. Otherwise a fast turn end
        // that arrives between two ticks would strand the trailing
        // thought characters in the buffer until `clear_local_display`
        // wipes them. Symmetric with the text-side flush below.
        if !self.thinking_buffer.is_idle() {
            let remaining_thought = self.thinking_buffer.flush();
            if !remaining_thought.is_empty() {
                self.append_to_streaming_thought(&remaining_thought, cx);
            }
        }
        let Some(idx) = self.streaming_message_idx.take() else {
            return;
        };
        let remaining = self.streaming_buffer.flush();
        if !remaining.is_empty() {
            self.append_to_streaming_text_at(idx, &remaining, cx);
        }
        // Skip persisting if the assistant message at `idx` is still
        // entirely empty (no text chunks beyond the seed, no tool calls
        // attached). This happens when `begin_assistant_stream` seeded
        // an empty Text chunk and the turn was cancelled before any
        // content arrived — persisting the empty message would clutter
        // the on-disk thread with phantom rows.
        let assistant_is_empty = self.items.get(idx).is_some_and(|it| match it {
            ThreadItem::AssistantMessage(am) => am.chunks.iter().all(|c| match c {
                AssistantMessageChunk::Text { text, .. } => text.is_empty(),
                AssistantMessageChunk::Thought { text, .. } => text.is_empty(),
            }),
            _ => false,
        });
        // The forced flush superseeds any pending throttled write.
        self.persist_deadline = None;
        if !assistant_is_empty {
            self.persist_snapshot_now(cx);
        }
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

    /// Apply a revealed thinking batch to the currently-open Thought
    /// chunk. Mirrors [`Self::append_to_streaming_text`] but routes
    /// to the trailing `Thought` instead of the trailing `Text`.
    /// Idempotent when no streaming Thought is open.
    fn append_to_streaming_thought(&mut self, chunk: &str, cx: &mut Context<Self>) {
        let Some((am_idx, chunk_idx)) = self.streaming_thinking_key else {
            return;
        };
        let Some(ThreadItem::AssistantMessage(am)) = self.items.get_mut(am_idx) else {
            return;
        };
        let Some(AssistantMessageChunk::Thought { text, markdown, .. }) =
            am.chunks.get_mut(chunk_idx)
        else {
            return;
        };
        text.push_str(chunk);
        markdown.update(cx, |m, cx| m.append(chunk, cx));
        if self.should_be_following.get() {
            self.list_state.scroll_to_end();
        }
        cx.notify();
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
        // US-022: this streaming item's text just changed; the next
        // persist tick must re-serialise only this slot, not all 200
        // items.
        self.mark_persist_dirty(idx);
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
        if let Some(idx) = self.streaming_message_idx {
            log::debug!(
                target: "agents::stream",
                "close_open_assistant_message idx={} pending_chars={}",
                idx,
                self.streaming_buffer.pending_chars(),
            );
        }
        self.streaming_message_idx = None;
    }

    /// Drain the paced streaming buffer into the currently-open
    /// assistant message's trailing `Text` chunk, **without** closing
    /// the pipeline. Called from every path that's about to interrupt
    /// an ongoing text stream with a non-text item (tool call,
    /// thinking burst, user message) so the pending chars are revealed
    /// in chronological order inside the same assistant turn.
    ///
    /// No-op when `streaming_message_idx` is `None` (nothing to drain
    /// into) or when the buffer is empty.
    fn drain_streaming_buffer_into_open(&mut self, cx: &mut Context<Self>) {
        // Drain the thinking buffer first so a Thought that arrived
        // just before this drain (and is still queued) renders ahead
        // of any text that follows.
        if !self.thinking_buffer.is_idle() {
            let remaining = self.thinking_buffer.flush();
            if !remaining.is_empty() {
                self.append_to_streaming_thought(&remaining, cx);
            }
        }
        let Some(idx) = self.streaming_message_idx else {
            return;
        };
        if self.streaming_buffer.is_idle() {
            return;
        }
        let remaining = self.streaming_buffer.flush();
        if remaining.is_empty() {
            return;
        }
        log::debug!(
            target: "agents::stream",
            "drain_streaming_buffer_into_open idx={} drained_bytes={}",
            idx,
            remaining.len(),
        );
        self.append_to_streaming_text_at(idx, &remaining, cx);
    }

    // -----------------------------------------------------------------
    // Inline tool-call burst grouping
    // -----------------------------------------------------------------

    /// Return the indices of every consecutive inline burst tool
    /// starting at `start_ix`. The burst extends to the right while
    /// `is_burst_tool` holds. Empty when `start_ix` itself is not a
    /// burst tool.
    fn collect_burst_at(&self, start_ix: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut ix = start_ix;
        while let Some(item) = self.items.get(ix) {
            if !is_burst_tool(item) {
                break;
            }
            out.push(ix);
            ix += 1;
        }
        out
    }

    /// `true` when the item at `ix` is the first item of a burst
    /// (i.e. an inline burst tool whose predecessor is NOT a burst
    /// tool). The header row is rendered at the burst-start index;
    /// subsequent items collapse to a zero-height row.
    fn is_burst_start(&self, ix: usize) -> bool {
        if !self.items.get(ix).is_some_and(is_burst_tool) {
            return false;
        }
        ix == 0 || !self.items.get(ix - 1).is_some_and(is_burst_tool)
    }

    /// Effective expand state for the burst at `start_ix`. A
    /// `WaitingForConfirmation` tool forces open regardless of the
    /// user override; otherwise the explicit override (if any) wins,
    /// falling back to `has_non_terminal` (open while a tool is still
    /// running, closed once every tool reached a terminal state).
    fn tool_group_is_expanded(
        &self,
        start_ix: usize,
        has_non_terminal: bool,
        has_awaiting: bool,
    ) -> bool {
        if has_awaiting {
            return true;
        }
        if let Some(&user) = self.tool_group_user_open.get(&start_ix) {
            return user;
        }
        has_non_terminal
    }

    /// Toggle the user-forced expand state for the burst that starts
    /// at `start_ix`. Writes the inverse of the current effective
    /// state so a click always flips what the user sees.
    pub fn toggle_tool_group(&mut self, start_ix: usize, cx: &mut Context<Self>) {
        let burst = self.collect_burst_at(start_ix);
        let mut has_non_terminal = false;
        let mut has_awaiting = false;
        for &ix in &burst {
            if let Some(ThreadItem::ToolCall(s)) = self.items.get(ix) {
                match s.status {
                    super::runtime::ToolCallStatusKind::WaitingForConfirmation => {
                        has_awaiting = true;
                        has_non_terminal = true;
                    }
                    super::runtime::ToolCallStatusKind::Pending
                    | super::runtime::ToolCallStatusKind::InProgress => {
                        has_non_terminal = true;
                    }
                    _ => {}
                }
            }
        }
        let current = self.tool_group_is_expanded(start_ix, has_non_terminal, has_awaiting);
        // A WaitingForConfirmation burst stays force-open; flipping
        // would be a no-op since the next render forces it back to
        // true. Skip the write so we don't accumulate stale entries.
        if has_awaiting {
            return;
        }
        self.tool_group_user_open.insert(start_ix, !current);
        cx.notify();
    }

    fn render_tool_call_group(
        &mut self,
        start_ix: usize,
        ui: crate::theme::UiColors,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::runtime::ToolCallStatusKind;
        use ui::{LabelCommon, LabelSize, SpinnerLabel};

        let burst = self.collect_burst_at(start_ix);
        if burst.is_empty() {
            return div().into_any_element();
        }
        let snaps: Vec<Arc<ToolCallSnapshot>> = burst
            .iter()
            .filter_map(|&ix| match self.items.get(ix) {
                Some(ThreadItem::ToolCall(s)) => Some(Arc::clone(s)),
                _ => None,
            })
            .collect();
        let count = snaps.len();

        // State derivation. `active` is the last non-terminal tool in
        // the burst (the one currently running, used as the header
        // command). `awaiting` is the first WaitingForConfirmation
        // (force-open + amber). `failed_count` triggers the red
        // "N failed" suffix once every tool reached a terminal state.
        let active = snaps.iter().rev().find(|s| {
            matches!(
                s.status,
                ToolCallStatusKind::Pending | ToolCallStatusKind::InProgress
            )
        });
        let awaiting = snaps
            .iter()
            .find(|s| matches!(s.status, ToolCallStatusKind::WaitingForConfirmation));
        let failed_count = snaps
            .iter()
            .filter(|s| {
                matches!(
                    s.status,
                    ToolCallStatusKind::Failed | ToolCallStatusKind::Canceled
                )
            })
            .count();
        let has_non_terminal = active.is_some() || awaiting.is_some();
        let has_awaiting = awaiting.is_some();
        let expanded = self.tool_group_is_expanded(start_ix, has_non_terminal, has_awaiting);

        // Header text. Strip backticks from the active title so the
        // path / command reads cleanly on a single line; markdown chip
        // styling lives in the expanded body where each tool is
        // rendered through `render_inline_tool_call`.
        let header_text: String = if let Some(a) = awaiting {
            format!("Awaiting Confirmation: {}", strip_backticks(&a.title))
        } else if let Some(a) = active {
            format!("Working · {}", strip_backticks(&a.title))
        } else if failed_count > 0 {
            if failed_count == count {
                format!("Used {count} tools · all failed")
            } else {
                format!("Used {count} tools · {failed_count} failed")
            }
        } else {
            let suffix = if count == 1 { "tool" } else { "tools" };
            format!("Used {count} {suffix}")
        };

        let amber: gpui::Hsla = rgb(0xeab676).into();
        // Dim terracotta -- reserved for the catastrophic case where
        // every tool in the burst failed. A single failure inside an
        // otherwise-completed burst stays muted with a discrete
        // textual suffix so it doesn't dominate the row (1 fail out
        // of 26 in red was visually disproportionate).
        let red_dim: gpui::Hsla = rgb(0xc97c5e).into();
        let all_failed = failed_count > 0 && failed_count == count;
        let header_color: gpui::Hsla = if has_awaiting {
            amber
        } else if has_non_terminal {
            ui.text
        } else if all_failed {
            red_dim
        } else {
            ui.muted
        };

        // Status glyph. Spinner while running, amber loader on
        // permission prompts, terracotta dot only on a full-failure
        // burst; partial failures keep the muted check and let the
        // textual suffix carry the count.
        let status_glyph: AnyElement = if has_awaiting {
            gpui::svg()
                .size(px(11.))
                .flex_none()
                .path("icons/loader-circle.svg")
                .text_color(amber)
                .into_any_element()
        } else if has_non_terminal {
            SpinnerLabel::dots()
                .size(LabelSize::Small)
                .into_any_element()
        } else if all_failed {
            div()
                .w(px(8.))
                .h(px(8.))
                .rounded_full()
                .bg(red_dim)
                .into_any_element()
        } else {
            gpui::svg()
                .size(px(11.))
                .flex_none()
                .path("icons/check.svg")
                .text_color(ui.muted)
                .into_any_element()
        };

        let chevron_path = if expanded {
            "icons/chevron-down.svg"
        } else {
            "icons/chevron-right.svg"
        };
        let chevron = gpui::svg()
            .size(px(11.))
            .flex_none()
            .path(chevron_path)
            .text_color(ui.muted);

        let count_chip: SharedString = format!("{count}").into();
        let header_label: SharedString = header_text.into();
        let header_id: SharedString = format!("tool-group-header-{start_ix}").into();

        // Inset the hover surface from the panel edges via `mx` so
        // the rounded pill reads as a tappable list item instead of
        // an edge-to-edge stripe. Inner `px` keeps the chevron lined
        // up with the assistant-message body (~20 px total).
        let header = div()
            .id(header_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .mx(px(12.))
            .px(px(8.))
            .py(px(4.))
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(ui.subtle))
            .on_click(cx.listener(move |this, _ev, _w, cx| {
                this.toggle_tool_group(start_ix, cx);
            }))
            .child(chevron)
            .child(status_glyph)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_color(header_color)
                    .text_size(px(12.))
                    .child(header_label),
            )
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(11.))
                    .child(count_chip),
            );

        if !expanded {
            return header.into_any_element();
        }

        // Expanded body: re-render each tool through the existing
        // `render_inline_tool_call` helper. No left border or vertical
        // rule -- the indented row alignment + the rounded header
        // pill make the cluster cohesive without extra chrome.
        // Callbacks duplicate the `render_item` wiring (toggle expand
        // of the individual row, permission decisions, pattern picker).
        let mut body = div().flex().flex_col().pl(px(28.)).pb(px(4.));

        for &ix in &burst {
            let snap = match self.items.get(ix) {
                Some(ThreadItem::ToolCall(s)) => Arc::clone(s),
                _ => continue,
            };
            let label_md = self.tool_label_markdown.get(&snap.id).cloned();
            let entity_weak = cx.entity().downgrade();
            let id_for_toggle = snap.id.clone();
            let id_for_perm = snap.id.clone();
            let id_for_picker = snap.id.clone();
            let id_for_pattern = snap.id.clone();
            let on_toggle = move |_ev: &ClickEvent, _w: &mut gpui::Window, cx: &mut gpui::App| {
                if let Some(entity) = entity_weak.upgrade() {
                    let id = id_for_toggle.clone();
                    entity.update(cx, |this, cx| {
                        this.toggle_tool_call_expanded(&id, cx);
                    });
                }
            };
            let entity_weak = cx.entity().downgrade();
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
            let elem = super::inline_tool_call::render_inline_tool_call(
                ix,
                &snap,
                label_md,
                ui,
                w,
                cx,
                on_toggle,
                on_permission,
                on_toggle_picker,
                on_apply_pattern,
            );
            body = body.child(elem);
        }

        div()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }

    // -----------------------------------------------------------------
    // Render
    // -----------------------------------------------------------------

    fn render_item(&mut self, ix: usize, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Reuse the theme + ui snapshot captured in `render()` so this
        // function — invoked once per visible item by GPUI's List
        // virtualizer — does not re-lock the theme cache. Fallback to
        // the lock path on the cold first call before `render` runs.
        let ui = self._ui_snapshot.unwrap_or_else(crate::theme::ui_colors);
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
                let cwd = self._cwd_snapshot.clone();
                let bubble = super::message_render::render_message_body(
                    role,
                    &um.msg.content,
                    md_entity,
                    ui,
                    cwd,
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
                // Inline burst grouping: a Read / Search / Execute /
                // Think / Fetch / Other tool call is rendered as part
                // of a collapsible group with its contiguous neighbors
                // of the same kind. The header is rendered at the
                // burst start; subsequent items in the burst collapse
                // to a zero-height row so the GPUI virtual list count
                // stays consistent without painting duplicate chrome.
                // Edit / Delete / Move (diff-bearing) tools keep their
                // own card layout below.
                if is_burst_tool_snap(snap) {
                    return if self.is_burst_start(ix) {
                        self.render_tool_call_group(ix, ui, w, cx)
                    } else {
                        div().h(px(0.)).into_any_element()
                    };
                }
                // Cheap pointer copy of the Arc -- no struct + string
                // clone. The actual ToolCallSnapshot is shared with
                // `self.items` and never mutated by the render path.
                let snap = Arc::clone(snap);
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
                        &snap,
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
                        &snap,
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
        // Use the per-render cwd snapshot (US-016) so this hot path
        // does not pay a per-chunk PathBuf clone for a value that is
        // constant across the render pass.
        let cwd = self._cwd_snapshot.clone();
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
                        cwd.clone(),
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
        // The earlier `py(10)` bump made text-around-tool-calls read as
        // visually disjoint bubbles -- two AssistantMessage entries
        // surrounding a ToolCall stacked 10+10=20px of vertical space
        // plus tool padding, which the eye reads as "new bubble" each
        // time the assistant resumes after a tool. Zed's 6px keeps the
        // text-tool-text sequence tight enough that the post-tool text
        // looks like a continuation of the same turn (see the
        // screenshot pattern: "...scanner v" -> [tools] -> "ite fait
        // la structure"). Match Zed's value verbatim.
        let mut outer = div()
            .flex()
            .flex_col()
            .w_full()
            .px(px(20.))
            .py(px(6.))
            .text_size(px(14.));
        if is_last {
            outer = outer.pb(px(16.));
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

/// `true` when `item` is an inline tool call eligible for burst
/// grouping: a Read / Search / Execute / Think / Fetch / Other /
/// SwitchMode tool with no attached diffs. Diff-bearing or Edit /
/// Delete / Move tools render through their own card layout and
/// break the burst when they appear in the timeline.
fn is_burst_tool(item: &ThreadItem) -> bool {
    match item {
        ThreadItem::ToolCall(snap) => is_burst_tool_snap(snap),
        _ => false,
    }
}

fn is_burst_tool_snap(snap: &ToolCallSnapshot) -> bool {
    !matches!(
        snap.kind,
        ToolKindKind::Edit | ToolKindKind::Delete | ToolKindKind::Move,
    ) && snap.diffs.is_empty()
}

/// Strip backticks from a tool title so it renders cleanly inline
/// inside a single-line header. The markdown title is preserved in
/// the expanded body where `Markdown` renders the chip styling.
fn strip_backticks(title: &str) -> String {
    title.replace('`', "")
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
        // Snapshot the theme once per render: every helper below derives
        // its colors from this value rather than re-locking the global
        // theme cache. `ui_colors` used to lock internally on every
        // call, and the agents view called it from both the outer
        // render and each visible item's render_item — under streaming
        // that's O(visible_items + 1) lock acquisitions per frame. The
        // `_theme_snapshot` lives on `self` so `render_item` (driven by
        // the virtualized List) can read it without a second lock.
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors_with(&theme);
        self._theme_snapshot = Some(theme);
        self._ui_snapshot = Some(ui);
        // US-016 (audit P2-4): snapshot the composer cwd once per
        // render. Previously `render_item` cloned it as a PathBuf per
        // visible message; on a viewport with ~20 visible items that
        // was 20 PathBuf allocs per frame for no semantic value (cwd
        // does not change mid-render).
        self._cwd_snapshot = self
            .composer
            .as_ref()
            .map(|c| std::sync::Arc::from(c.read(cx).cwd()));
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

        let theme = self
            ._theme_snapshot
            .unwrap_or_else(crate::theme::active_theme);
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

/// US-022 (cli-hardening-followup-2026-Q3): pure-function core of
/// `ThreadView::collect_persisted_items`. Walks `items` in lockstep
/// with `cache`; a `Some` slot is reused verbatim (modulo the
/// per-tool-call `reviewed` flag, which lives on the view -- not
/// the snapshot -- and can flip without bumping the item itself),
/// a `None` slot is rebuilt via [`serialize_thread_item`]. Extracted
/// as a free function so the cache-skip behaviour can be exercised
/// in a unit test without constructing a full `ThreadView` (no GPUI
/// `Context` available in plain `#[test]` harness).
fn build_persisted_from_cache(
    items: &[ThreadItem],
    cache: &[Option<PersistedThreadItem>],
    reviewed_edits: &HashSet<String>,
    tool_call_index: &HashMap<String, usize>,
) -> Vec<PersistedThreadItem> {
    let mut out: Vec<PersistedThreadItem> = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        if let Some(Some(persisted)) = cache.get(idx) {
            if let PersistedThreadItem::Tool(t) = persisted {
                let want_reviewed = tool_call_index
                    .iter()
                    .find_map(|(id, &i)| (i == idx).then_some(id.clone()))
                    .map(|id| reviewed_edits.contains(&id))
                    .unwrap_or(false);
                if t.reviewed == want_reviewed {
                    out.push(persisted.clone());
                    continue;
                }
                // Fall through to rebuild when `reviewed` flips.
            } else {
                out.push(persisted.clone());
                continue;
            }
        }
        if let Some(rebuilt) = serialize_thread_item(item, reviewed_edits) {
            out.push(rebuilt);
        }
    }
    out
}

/// US-022 (cli-hardening-followup-2026-Q3): build the persisted
/// form for a single `ThreadItem`. Extracted from the body of
/// `ThreadView::collect_persisted_items` so the per-item cache can
/// call it lazily for the slots flagged dirty. Returns `None` for
/// an empty `AssistantMessage` (matches the pre-cache filter
/// semantics; an empty assistant turn never makes it to disk).
fn serialize_thread_item(
    item: &ThreadItem,
    reviewed_edits: &HashSet<String>,
) -> Option<PersistedThreadItem> {
    match item {
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
            persisted.reviewed = reviewed_edits.contains(&snap.id);
            Some(PersistedThreadItem::Tool(persisted))
        }
    }
}

/// US-004 (cli-hardening-followup-2026-Q3): apply the
/// terminal-status prune to the two per-tool-call UI maps. Decision
/// table:
///
/// | terminal? | has_diffs | label removed | scroll removed |
/// |-----------|-----------|---------------|----------------|
/// | no        | -         | no            | no             |
/// | yes       | yes       | no (kept for header on re-expand) | yes |
/// | yes       | no        | yes           | yes            |
///
/// Extracted as a free function so the unit test below can exercise
/// it without a `Context<ThreadView>`. Generic on the map value
/// types so the test can pass cheap placeholders (`String` / `()`)
/// instead of having to construct `Entity<Markdown>` and
/// `gpui::ScrollHandle` from a test harness.
fn apply_terminal_tool_ui_prune<L, S>(
    label_map: &mut HashMap<String, L>,
    scroll_map: &mut HashMap<String, S>,
    id: &str,
    is_terminal: bool,
    has_diffs: bool,
) {
    if !is_terminal {
        return;
    }
    scroll_map.remove(id);
    if !has_diffs {
        label_map.remove(id);
    }
}

/// US-001 (cli-hardening-followup-2026-Q3): free `DiffSnapshot.old_text`
/// on every diff of a reviewed snapshot and record the original
/// line count in `cleared_diff_lines` so the renderer can show a
/// `[diff body cleared after review, N lines]` placeholder instead
/// of computing a `[]` vs `new_text` diff. Returns the number of
/// diffs that were actually mutated (idempotent: already-cleared
/// diffs are skipped).
///
/// Extracted as a free function (rather than a method) so the unit
/// test in this module can exercise it without constructing a full
/// `ThreadView` + GPUI `Context`.
fn clear_reviewed_diff_bodies(snap: &mut ToolCallSnapshot) -> usize {
    let mut cleared = 0usize;
    for diff in snap.diffs.iter_mut() {
        if diff.cleared_diff_lines.is_some() {
            continue;
        }
        let count = super::edit_tool_block::diff_line_count(
            diff.old_text.as_deref().unwrap_or(""),
            &diff.new_text,
        );
        diff.cleared_diff_lines = Some(count as u32);
        diff.old_text = None;
        cleared += 1;
    }
    cleared
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

    /// US-004: short messages stay at the 60 Hz FAST cadence.
    #[test]
    fn adaptive_streaming_tick_short_message_uses_fast_cadence() {
        assert_eq!(adaptive_streaming_tick(0), STREAMING_TICK_FAST);
        assert_eq!(adaptive_streaming_tick(100), STREAMING_TICK_FAST);
        assert_eq!(
            adaptive_streaming_tick(STREAMING_ADAPT_MEDIUM - 1),
            STREAMING_TICK_FAST,
        );
    }

    /// US-004: past the medium threshold the tick drops to ~20 Hz.
    #[test]
    fn adaptive_streaming_tick_medium_message_uses_medium_cadence() {
        assert_eq!(
            adaptive_streaming_tick(STREAMING_ADAPT_MEDIUM),
            STREAMING_TICK_MEDIUM,
        );
        assert_eq!(
            adaptive_streaming_tick(STREAMING_ADAPT_LONG - 1),
            STREAMING_TICK_MEDIUM,
        );
    }

    /// US-004: past the long threshold the tick drops to ~7 Hz so
    /// the cumulative O(n^2) markdown.append cost stays bounded.
    #[test]
    fn adaptive_streaming_tick_long_message_uses_slow_cadence() {
        assert_eq!(
            adaptive_streaming_tick(STREAMING_ADAPT_LONG),
            STREAMING_TICK_SLOW,
        );
        assert_eq!(adaptive_streaming_tick(100_000), STREAMING_TICK_SLOW);
    }

    fn synthetic_tool_call_snap(id: &str) -> Arc<super::ToolCallSnapshot> {
        use super::super::runtime::{ToolCallStatusKind, ToolKindKind};
        Arc::new(super::ToolCallSnapshot {
            id: id.to_string(),
            title: format!("Tool {id}"),
            kind: ToolKindKind::Other,
            status: ToolCallStatusKind::Completed,
            raw_input_pretty: None,
            raw_output_pretty: None,
            content_text: String::new(),
            diffs: Vec::new(),
            expanded: false,
            permission_options: Vec::new(),
            permission_picker_open: false,
            locations: Vec::new(),
        })
    }

    /// US-006 (review follow-up): the index-derived state in
    /// [`super::compute_activity_state`] must always agree with a
    /// from-scratch scan of [`ThreadView::items`]. This test pins the
    /// pure-function half of that invariant -- the production debug-
    /// assert called from each mutation site catches drift at runtime.
    #[test]
    fn expected_tool_call_indices_filters_only_tool_call_variants() {
        use super::{AssistantMessage, ThreadItem, UserMessage, expected_tool_call_indices};
        use paneflow_threads::message::Message;

        // Mixed sequence: user / tool / assistant / tool / tool / user.
        // The expected indices are the ToolCall positions: [1, 3, 4].
        let items = vec![
            ThreadItem::UserMessage(UserMessage {
                msg: Message::user_text("hello"),
                markdown: None,
            }),
            ThreadItem::ToolCall(synthetic_tool_call_snap("a")),
            ThreadItem::AssistantMessage(AssistantMessage { chunks: Vec::new() }),
            ThreadItem::ToolCall(synthetic_tool_call_snap("b")),
            ThreadItem::ToolCall(synthetic_tool_call_snap("c")),
            ThreadItem::UserMessage(UserMessage {
                msg: Message::user_text("bye"),
                markdown: None,
            }),
        ];
        assert_eq!(expected_tool_call_indices(&items), vec![1, 3, 4]);
    }

    /// US-006 (review follow-up): a sequence with no tool calls
    /// produces an empty index vector -- the activity bar correctly
    /// short-circuits to None in that case.
    #[test]
    fn expected_tool_call_indices_empty_when_no_tool_calls() {
        use super::{AssistantMessage, ThreadItem, UserMessage, expected_tool_call_indices};
        use paneflow_threads::message::Message;
        let items = vec![
            ThreadItem::UserMessage(UserMessage {
                msg: Message::user_text("hi"),
                markdown: None,
            }),
            ThreadItem::AssistantMessage(AssistantMessage { chunks: Vec::new() }),
        ];
        assert!(expected_tool_call_indices(&items).is_empty());
    }

    /// US-006 (review follow-up): a sequence that's *all* tool calls
    /// yields a dense range -- truncate_for_edit and clear_local_display
    /// land here when the head of a thread is a tool-only burst.
    #[test]
    fn expected_tool_call_indices_dense_sequence() {
        use super::{ThreadItem, expected_tool_call_indices};
        let items = vec![
            ThreadItem::ToolCall(synthetic_tool_call_snap("a")),
            ThreadItem::ToolCall(synthetic_tool_call_snap("b")),
            ThreadItem::ToolCall(synthetic_tool_call_snap("c")),
        ];
        assert_eq!(expected_tool_call_indices(&items), vec![0, 1, 2]);
    }

    /// US-004 (cli-hardening-followup-2026-Q3): a non-edit
    /// tool call that transitions to Completed must release BOTH
    /// the label markdown entity and the scroll handle -- there is
    /// no Keep All / Reject All path for it. Edit calls keep the
    /// label (the header path is re-read on re-expansion) and only
    /// drop the live scroll handle.
    #[test]
    fn non_edit_tool_pruned_on_completion() {
        use super::apply_terminal_tool_ui_prune;
        use std::collections::HashMap;
        let mut labels: HashMap<String, &'static str> = HashMap::from([
            ("non-edit".to_string(), "Read file Cargo.toml"),
            ("edit".to_string(), "Edit src/main.rs"),
        ]);
        let mut scrolls: HashMap<String, ()> =
            HashMap::from([("non-edit".to_string(), ()), ("edit".to_string(), ())]);

        // Non-edit terminal: label + scroll both removed.
        apply_terminal_tool_ui_prune(
            &mut labels,
            &mut scrolls,
            "non-edit",
            /*is_terminal=*/ true,
            /*has_diffs=*/ false,
        );
        assert!(!labels.contains_key("non-edit"));
        assert!(!scrolls.contains_key("non-edit"));

        // Edit terminal: label kept, scroll removed.
        apply_terminal_tool_ui_prune(
            &mut labels,
            &mut scrolls,
            "edit",
            /*is_terminal=*/ true,
            /*has_diffs=*/ true,
        );
        assert!(labels.contains_key("edit"));
        assert!(!scrolls.contains_key("edit"));

        // Non-terminal: no-op regardless of has_diffs.
        labels.insert("pending".to_string(), "Pending tool");
        scrolls.insert("pending".to_string(), ());
        apply_terminal_tool_ui_prune(
            &mut labels,
            &mut scrolls,
            "pending",
            /*is_terminal=*/ false,
            /*has_diffs=*/ true,
        );
        assert!(labels.contains_key("pending"));
        assert!(scrolls.contains_key("pending"));
    }

    /// US-001 (cli-hardening-followup-2026-Q3): a Keep / Reject
    /// review must free `DiffSnapshot.old_text` on every diff and
    /// record the original diff-line count in `cleared_diff_lines`
    /// so the renderer can show a static placeholder instead of
    /// re-diffing on re-expansion. Idempotent: a second call on the
    /// same snapshot returns 0 cleared.
    #[test]
    fn purge_reviewed_diffs_clears_old_text() {
        use super::super::runtime::{
            DiffSnapshot, ToolCallSnapshot, ToolCallStatusKind, ToolKindKind,
        };
        use super::clear_reviewed_diff_bodies;

        let mut snap = ToolCallSnapshot {
            id: "tc-1".to_string(),
            title: "Edit src/main.rs".to_string(),
            kind: ToolKindKind::Edit,
            status: ToolCallStatusKind::Completed,
            raw_input_pretty: None,
            raw_output_pretty: None,
            content_text: String::new(),
            diffs: vec![
                DiffSnapshot {
                    path: std::path::PathBuf::from("src/main.rs"),
                    old_text: Some("a\nb\nc\n".to_string()),
                    new_text: "a\nB\nc\n".to_string(),
                    cleared_diff_lines: None,
                },
                DiffSnapshot {
                    path: std::path::PathBuf::from("src/lib.rs"),
                    old_text: Some("foo\n".to_string()),
                    new_text: "foo\nbar\n".to_string(),
                    cleared_diff_lines: None,
                },
            ],
            expanded: false,
            permission_options: Vec::new(),
            permission_picker_open: false,
            locations: Vec::new(),
        };

        let cleared = clear_reviewed_diff_bodies(&mut snap);
        assert_eq!(cleared, 2);
        for diff in &snap.diffs {
            assert!(diff.old_text.is_none(), "old_text must be None post-clear");
            assert!(
                diff.cleared_diff_lines.is_some(),
                "cleared_diff_lines must be set"
            );
            // The recorded count must be > 0 (each diff has at least
            // one changed line).
            assert!(diff.cleared_diff_lines.unwrap() > 0);
        }
        // Idempotence: a second pass mutates nothing.
        assert_eq!(clear_reviewed_diff_bodies(&mut snap), 0);
    }

    /// US-022 (cli-hardening-followup-2026-Q3): the per-item cache
    /// must skip re-serialising slots flagged clean. Primes the
    /// cache with a sentinel persisted form whose text does NOT
    /// appear in the corresponding live item; the assertion is that
    /// the sentinel survives untouched (cache hit) while the dirty
    /// slot rebuilds from the live item (cache miss).
    #[test]
    fn collect_persisted_items_skips_clean_items() {
        use super::build_persisted_from_cache;
        use paneflow_threads::message::Message;
        use paneflow_threads::{ContentBlock, PersistedThreadItem};

        let items = vec![
            ThreadItem::UserMessage(UserMessage {
                msg: Message::user_text("live-dirty-text"),
                markdown: None,
            }),
            ThreadItem::UserMessage(UserMessage {
                msg: Message::user_text("live-clean-text"),
                markdown: None,
            }),
        ];
        // Sentinel cached form for idx 1 — distinct text proves a
        // cache hit (returned verbatim) vs a cache miss (would
        // re-serialise from `live-clean-text`).
        let cache = vec![
            None,
            Some(PersistedThreadItem::Message(Message::user_text(
                "SENTINEL-FROM-CACHE",
            ))),
        ];

        let out = build_persisted_from_cache(&items, &cache, &HashSet::new(), &HashMap::new());

        assert_eq!(out.len(), 2, "expected one persisted entry per item");

        // Dirty slot: serialize_thread_item ran -> live text wins.
        let extract = |item: &PersistedThreadItem| -> String {
            match item {
                PersistedThreadItem::Message(m) => m
                    .content
                    .iter()
                    .find_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            }
        };
        assert_eq!(
            extract(&out[0]),
            "live-dirty-text",
            "dirty slot must be rebuilt from the live item"
        );
        assert_eq!(
            extract(&out[1]),
            "SENTINEL-FROM-CACHE",
            "clean slot must return the cached sentinel verbatim (no re-serialisation)",
        );
    }
}
