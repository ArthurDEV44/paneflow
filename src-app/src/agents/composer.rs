// US-016 wires this into ThreadView. While the wiring lands the
// public surface is touched in fewer places than it eventually will
// be -- a single allow-dead-code on the module keeps the iteration
// loop quiet without scattering attributes per item.
#![allow(dead_code)]

//! US-016 (prd-agents-view.md): the agents-view Composer.
//!
//! Anchored at the bottom of the [`super::thread_view::ThreadView`]
//! as a sticky card. Owns:
//! - the multi-line input (via [`crate::widgets::text_area::TextArea`]),
//! - the Send/Stop button morph (driven by `is_streaming`),
//! - three picker pills (agent / model / mode) with inline panels,
//! - the live [`super::runtime::SessionRuntime`] for the selected agent.
//!
//! Behavioural contract:
//! - Send: clears the input, appends the user message to
//!   ThreadView, opens an assistant stream, dispatches the prompt to
//!   the runtime.
//! - Stop: routes a `session/cancel` notification to the runtime,
//!   flushes the streaming buffer, leaves the partial assistant
//!   message in place with a "Stopped" indicator (AC #8 -- partial
//!   message preserved).
//! - Pump task: a 16 ms `cx.spawn` loop drains
//!   [`SessionRuntime::poll`] and routes events into the ThreadView
//!   streaming pipeline / the composer's picker pill state.
//!
//! The Composer is owned by ThreadView. Dropping ThreadView drops
//! the Composer which drops the SessionRuntime (AC #7 -- agent
//! switch creates a NEW session by reconstructing the runtime).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use agent_client_protocol::schema::SessionMode;
use gpui::{
    AnyElement, AsyncApp, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Image,
    ImageFormat, InteractiveElement, IntoElement, ObjectFit, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div, img, prelude::*, px, rgb,
    svg,
};
use paneflow_acp::{AgentDiscovery, AgentKind};
use paneflow_threads::{ContentBlock as ThreadsContentBlock, ThreadId, ThreadStore};

use super::composer_ext::{
    AttachmentKind, DropClassification, MAX_ATTACHMENTS, MENTION_DEBOUNCE, MentionState,
    PendingAttachment, SlashCommand, SlashCommandSource, SlashState, agent_slash_command_from_acp,
    attachment_limit_message, built_in_slash_commands, classify_dropped_path, combine_prompt,
    detect_image_mime, image_block_from_bytes, image_too_large_message,
    merge_and_filter_slash_commands, resource_block_for_path, scan_files, token_before_cursor,
};
use super::runtime::{ModelChoice, RuntimeEvent, SessionRuntime, SpawnOptions, StopReasonKind};
// US-017: tool-call events surface to the ThreadView's timeline.
// The types come back through `super::runtime`, but we forward the
// payload verbatim so this re-import is just to keep the match arm
// concise.
use super::thread_view::ThreadView;

/// 16 ms pump cadence -- matches the streaming pipeline ticker so
/// the ACP -> ThreadView hop never lags behind reveal pacing.
const PUMP_TICK: std::time::Duration = std::time::Duration::from_millis(16);

/// US-102: how long the composer waits after the last keystroke
/// before flushing the in-progress draft to SQLite. AC #4 caps disk
/// writes at ≤ 5 / sec under continuous typing -- a 300 ms debounce
/// produces ~3.3 writes / sec.
const DRAFT_FLUSH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// Per-message thinking effort the user picks from the composer pill,
/// independent of any ACP session mode. UI-only for now — wiring it
/// through to the agent's prompt parameters lands as a follow-up
/// (Codex accepts `--reasoning-effort`; Claude Code currently ignores
/// the value but still surfaces the chooser for UX consistency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
    ExtraHigh,
}

impl ThinkingEffort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::ExtraHigh => "Extra High",
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Low, Self::Medium, Self::High, Self::ExtraHigh]
    }

    /// Lowercase token the Claude Code CLI writes in
    /// `~/.claude/settings.json` under the `effortLevel` key.
    fn settings_token(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::ExtraHigh => "xhigh",
        }
    }

    fn from_settings_token(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::ExtraHigh),
            _ => None,
        }
    }
}

/// Location of Claude Code's persisted settings. The CLI reads this
/// on each invocation, so writing here is enough for the change to
/// take effect on the next prompt — no need to restart anything.
fn claude_settings_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// Best-effort read of the current `effortLevel` from Claude Code's
/// settings. `None` means either the file doesn't exist, isn't valid
/// JSON, or has no `effortLevel` key — caller falls back to the UI
/// default.
fn read_claude_effort_from_settings() -> Option<ThinkingEffort> {
    let path = claude_settings_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    ThinkingEffort::from_settings_token(value.get("effortLevel")?.as_str()?)
}

/// Write the new `effortLevel` back into Claude Code's settings,
/// preserving every other key in the JSON object. Errors are logged
/// at warn level — the UI side has already updated, so a write
/// failure is a soft failure (next session will not pick up the
/// change, but the current chat keeps working).
fn write_claude_effort_to_settings(effort: ThinkingEffort) {
    let Some(path) = claude_settings_path() else {
        log::warn!("composer: cannot resolve ~/.claude/settings.json");
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "effortLevel".to_string(),
            serde_json::Value::String(effort.settings_token().to_string()),
        );
    } else {
        log::warn!("composer: ~/.claude/settings.json is not a JSON object");
        return;
    }
    match serde_json::to_string_pretty(&value) {
        Ok(serialised) => {
            if let Err(err) = std::fs::write(&path, serialised) {
                log::warn!("composer: failed to write effortLevel: {err}");
            }
        }
        Err(err) => log::warn!("composer: failed to serialise settings: {err}"),
    }
}

// ---------------------------------------------------------------------------
// Codex bridge — TOML config + base model list from the disk cache
// ---------------------------------------------------------------------------

fn codex_config_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("config.toml"))
}

fn codex_models_cache_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("models_cache.json"))
}

/// Read `model` + `model_reasoning_effort` from `~/.codex/config.toml`.
/// Both fields are independently optional — the file may exist with
/// only one set (or neither), in which case the caller falls back to
/// the per-agent UI defaults.
fn read_codex_settings() -> (Option<String>, Option<ThinkingEffort>) {
    let Some(path) = codex_config_path() else {
        return (None, None);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (None, None);
    };
    let Ok(value): Result<toml::Value, _> = toml::from_str(&text) else {
        return (None, None);
    };
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let effort = value
        .get("model_reasoning_effort")
        .and_then(|v| v.as_str())
        .and_then(ThinkingEffort::from_settings_token);
    (model, effort)
}

/// Read the base model list Codex caches on disk after talking to
/// the OpenAI API. Returns `(slug, display_name)` pairs — already
/// deduplicated (one entry per base model, no per-effort variants),
/// which is exactly the shape the model picker wants.
fn read_codex_models() -> Vec<(String, String)> {
    let Some(path) = codex_models_cache_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value): Result<serde_json::Value, _> = serde_json::from_str(&text) else {
        return Vec::new();
    };
    let Some(arr) = value.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let slug = entry.get("slug")?.as_str()?.to_string();
            let display = entry
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| slug.clone());
            Some((slug, display))
        })
        .collect()
}

/// Update a single top-level key in `~/.codex/config.toml`, leaving
/// every other key untouched. Uses string concat on a fresh `toml`
/// value so comments and ordering aren't preserved — Codex itself
/// rewrites this file the same way when the user runs slash commands.
fn write_codex_string_field(field: &str, value: &str) {
    let Some(path) = codex_config_path() else {
        log::warn!("composer: cannot resolve ~/.codex/config.toml");
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Value =
        toml::from_str(&text).unwrap_or_else(|_| toml::Value::Table(Default::default()));
    if let Some(table) = doc.as_table_mut() {
        table.insert(field.to_string(), toml::Value::String(value.to_string()));
    } else {
        log::warn!("composer: ~/.codex/config.toml is not a TOML table");
        return;
    }
    match toml::to_string(&doc) {
        Ok(serialised) => {
            if let Err(err) = std::fs::write(&path, serialised) {
                log::warn!("composer: failed to write codex {field}: {err}");
            }
        }
        Err(err) => log::warn!("composer: failed to serialise codex config: {err}"),
    }
}

fn write_codex_effort(effort: ThinkingEffort) {
    write_codex_string_field("model_reasoning_effort", effort.settings_token());
}

fn write_codex_model(model_id: &str) {
    write_codex_string_field("model", model_id);
}

// ---------------------------------------------------------------------------
// Per-agent hydration dispatchers
// ---------------------------------------------------------------------------

/// Pull the persisted reasoning effort for `kind`, falling back to
/// [`ThinkingEffort::Medium`] when nothing is configured.
fn hydrate_effort_for(kind: AgentKind) -> ThinkingEffort {
    match kind {
        AgentKind::ClaudeCode => {
            read_claude_effort_from_settings().unwrap_or(ThinkingEffort::Medium)
        }
        AgentKind::Codex => {
            let (_, effort) = read_codex_settings();
            effort.unwrap_or(ThinkingEffort::Medium)
        }
    }
}

/// Pull the persisted model id for `kind` (Codex only — Claude Code
/// doesn't persist a flat `model` field). `None` means "fall back to
/// whatever ACP `SessionReady` reports".
fn hydrate_model_for(kind: AgentKind) -> Option<String> {
    match kind {
        AgentKind::Codex => read_codex_settings().0,
        _ => None,
    }
}

/// Mirror the picker selection back into the CLI's settings file
/// so the next agent invocation reads the new value. No-op for
/// agents without a known persistence path.
fn persist_effort(kind: AgentKind, effort: ThinkingEffort) {
    match kind {
        AgentKind::ClaudeCode => write_claude_effort_to_settings(effort),
        AgentKind::Codex => write_codex_effort(effort),
    }
}

fn persist_model(kind: AgentKind, model_id: &str) {
    if matches!(kind, AgentKind::Codex) {
        write_codex_model(model_id);
    }
}

// ---------------------------------------------------------------------------
// US-115: profile snapshot model + built-in seed
// ---------------------------------------------------------------------------

/// Resolved profile shape ready to apply to a `Composer`. Mirrors
/// the persisted [`paneflow_config::schema::ProfileConfig`] but
/// carries typed enums so the apply path can short-circuit invalid
/// strings (e.g. an unknown agent tag) at construction time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileSnapshot {
    pub agent: Option<AgentKind>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub effort: Option<ThinkingEffort>,
    pub tools: Vec<String>,
}

impl ProfileSnapshot {
    /// Lift a persisted [`paneflow_config::schema::ProfileConfig`]
    /// into the runtime shape. Unknown agent / effort tags are
    /// dropped (forward-compat) so a config edited against a future
    /// schema does not crash the apply path.
    fn from_config(config: &paneflow_config::schema::ProfileConfig) -> Self {
        let agent = config
            .agent
            .as_deref()
            .and_then(crate::project::agent_kind_from_str);
        let effort = config
            .effort
            .as_deref()
            .and_then(ThinkingEffort::from_settings_token);
        Self {
            agent,
            model: config.model.clone(),
            mode: config.mode.clone(),
            effort,
            tools: config.tools.clone(),
        }
    }

    /// Lower a runtime snapshot back to the persisted shape -- used
    /// by the "save current as profile" flow.
    fn to_config(&self) -> paneflow_config::schema::ProfileConfig {
        paneflow_config::schema::ProfileConfig {
            agent: self
                .agent
                .map(|k| crate::project::agent_kind_to_str(k).to_string()),
            model: self.model.clone(),
            mode: self.mode.clone(),
            effort: self.effort.map(|e| e.settings_token().to_string()),
            tools: self.tools.clone(),
        }
    }
}

/// One row surfaced in the profile picker popover. The `is_builtin`
/// flag drives the "Delete" affordance — built-ins cannot be deleted
/// per AC #7.
#[derive(Debug, Clone)]
pub struct ProfileSummary {
    pub name: String,
    pub is_builtin: bool,
    pub snapshot: ProfileSnapshot,
}

/// US-115 AC #1: three built-in profiles seeded by the runtime. They
/// are NOT written to `paneflow.json` unless the user customises one
/// — a clean `paneflow.json` keeps the profiles map empty, and the
/// composer overlays these in-memory.
///
/// Snapshots:
/// - **Write**: high effort, no agent/model lock-in (the user's
///   current pick stays). Tool list is empty (== "all tools").
/// - **Ask**: low effort + only read-only tools enabled (`read`,
///   `search`, `fetch`, `think`).
/// - **Minimal**: low effort + zero tools enabled (chat-only).
pub fn built_in_profiles() -> Vec<ProfileSummary> {
    vec![
        ProfileSummary {
            name: "Write".to_string(),
            is_builtin: true,
            snapshot: ProfileSnapshot {
                agent: None,
                model: None,
                mode: None,
                effort: Some(ThinkingEffort::High),
                tools: Vec::new(),
            },
        },
        ProfileSummary {
            name: "Ask".to_string(),
            is_builtin: true,
            snapshot: ProfileSnapshot {
                agent: None,
                model: None,
                mode: None,
                effort: Some(ThinkingEffort::Low),
                tools: vec![
                    "read".to_string(),
                    "search".to_string(),
                    "fetch".to_string(),
                    "think".to_string(),
                ],
            },
        },
        ProfileSummary {
            name: "Minimal".to_string(),
            is_builtin: true,
            snapshot: ProfileSnapshot {
                agent: None,
                model: None,
                mode: None,
                effort: Some(ThinkingEffort::Low),
                tools: Vec::new(),
            },
        },
    ]
}

/// US-115: full list of profiles surfaced in the picker. Built-ins
/// come first (in declared order), then user-saved customs (also in
/// declared order from the JSON map -- order within a HashMap is
/// not stable, but the picker sorts customs alphabetically so the
/// UI stays predictable).
pub fn all_profiles() -> Vec<ProfileSummary> {
    let mut out = built_in_profiles();
    let built_in_names: std::collections::HashSet<String> =
        out.iter().map(|p| p.name.clone()).collect();
    let mut customs: Vec<(String, paneflow_config::schema::ProfileConfig)> =
        super::panel_config::active_profiles().into_iter().collect();
    customs.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, cfg) in customs {
        if built_in_names.contains(&name) {
            // A custom save under a built-in name overrides the
            // built-in (AC: "user-saved customisations + overrides").
            if let Some(slot) = out.iter_mut().find(|p| p.name == name) {
                slot.snapshot = ProfileSnapshot::from_config(&cfg);
                slot.is_builtin = false;
            }
        } else {
            out.push(ProfileSummary {
                name,
                is_builtin: false,
                snapshot: ProfileSnapshot::from_config(&cfg),
            });
        }
    }
    out
}

/// US-019: short human-friendly label for an attached file path.
/// Strips the thread cwd prefix when applicable; otherwise displays
/// the file name (so users see "src/lib.rs" inside the project but
/// just "report.pdf" for an outside-the-project pick).
fn label_for_path(path: &Path, cwd: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(cwd) {
        return rel.display().to_string();
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// US-106: one prompt buffered behind an in-flight turn. Stored as a
/// fully-baked `Vec<ContentBlock>` (text + attachments already
/// combined) so dequeue can hit the wire without re-running mention /
/// slash parsing on an empty textarea.
#[derive(Debug, Clone)]
pub struct PendingPrompt {
    pub blocks: Vec<agent_client_protocol::schema::ContentBlock>,
}

/// US-106: the four user-visible send-button states. Computed by
/// [`send_button_state`] from `(is_streaming, has_content)` so the
/// branching is testable without spinning up a GPUI context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendButtonState {
    /// State 1: idle agent, empty composer. Button is disabled.
    IdleEmpty,
    /// State 2: idle agent, composer has text or attachments. Click
    /// sends the prompt.
    IdleContent,
    /// State 3: streaming, empty composer. Button morphs to a red Stop
    /// that cancels the current turn.
    StreamingEmpty,
    /// State 4: streaming, composer has text or attachments. Click
    /// enqueues; `Ctrl+Shift+Enter` (Cmd on macOS) bypasses the queue
    /// and sends immediately.
    StreamingContent,
}

/// US-106: pure-function predicate that maps `(is_streaming,
/// has_content)` onto one of the four user-visible send-button states.
/// Lives outside `Composer::render_send_button` so the state matrix is
/// unit-testable without a GPUI test harness.
pub fn send_button_state(is_streaming: bool, has_content: bool) -> SendButtonState {
    match (is_streaming, has_content) {
        (false, false) => SendButtonState::IdleEmpty,
        (false, true) => SendButtonState::IdleContent,
        (true, false) => SendButtonState::StreamingEmpty,
        (true, true) => SendButtonState::StreamingContent,
    }
}

/// Take the first non-empty line of a prompt, collapse whitespace,
/// and truncate to ~60 chars on a word boundary. Returns `None` when
/// the prompt is empty / pure whitespace so the caller can skip
/// emitting a useless suggestion. Used by [`Composer::send_prompt`]
/// to seed a sidebar title on the first prompt of a thread without
/// waiting for an ACP `SessionInfoUpdate`.
const AUTO_TITLE_MAX_CHARS: usize = 60;
pub(crate) fn derive_title_from_prompt(text: &str) -> Option<String> {
    let first_line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let collapsed: String = first_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= AUTO_TITLE_MAX_CHARS {
        return Some(collapsed);
    }
    // Cut on a word boundary to keep the title readable. Walking chars
    // (not bytes) keeps the truncation safe for non-ASCII prompts.
    let mut cut: String = collapsed.chars().take(AUTO_TITLE_MAX_CHARS).collect();
    if let Some(last_space) = cut.rfind(' ') {
        cut.truncate(last_space);
    }
    cut.push('\u{2026}');
    Some(cut)
}

pub struct Composer {
    text_area: Entity<crate::widgets::text_area::TextArea>,
    agent_kind: AgentKind,
    discovery: Arc<AgentDiscovery>,
    cwd: PathBuf,
    thread_view: WeakEntity<ThreadView>,

    runtime: Option<SessionRuntime>,
    is_streaming: bool,
    fatal_error: Option<String>,

    /// Informational status surfaced after a non-clean turn-end (e.g.
    /// "Response truncated: max output tokens reached"). Distinct
    /// from [`Self::fatal_error`] which is reserved for runtime
    /// crashes -- this is the model stopping for non-error reasons
    /// (MaxTokens / MaxTurnRequests / Cancelled). Cleared on the
    /// next prompt submission so a fresh turn doesn't ship a stale
    /// banner. Sourced from [`StopReasonKind::status_message`].
    stop_status: Option<&'static str>,

    /// US-106: prompts the user submitted while a previous turn was
    /// still streaming. Auto-dequeued on `RuntimeEvent::TurnEnded`.
    /// Drops only via an explicit clear (the "x" affordance in the
    /// activity bar US-107) -- cancellation preserves the queue so the
    /// user can stop the current turn and have their queued prompt
    /// take over rather than getting silently dropped.
    pending_prompts: VecDeque<PendingPrompt>,

    modes: Vec<SessionMode>,
    current_mode_id: Option<String>,
    models: Vec<ModelChoice>,
    current_model_id: Option<String>,

    show_agent_picker: bool,
    show_model_picker: bool,
    show_mode_picker: bool,

    /// Per-message thinking-effort selection surfaced as the
    /// lightbulb pill in the composer toolbar. Defaults to `Medium`.
    /// Independent from ACP session modes — see [`ThinkingEffort`].
    current_effort: ThinkingEffort,
    show_effort_picker: bool,

    /// US-019: chips above the textarea. Each entry maps to a
    /// `ContentBlock` (Image or ResourceLink) that gets spliced into
    /// the next `session/prompt`. The user can remove a chip before
    /// sending.
    attachments: Vec<PendingAttachment>,

    /// US-019: surfaced under the input row whenever an attach
    /// operation fails (e.g. image too large). Independent from
    /// `fatal_error` (runtime-level) so an attachment hiccup never
    /// hides a fatal agent error or vice versa.
    attach_error: Option<String>,

    /// US-019: `+` button popover. When open, the `+` button shows
    /// the four attach actions; clicking outside (or any picker
    /// pill) closes it.
    show_attach_menu: bool,

    /// US-002 (visual-parity): expand toggle for the composer card.
    /// When `true`, the textarea wrapper grows to 80% of the viewport
    /// height (mirrors Zed's `vh(0.8, window)` at
    /// `thread_view.rs:3582`). Flipped by the absolute top-right
    /// expand button (`icons/generic_maximize.svg` collapsed →
    /// `icons/chevron_down.svg` expanded).
    editor_expanded: bool,

    /// US-019: active `@`-mention popup. `None` when the cursor is
    /// not currently inside an `@<word>` token.
    mention_state: Option<MentionState>,

    /// US-019: active `/`-slash popup. `None` when the cursor is not
    /// currently inside a `/<word>` token at line-start.
    slash_state: Option<SlashState>,

    /// US-112: cache of `/`-commands the active ACP session has
    /// advertised via `available_commands_update`. Merged with the
    /// built-in `/clear` + `/export` at filter time. Cleared on
    /// agent switch (each agent ships its own command list).
    agent_slash_commands: Vec<SlashCommand>,

    /// US-115: name of the profile last applied to this composer.
    /// `None` means "no preset active" -- the user is in a custom
    /// state (free-form pill picks). Surfaced as the profile pill
    /// label in the composer footer.
    current_profile_name: Option<String>,
    /// US-115: profile picker popover open / closed.
    show_profile_picker: bool,
    /// US-115: when `Some`, the picker is in "name a new save"
    /// mode -- the bottom row of the popover swaps to an inline
    /// text input whose key handler routes through
    /// [`Composer::handle_profile_name_key`]. `None` when the
    /// popover is in its normal pick mode.
    profile_save_text: Option<String>,
    /// US-115: focus handle for the save-as text input.
    profile_save_focus: FocusHandle,

    /// Timer-driven housekeeping pump: debounced `@`-mention scan,
    /// debounced draft flush, and the SessionReady timeout. Runs at
    /// `PUMP_TICK` (16 ms). Events from the ACP runtime flow through
    /// the separate push-based [`Self::_event_task`] -- they do NOT
    /// wait for this tick anymore. Dropping cancels the task so no
    /// dangling loop survives the composer.
    _pump_task: Option<Task<()>>,

    /// Push-based ACP event task. Awaits on a
    /// [`futures::channel::mpsc::UnboundedReceiver`] handed over by
    /// [`SessionRuntime::take_event_receiver`] and drains every queued
    /// event in one entity update per wake. Replaces the previous
    /// 16 ms poll path so first-token latency drops from "up to one
    /// pump tick" to "as soon as GPUI schedules the task". Mirrors
    /// Zed's terminal event loop (`crates/terminal/src/terminal.rs:701`).
    /// `None` until [`Self::ensure_runtime`] hands the receiver over.
    _event_task: Option<Task<()>>,

    /// US-116 AC #3: tools invoked since the last prompt was sent.
    /// The OS-notification helper reads this to choose between
    /// "Finished running tools" (when > 0) and "New message" (when
    /// 0). Reset to 0 on send and right after the notification fires
    /// so the next turn starts from a clean slate.
    current_turn_tool_count: u32,

    // ---- US-102: draft persistence ---------------------------------
    /// Persisted-thread row id when the parent ThreadView has one.
    /// `None` for in-session-only threads — drafts are skipped on
    /// those (nothing to key the row by).
    draft_thread_id: Option<ThreadId>,
    /// SQLite-backed store the composer reads/writes/deletes drafts
    /// against. Cloned from the parent ThreadView; cheap to hold.
    draft_store: Option<ThreadStore>,
    /// Deadline at which the pump task flushes the in-progress draft
    /// (text + attachments) to disk. `Some(t)` means "dirty since the
    /// last write; flush once `Instant::now() >= t`". AC #4 — 300ms
    /// debounce so continuous typing produces ≤ ~3 writes / sec.
    draft_flush_deadline: Option<std::time::Instant>,
    /// Snapshot of what the on-disk draft currently looks like, so
    /// `on_text_change` can no-op when the user types back to the
    /// persisted state. Empty Vec for "no draft on disk".
    draft_last_persisted: Vec<ThreadsContentBlock>,

    /// US-120 / US-121: last seen ACP `UsageUpdate` for the session.
    /// `(used, size)` in tokens; the composer footer renders the badge
    /// from `used` (humanized as `Nk tokens`), the activity bar gates
    /// US-121's throughput suffix on `used >= 1000`. `None` until the
    /// first `UsageUpdate` arrives -- AC #4 / #2 keeps the badge absent
    /// rather than rendering `0 tokens`. Cleared on `SessionReady`
    /// (a fresh session re-opens the count).
    last_usage: Option<(u64, u64)>,

    /// US-121: timestamp the most recent prompt was sent. Drives the
    /// elapsed-time suffix in the activity bar (AC #1) -- gated at 3s
    /// per the Paneflow divergence from Zed's 30s STOPWATCH_THRESHOLD.
    /// Cleared when `is_streaming` flips back to `false` so the row
    /// collapses cleanly (AC #5).
    turn_started_at: Option<std::time::Instant>,

    /// One-shot guard for the background title summarizer (Zed
    /// `Thread::generate_title` parity). `false` until the first
    /// clean `TurnEnded` for this thread fires the summarizer;
    /// `true` afterwards so subsequent turn-ends never re-trigger.
    /// Mirrors Zed's `pending_title_generation.is_none()` gate at
    /// `crates/agent/src/thread.rs:2734`. Survives a runtime crash
    /// / restart (we never reset it) -- the auto-title is best
    /// generated from the very first exchange, not from a recovered
    /// mid-conversation state.
    title_summarization_done: bool,

    /// Title in the sidebar at the moment the first user prompt was
    /// sent. The background summarizer ships this back via
    /// `TitleReplacePolicy::OnlyIfStillEqualTo(snapshot)` so a user
    /// rename that lands during the (~1-3 s) transient session round
    /// trip is preserved. `Some` only between first `send_prompt`
    /// and first `TurnEnded`; `None` afterwards (taken by the
    /// summarizer trigger).
    title_snapshot_for_summary: Option<String>,

    /// Tally of consecutive `RuntimeEvent::Fatal` since the last clean
    /// `SessionReady`. Used to compute an exponential backoff before
    /// re-spawning a broken agent so a permanently-failing CLI does
    /// not spin in a tight spawn/crash loop.
    consecutive_fatal_count: u8,

    /// Earliest instant at which the next `ensure_runtime` may spawn
    /// the agent. Bumped on Fatal, cleared on SessionReady. Acts as
    /// the backoff fence; an attempt before this instant is no-op and
    /// surfaces a transient `fatal_error`.
    runtime_respawn_after: Option<std::time::Instant>,

    /// Deadline after which a freshly-spawned agent that hasn't sent
    /// `SessionReady` is treated as hung. Set on spawn, cleared on
    /// SessionReady or Fatal. The pump synthesizes a `Fatal` if the
    /// deadline elapses while `runtime.is_some()` but no SessionReady
    /// has arrived.
    session_ready_deadline: Option<std::time::Instant>,

    /// Cached `all_profiles()` snapshot. Populated lazily so the
    /// profile picker render path doesn't touch the disk every frame
    /// (60 fps × disk read while the popover is open). Invalidated
    /// on every profile mutation (save / delete / apply / agent
    /// switch).
    cached_profiles: Option<Vec<ProfileSummary>>,

    /// Cached Codex model list parsed from the per-user config file.
    /// Populated on first `models()` read and invalidated on
    /// `SessionReady` (a different agent or different config).
    /// Avoids parsing the JSON on every frame the model picker is
    /// open.
    cached_codex_models: Option<Vec<ModelChoice>>,

    /// Cached "is the composer non-empty?" flag. Updated in
    /// `on_text_change` (text edits) and on every push/pop of
    /// `attachments`, so `render_send_button` doesn't re-read the
    /// text area entity from inside the render pass. Mirrors Zed's
    /// MessageEditor pattern of pushing button state through events
    /// instead of pulling from the editor entity at render time.
    has_content_cached: bool,
}

impl Composer {
    pub fn new(
        agent_kind: AgentKind,
        discovery: Arc<AgentDiscovery>,
        cwd: PathBuf,
        thread_view: WeakEntity<ThreadView>,
        draft_thread_id: Option<ThreadId>,
        draft_store: Option<ThreadStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_area = cx.new(|cx| crate::widgets::text_area::TextArea::new("Ask anything", cx));

        // US-102 AC #3 / #6: read the persisted draft synchronously
        // so the textarea + attachment chips are populated by the
        // time the view mounts (well under the 100 ms target -- the
        // read is a single SQLite point lookup).
        let (initial_text, initial_attachments, last_persisted) =
            match (draft_thread_id.as_ref(), draft_store.as_ref()) {
                (Some(id), Some(store)) => match store.read_draft(id) {
                    Ok(Some(blocks)) => {
                        let (text, attachments) = split_draft_blocks(&blocks);
                        (text, attachments, blocks)
                    }
                    Ok(None) => (String::new(), Vec::new(), Vec::new()),
                    Err(err) => {
                        log::warn!("Composer: read_draft failed: {err}");
                        (String::new(), Vec::new(), Vec::new())
                    }
                },
                _ => (String::new(), Vec::new(), Vec::new()),
            };

        if !initial_text.is_empty() {
            text_area.update(cx, |ta, cx| ta.set_value(initial_text.clone(), cx));
        }

        // Wire the textarea's submit signal back into the composer:
        // the submit callback captures a WeakEntity<Composer> and
        // calls `send_prompt`. We update the textarea's on_submit
        // *after* the composer is built so the WeakEntity is real.
        let mut composer = Self {
            text_area,
            agent_kind,
            discovery,
            cwd,
            thread_view,
            runtime: None,
            is_streaming: false,
            fatal_error: None,
            stop_status: None,
            pending_prompts: VecDeque::new(),
            modes: Vec::new(),
            current_mode_id: None,
            models: Vec::new(),
            // Hydrate the model pill the same way as effort: read
            // what the CLI persisted (Codex only — Claude Code does
            // not expose a flat `model` field, so it stays None until
            // ACP `SessionReady` reports the current model).
            current_model_id: hydrate_model_for(agent_kind),
            show_agent_picker: false,
            show_model_picker: false,
            show_mode_picker: false,
            // Hydrate the effort pill from whatever the CLI persisted.
            // Claude Code → `effortLevel` in
            // `~/.claude/settings.json`. Codex →
            // `model_reasoning_effort` in `~/.codex/config.toml`.
            current_effort: hydrate_effort_for(agent_kind),
            show_effort_picker: false,
            attachments: initial_attachments,
            attach_error: None,
            show_attach_menu: false,
            editor_expanded: false,
            mention_state: None,
            slash_state: None,
            agent_slash_commands: Vec::new(),
            current_profile_name: super::panel_config::active_default_profile(),
            show_profile_picker: false,
            profile_save_text: None,
            profile_save_focus: cx.focus_handle(),
            _pump_task: None,
            _event_task: None,
            draft_thread_id,
            draft_store,
            draft_flush_deadline: None,
            draft_last_persisted: last_persisted,
            current_turn_tool_count: 0,
            last_usage: None,
            turn_started_at: None,
            title_summarization_done: false,
            title_snapshot_for_summary: None,
            consecutive_fatal_count: 0,
            runtime_respawn_after: None,
            session_ready_deadline: None,
            cached_profiles: None,
            cached_codex_models: None,
            has_content_cached: false,
        };
        composer.install_submit_callback(cx);
        composer.install_submit_immediate_callback(cx);
        composer.install_change_callback(cx);
        composer.install_escape_callback(cx);
        composer.start_pump(cx);
        // Eager-spawn the ACP runtime so `SessionReady` arrives
        // before the user touches any pill. Without this the
        // model/mode pickers stay on placeholder labels until the
        // first prompt is sent. `ensure_runtime` is idempotent and
        // fast (the actual CLI spawn happens on a background task),
        // and failure is captured in `fatal_error` so the UI surfaces
        // a "not on PATH" message instead of silently no-op'ing.
        let _ = composer.ensure_runtime(cx);
        composer.install_release_hook(cx);
        composer
    }

    /// Wire a GPUI `on_release` so that when the `Composer` entity is
    /// dropped (sidebar switch, app shutdown) any in-flight stream is
    /// flushed and persisted. Without this, chunks accumulated since
    /// the last throttled persist would be silently lost on close.
    fn install_release_hook(&mut self, cx: &mut Context<Self>) {
        let view_weak = self.thread_view.clone();
        cx.on_release(move |_composer, app| {
            if let Some(view) = view_weak.upgrade() {
                view.update(app, |tv, cx| {
                    // Order matters: `flush_streaming` MUST run before
                    // `finalize_thinking`. `finalize_thinking` calls
                    // `close_open_assistant_message` which sets
                    // `streaming_message_idx = None`; if it runs first,
                    // `flush_streaming` then sees `idx=None`, bails
                    // early, and any chars still pacing through
                    // `streaming_buffer` are silently dropped. Symptom
                    // pre-fix: Codex turns appeared truncated at the
                    // last 20–30 chars because the trailing burst sat
                    // in the buffer when the turn ended.
                    tv.flush_streaming(cx);
                    tv.finalize_thinking(cx);
                });
            }
        })
        .detach();
    }

    /// Connect the TextArea's Enter -> submit signal to
    /// [`Composer::send_prompt`].
    fn install_submit_callback(&mut self, cx: &mut Context<Self>) {
        let composer_weak = cx.weak_entity();
        self.text_area.update(cx, move |ta, _| {
            ta.on_submit(move |text, _w, cx| {
                if let Some(this) = composer_weak.upgrade() {
                    this.update(cx, |composer, cx| {
                        composer.send_prompt(text, cx);
                    });
                }
            });
        });
    }

    /// US-106: connect the TextArea's Ctrl+Shift+Enter -> submit-
    /// immediate signal to [`Composer::send_prompt_immediate`]. Bypasses
    /// the queue: cancels any in-flight turn and dispatches the new
    /// prompt straight to the runtime.
    fn install_submit_immediate_callback(&mut self, cx: &mut Context<Self>) {
        let composer_weak = cx.weak_entity();
        self.text_area.update(cx, move |ta, _| {
            ta.on_submit_immediate(move |text, _w, cx| {
                if let Some(this) = composer_weak.upgrade() {
                    this.update(cx, |composer, cx| {
                        composer.send_prompt_immediate(text, cx);
                    });
                }
            });
        });
    }

    /// US-019: wire the textarea's content-change signal to
    /// [`Composer::on_text_change`] so popups stay in sync with the
    /// cursor position and the current `@` / `/` token.
    fn install_change_callback(&mut self, cx: &mut Context<Self>) {
        let composer_weak = cx.weak_entity();
        self.text_area.update(cx, move |ta, _| {
            ta.on_change(move |text, cursor, cx_ta| {
                if let Some(this) = composer_weak.upgrade() {
                    let text = text.to_string();
                    this.update(cx_ta, |composer, cx| {
                        composer.on_text_change(&text, cursor, cx);
                    });
                }
            });
        });
    }

    /// US-019: wire the textarea's Escape signal to dismiss any open
    /// popup. The action fires only while the textarea is focused,
    /// matching the AC: Escape never affects the buffer text -- it
    /// only closes the popup. The `@` / `/` character that opened the
    /// popup remains in the buffer because Escape is a no-op on
    /// content.
    fn install_escape_callback(&mut self, cx: &mut Context<Self>) {
        let composer_weak = cx.weak_entity();
        self.text_area.update(cx, move |ta, _| {
            ta.on_escape(move |_w, app| {
                if let Some(this) = composer_weak.upgrade() {
                    this.update(app, |composer, cx| {
                        composer.dismiss_popups(cx);
                    });
                }
            });
        });
    }

    /// US-019: textarea content-change handler. Recomputes the
    /// `@`-mention and `/`-slash popup state. The actual filesystem
    /// scan is deferred to the pump task to honor the 200ms
    /// debounce (AC).
    fn on_text_change(&mut self, content: &str, cursor: usize, cx: &mut Context<Self>) {
        // Reset attachment errors on edit -- the user is moving on.
        self.attach_error = None;

        // Track non-empty composer state without re-reading the TextArea
        // entity from inside the render pass — Zed parity for
        // MessageEditor's button-state pattern.
        self.has_content_cached = !content.trim().is_empty() || !self.attachments.is_empty();

        // US-102 AC #4: arm the 300 ms debounce deadline on every
        // edit. The pump tick checks the deadline and writes when
        // elapsed -- so continuous typing produces at most one disk
        // write every 300 ms (≤ ~3.3 / sec, well under the 5 / sec
        // ceiling the AC measures against).
        if self.draft_thread_id.is_some() && self.draft_store.is_some() {
            self.draft_flush_deadline = Some(std::time::Instant::now() + DRAFT_FLUSH_DEBOUNCE);
        }

        // `/` first so an unrelated `@` later in the buffer cannot
        // hijack a line-start slash. Both helpers walk backward from
        // the cursor and only return a hit when the trigger sits at
        // the right boundary (line-start for `/`, word-start for `@`).
        let slash = token_before_cursor(content, cursor, '/');
        if let Some((anchor, query)) = slash {
            let results = self.filter_slash_commands(&query);
            self.slash_state = Some(SlashState {
                anchor,
                query,
                results,
            });
        } else {
            self.slash_state = None;
        }

        let mention = token_before_cursor(content, cursor, '@');
        if let Some((anchor, query)) = mention {
            let needs_rescan = match self.mention_state.as_ref() {
                Some(prev) => prev.anchor != anchor || prev.query != query,
                None => true,
            };
            if needs_rescan {
                self.mention_state = Some(MentionState {
                    anchor,
                    query,
                    results: Vec::new(),
                    query_started_at: Instant::now(),
                    scanned: false,
                });
            }
        } else {
            self.mention_state = None;
        }

        cx.notify();
    }

    /// Recompute the `has_content_cached` flag by reading the current
    /// TextArea content + attachment count. Call this from any code
    /// path that mutates `self.attachments`. Cheap (one borrow + one
    /// `is_empty` call) and avoids the render-time re-entrancy risk
    /// of reading the TextArea from `render_send_button`.
    fn refresh_has_content_cache(&mut self, cx: &Context<Self>) {
        let text_non_empty = !self.text_area.read(cx).is_empty();
        self.has_content_cached = text_non_empty || !self.attachments.is_empty();
    }

    /// US-019: close every popup. Used by Escape and by any path
    /// that takes a definitive action (send, cancel, pick an option).
    pub fn dismiss_popups(&mut self, cx: &mut Context<Self>) {
        let was_open = self.show_attach_menu
            || self.mention_state.is_some()
            || self.slash_state.is_some()
            || self.show_agent_picker
            || self.show_model_picker
            || self.show_mode_picker
            || self.show_effort_picker
            || self.show_profile_picker;
        self.show_attach_menu = false;
        self.mention_state = None;
        self.slash_state = None;
        // Picker pills (agent/model/mode/effort/profile) are not real
        // PopoverMenus, so they don't close themselves on outside-click;
        // bundle them here so any "definitive action" path clears them.
        self.show_agent_picker = false;
        self.show_model_picker = false;
        self.show_mode_picker = false;
        self.show_effort_picker = false;
        self.show_profile_picker = false;
        if was_open {
            cx.notify();
        }
    }

    /// US-019 + US-112: filter the merged slash command list against
    /// `query` (case-insensitive substring on `name`). The merge
    /// concatenates built-ins (`/clear`, `/export`) with the agent's
    /// own commands; on name collision the agent's version wins.
    fn filter_slash_commands(&self, query: &str) -> Vec<SlashCommand> {
        let built_ins = built_in_slash_commands();
        merge_and_filter_slash_commands(&built_ins, &self.agent_slash_commands, query)
    }

    /// Spawn the timer-driven housekeeping pump. After the runtime
    /// migration to push-based events (see [`Self::start_event_task`]),
    /// this loop ONLY handles debounced work: mention-scan flush,
    /// draft-flush, and the SessionReady timeout. ACP chunks /
    /// tool-call updates no longer pass through here -- they take the
    /// direct path from runtime -> event task -> handle_runtime_event,
    /// matching Zed's no-poll terminal flow (`crates/terminal/src/
    /// terminal.rs:701`).
    fn start_pump(&mut self, cx: &mut Context<Self>) {
        let weak_self: WeakEntity<Self> = cx.weak_entity();
        self._pump_task = Some(cx.spawn(async move |_, cx: &mut AsyncApp| {
            loop {
                smol::Timer::after(PUMP_TICK).await;
                let outcome =
                    cx.update(|cx| weak_self.update(cx, |composer, cx| composer.pump_runtime(cx)));
                // `Ok(true)`  -- runtime is still alive, keep polling.
                // `Ok(false)` -- pump asked to stop (currently unused).
                // `Err(_)`    -- the composer entity is gone; bail.
                match outcome {
                    Ok(true) => continue,
                    _ => break,
                }
            }
        }));
    }

    /// Start the push-based ACP event drain. Awaits on the runtime's
    /// futures::mpsc receiver and drains every queued event in one
    /// entity update per wake. The first event arrives "immediately"
    /// (no 16 ms polling delay); a burst of N chunks coalesces into
    /// one render. Replaces the previous `runtime.poll()` call that
    /// sat behind the pump tick.
    fn start_event_task(
        &mut self,
        mut rx: futures::channel::mpsc::UnboundedReceiver<RuntimeEvent>,
        cx: &mut Context<Self>,
    ) {
        let weak_self: WeakEntity<Self> = cx.weak_entity();
        self._event_task = Some(cx.spawn(async move |_, cx: &mut AsyncApp| {
            use futures::stream::StreamExt;
            while let Some(first) = rx.next().await {
                let outcome = cx.update(|cx| {
                    weak_self.update(cx, |composer, cx| {
                        composer.handle_runtime_event(first, cx);
                        // Coalesce burst: drain everything else already
                        // queued before yielding back to GPUI. One
                        // notify per batch instead of one per chunk.
                        while let Ok(ev) = rx.try_recv() {
                            composer.handle_runtime_event(ev, cx);
                        }
                    })
                });
                if outcome.is_err() {
                    break;
                }
            }
        }));
    }

    /// Drain the runtime queue. Returns false when the pump should
    /// stop (no live runtime).
    fn pump_runtime(&mut self, cx: &mut Context<Self>) -> bool {
        // US-019: also pump the `@`-mention debounce here so the file
        // walk runs on the GPUI main thread without a second task.
        // The walk is bounded (`MAX_FILE_RESULTS=50`, max_depth=8) so
        // the budget per tick is in the ~ms range even on cold caches.
        self.maybe_run_mention_scan(cx);

        // US-102: flush the draft when the 300 ms debounce elapses.
        self.maybe_flush_draft(cx);

        // SessionReady timeout: if a runtime spawn has not produced a
        // SessionReady within the deadline, synthesize a Fatal so the
        // UI surfaces "the agent isn't responding" instead of leaving
        // the pickers stuck on placeholder labels.
        if let Some(deadline) = self.session_ready_deadline
            && self.runtime.is_some()
            && std::time::Instant::now() >= deadline
        {
            self.session_ready_deadline = None;
            self.handle_runtime_event(
                RuntimeEvent::Fatal(format!(
                    "{} spawned but did not respond to session/new within 30s.",
                    self.agent_kind.display_name()
                )),
                cx,
            );
        }

        // ACP chunks / tool-call updates land via the push-based
        // `_event_task` -- no polling here.
        true
    }

    /// US-102: write the current composer state (text + attachments)
    /// to the `drafts` table when the debounce deadline has elapsed.
    /// No-op when no deadline is armed or the deadline is still in
    /// the future, so the cost on idle ticks is one [`Instant::now`].
    fn maybe_flush_draft(&mut self, cx: &mut Context<Self>) {
        let Some(deadline) = self.draft_flush_deadline else {
            return;
        };
        if std::time::Instant::now() < deadline {
            return;
        }
        self.draft_flush_deadline = None;

        let (Some(id), Some(store)) = (self.draft_thread_id.as_ref(), self.draft_store.as_ref())
        else {
            return;
        };
        let text = self.text_area.read(cx).value();
        let blocks = combine_prompt(&text, &self.attachments);
        // If the blocks match the last persisted state, skip the
        // write -- happens when the user types and then undoes back
        // to a previously-saved snapshot.
        if blocks == self.draft_last_persisted {
            return;
        }
        // The actual SQLite write happens on a background executor so a
        // slow `write_draft` does not stall this 16 ms pump tick (the
        // call lands in the GPUI main thread). The eager
        // `draft_last_persisted` update keeps the in-memory dedup
        // contract correct even if the write itself fails later.
        let store = store.clone();
        let id = id.clone();
        if blocks.is_empty() {
            // The composer is now empty (cleared via backspace or
            // /clear). Treat that as "remove the draft" so the next
            // open doesn't replay a stale empty payload as text.
            cx.background_spawn(async move {
                if let Err(err) = store.delete_draft(&id) {
                    log::warn!("Composer: delete_draft (on empty) failed: {err}");
                }
            })
            .detach();
            self.draft_last_persisted.clear();
            return;
        }
        let blocks_to_persist = blocks.clone();
        cx.background_spawn(async move {
            if let Err(err) = store.write_draft(&id, &blocks_to_persist) {
                log::warn!("Composer: write_draft failed: {err}");
            }
        })
        .detach();
        self.draft_last_persisted = blocks;
    }

    /// US-102 AC #5: drop the persisted draft for the current
    /// thread. Called from both send paths so a successful prompt
    /// always clears the row -- otherwise the next open would
    /// replay the just-sent text.
    fn delete_draft_now(&mut self, cx: &mut Context<Self>) {
        self.draft_flush_deadline = None;
        self.draft_last_persisted.clear();
        let (Some(id), Some(store)) = (self.draft_thread_id.as_ref(), self.draft_store.as_ref())
        else {
            return;
        };
        // Push the delete to the background executor so a slow SQLite
        // commit doesn't stall the click handler that triggered this
        // call (typically the Send button).
        let store = store.clone();
        let id = id.clone();
        cx.background_spawn(async move {
            if let Err(err) = store.delete_draft(&id) {
                log::warn!("Composer: delete_draft failed: {err}");
            }
        })
        .detach();
    }

    /// US-019: if a mention query has been pending for at least
    /// [`MENTION_DEBOUNCE`] and not yet scanned, run the walk now.
    /// Idempotent -- subsequent ticks short-circuit because
    /// `scanned = true`.
    fn maybe_run_mention_scan(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.mention_state.as_mut() else {
            return;
        };
        if state.scanned {
            return;
        }
        if state.query_started_at.elapsed() < MENTION_DEBOUNCE {
            return;
        }
        // Walking a large cwd (huge monorepo, unfiltered node_modules)
        // can exceed the 16 ms pump tick and drop a frame. Push the
        // walk to the background executor and post results back into
        // `mention_state` only if the query string still matches —
        // otherwise the user has typed a different query and the
        // stale results would clobber the active picker.
        state.scanned = true;
        let cwd = self.cwd.clone();
        let query = state.query.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async: &mut AsyncApp| {
            let query_back = query.clone();
            let results = smol::unblock(move || scan_files(&cwd, &query)).await;
            cx_async.update(|cx| {
                let _ = weak.update(cx, |composer, cx| {
                    if let Some(state) = composer.mention_state.as_mut()
                        && state.query == query_back
                    {
                        state.results = results;
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    fn handle_runtime_event(&mut self, ev: RuntimeEvent, cx: &mut Context<Self>) {
        match ev {
            RuntimeEvent::SessionReady {
                modes,
                current_mode_id,
                models,
                current_model_id,
            } => {
                self.modes = modes;
                self.current_mode_id = current_mode_id.map(|id| id.to_string());
                self.models = models;
                self.current_model_id = current_model_id;
                // US-120: a fresh session starts with no token count;
                // the badge stays absent until the agent emits the first
                // UsageUpdate. Reset here so a thread-switch never
                // surfaces a stale count from the previous session.
                self.last_usage = None;
                // Only clear `fatal_error` once we know the respawn
                // actually reached SessionReady. Clearing it earlier (in
                // `ensure_runtime`) would mask a silent failure where
                // the new process spawned but never replied — the user
                // would see the old error vanish with no replacement.
                self.fatal_error = None;
                self.stop_status = None;
                // Clear the backoff state — the respawn made it. The
                // SessionReady-timeout deadline is also released so the
                // pump stops watching for a hung process.
                self.consecutive_fatal_count = 0;
                self.runtime_respawn_after = None;
                self.session_ready_deadline = None;
                // Invalidate the codex-model cache: a SessionReady can
                // carry a different model set (different agent or new
                // config on disk).
                self.cached_codex_models = None;
                cx.notify();
            }
            RuntimeEvent::Chunk(text) => {
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| {
                        // Always make sure the streaming pipeline has
                        // a target Text chunk to append into. Don't
                        // force-close a prior thinking burst — Zed
                        // keeps Thought + Text chunks in the SAME
                        // AssistantMessage so the panel reads as one
                        // continuous turn, not a stack of paragraphs.
                        if !tv.is_streaming() {
                            tv.begin_assistant_stream(cx);
                        }
                        tv.push_streaming_chunk(&text, cx);
                    });
                }
            }
            RuntimeEvent::Thought(text) => {
                // Route reasoning tokens to the per-burst collapsible
                // thinking block. ThreadView decides whether to open
                // a new burst (no current) or append to the existing
                // one.
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| tv.push_thinking_chunk(&text, cx));
                }
            }
            RuntimeEvent::TurnEnded(reason) => {
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| {
                        // `flush_streaming` first -- it must drain the
                        // pacing buffer into the visible text BEFORE
                        // `finalize_thinking` closes the assistant
                        // message (which nulls `streaming_message_idx`
                        // and would make `flush_streaming` a no-op).
                        // Without this ordering the last 20–30 chars
                        // of a Codex/Claude-Code turn went missing --
                        // see the trace at `flush_streaming: no active
                        // stream (idx=None)` confirmed in LOG_CODEX.txt.
                        tv.flush_streaming(cx);
                        tv.finalize_thinking(cx);
                        // Defensive sweep: some ACP backends (observed
                        // with Claude Code + Codex) end the turn
                        // without emitting a final `Completed` status
                        // update for the last tool call. Without this,
                        // `compute_activity_state` keeps finding an
                        // InProgress snapshot and the activity-bar
                        // "Reading file" spinner spins forever. See
                        // `ThreadView::sweep_pending_tools_at_turn_end`
                        // for the per-`StopReasonKind` policy.
                        tv.sweep_pending_tools_at_turn_end(reason, cx);
                    });
                }
                // Background title summarizer (Zed parity --
                // `Thread::generate_title` at thread.rs:2808). Fire
                // once on the first clean turn end, only for ACP
                // agents that don't push their own
                // `SessionInfoUpdate.title` (Codex). Best-effort: if
                // the transient session fails, the auto-derived
                // title stays.
                //
                // Robust against MaxTokens-first turns: we ONLY commit
                // `title_summarization_done = true` if the spawn
                // actually happens (we got both user + assistant text
                // and the view is still alive). When the first
                // EndTurn happens without an assistant body (e.g.
                // MaxTokens cut the response before any chunk),
                // `title_snapshot_for_summary` is re-armed so the
                // next clean turn can try again.
                if !self.title_summarization_done
                    && matches!(reason, StopReasonKind::EndTurn)
                    && let Some(snapshot) = self.title_snapshot_for_summary.take()
                {
                    let mut consumed_snapshot = false;
                    if let Some(view) = self.thread_view.upgrade() {
                        let (user_text, assistant_text) = {
                            let tv = view.read(cx);
                            (tv.first_user_text(), tv.first_assistant_text())
                        };
                        if let (Some(user), Some(assistant)) = (user_text, assistant_text) {
                            let req = crate::agents::title_summarizer::SummarizeRequest {
                                agent_kind: self.agent_kind,
                                cwd: self.cwd.clone(),
                                discovery: Arc::clone(&self.discovery),
                                user_prompt: user,
                                assistant_response: assistant,
                                title_snapshot: snapshot.clone(),
                                thread_view: view.downgrade(),
                            };
                            // Route through ThreadView so the new Task<()>
                            // replaces any in-flight summarizer (Zed parity:
                            // `pending_title_generation` guard at
                            // `crates/agent/src/thread.rs:962`). Without
                            // this, two close TurnEnded events would race
                            // two `claude -p` children.
                            view.update(cx, |tv, cx| {
                                tv.start_title_summarization(req, cx);
                            });
                            self.title_summarization_done = true;
                            consumed_snapshot = true;
                        }
                    }
                    if !consumed_snapshot {
                        // Keep the snapshot so a future clean turn can fire.
                        self.title_snapshot_for_summary = Some(snapshot);
                    }
                }
                self.is_streaming = false;
                // US-121 AC #5: clear the stopwatch so the activity-bar
                // row collapses cleanly when the spinner disappears.
                self.turn_started_at = None;
                // AC #8: partial message + "Stopped" indicator when
                // cancellation drove the turn end. The marker lives
                // on the Composer (rendered in its status row) so
                // ThreadView's history rendering stays role-only.
                // `Refusal` (US-116) and `Other` both count as
                // non-success turn-ends; neither is a crash, so we
                // clear any stale `fatal_error` left over from a
                // prior pump (e.g. a transient JSON-RPC hiccup).
                if !matches!(reason, StopReasonKind::EndTurn) {
                    self.fatal_error = None; // Stopped / refused, not an error
                }
                // Surface MaxTokens / MaxTurnRequests / Cancelled as a
                // muted status row beneath the textarea so the user
                // knows WHY the response cut off (otherwise a truncated
                // word like "ded" looks like a Paneflow bug). EndTurn /
                // Refusal map to None and clear any prior banner.
                self.stop_status = reason.status_message();
                // US-116: fire the OS notification before draining the
                // queue, so the user is alerted about THIS turn's
                // outcome even if the next prompt re-enters streaming
                // on the same event-pump tick.
                let ran_tools = self.current_turn_tool_count > 0;
                let model_label = self.current_model_id.clone();
                super::notifications::on_turn_ended(reason, ran_tools, model_label.as_deref());
                self.current_turn_tool_count = 0;
                // US-106 AC #6: drain the next queued prompt onto a
                // fresh stream. `dequeue_next_prompt` sets
                // `is_streaming` back to true on success.
                if !self.pending_prompts.is_empty() {
                    self.dequeue_next_prompt(cx);
                }
                cx.notify();
            }
            RuntimeEvent::Fatal(msg) => {
                tracing::warn!(target: "paneflow_app::agents::composer", "runtime fatal: {msg}");
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| {
                        // Drain streaming buffer BEFORE closing the
                        // assistant message. See the matching
                        // `RuntimeEvent::TurnEnded` arm above for the
                        // rationale.
                        tv.flush_streaming(cx);
                        tv.finalize_thinking(cx);
                        // Agent process crashed / pipe broke -- we
                        // can't ask the agent whether the in-flight
                        // tool calls actually finished, so mark them
                        // Failed (vs. Completed for a clean EndTurn).
                        tv.sweep_pending_tools_on_fatal(cx);
                    });
                }
                self.is_streaming = false;
                self.fatal_error = Some(msg);
                self.runtime = None; // drop the runtime so the next prompt re-spawns
                // Stop draining the (now-disconnected) event channel
                // so a fresh task gets installed on the next runtime
                // spawn instead of stacking on a closed receiver.
                self._event_task = None;
                // Stale mode/model labels from the crashed session must
                // not survive — the pickers would otherwise show entries
                // that no longer correspond to any running agent until
                // `SessionReady` of the respawn replaces them.
                self.modes.clear();
                self.current_mode_id = None;
                self.models.clear();
                // Pending prompts queued behind the crashed turn will
                // never see a `TurnEnded`, so they would silently stay
                // in the queue forever. Drop them so the user can re-send
                // explicitly; we log how many were dropped for diagnosis.
                if !self.pending_prompts.is_empty() {
                    tracing::warn!(
                        target: "paneflow_app::agents::composer",
                        dropped = self.pending_prompts.len(),
                        "dropping queued prompts after runtime Fatal — re-send manually",
                    );
                    self.pending_prompts.clear();
                }
                // If the title summarizer is still armed from a turn that
                // never reached EndTurn, releasing the snapshot lets a
                // future clean turn fire its own summarization. Without
                // this reset, `title_summarization_done == false` AND
                // `title_snapshot_for_summary == None` would silently
                // disable summarization forever.
                if !self.title_summarization_done {
                    self.title_snapshot_for_summary = None;
                }
                // Exponential backoff for repeated Fatal events: capped
                // at 30 s. A permanently-failing CLI (binary missing,
                // auth broken, port already taken) would otherwise spin
                // in a spawn/crash loop while the user is trying to
                // unjam it manually. Backoff resets to zero on a clean
                // `SessionReady`.
                self.consecutive_fatal_count = self.consecutive_fatal_count.saturating_add(1);
                if self.consecutive_fatal_count >= 2 {
                    let delay_secs: u64 = match self.consecutive_fatal_count {
                        2 => 2,
                        3 => 5,
                        4 => 10,
                        _ => 30,
                    };
                    self.runtime_respawn_after = Some(
                        std::time::Instant::now() + std::time::Duration::from_secs(delay_secs),
                    );
                }
                self.session_ready_deadline = None;
                // US-116 AC #5: error-stop fires a warning notification
                // when the panel is not focused.
                super::notifications::on_fatal();
                cx.notify();
            }
            // US-017: route tool-call events into ThreadView's
            // timeline. Visible chunk reveal (`flush_streaming`)
            // continues to run in parallel: messages and tool calls
            // share the list item space but the streaming pipeline
            // anchors itself to a message index, so interleaved
            // ToolCall items don't corrupt the in-progress reveal.
            RuntimeEvent::ToolCall(snap) => {
                // US-116: count each tool the agent invokes so the
                // turn-end notification can switch between "Finished
                // running tools" and "New message" without poking
                // ThreadView for its item list.
                self.current_turn_tool_count = self.current_turn_tool_count.saturating_add(1);
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| tv.add_tool_call(snap, cx));
                }
            }
            RuntimeEvent::ToolCallUpdate(patch) => {
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| tv.update_tool_call(patch, cx));
                }
            }
            // US-018 AC #1: a permission request lands on the
            // ThreadView's matching tool-call snapshot, which the
            // renderer (tool_call_view) reads to surface the
            // Allow / Deny buttons.
            RuntimeEvent::PermissionRequest {
                tool_call_id,
                options,
            } => {
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |tv, cx| {
                        tv.set_pending_permission(&tool_call_id, options, cx)
                    });
                }
            }
            // US-112: cache the agent's slash commands so the next
            // `/` keystroke surfaces them alongside built-ins. If the
            // popup is already open, refresh its filtered results in
            // place so the new entries appear without a re-trigger.
            RuntimeEvent::AvailableCommandsUpdate(commands) => {
                self.agent_slash_commands =
                    commands.iter().map(agent_slash_command_from_acp).collect();
                if let Some(state) = self.slash_state.as_mut() {
                    state.results = merge_and_filter_slash_commands(
                        &built_in_slash_commands(),
                        &self.agent_slash_commands,
                        &state.query,
                    );
                }
                tracing::info!(
                    target: "paneflow_app::agents::composer",
                    count = self.agent_slash_commands.len(),
                    "available_commands_update",
                );
                cx.notify();
            }
            RuntimeEvent::SessionTitle(title) => {
                // Agent-pushed summary -- always applies, overrides
                // any client-side auto-derive (`only_if_default = false`).
                if let Some(view) = self.thread_view.upgrade() {
                    view.update(cx, |_tv, cx| {
                        cx.emit(crate::agents::thread_view::TitleSuggested {
                            title: title.clone(),
                            policy: crate::agents::thread_view::TitleReplacePolicy::Always,
                        });
                    });
                }
            }
            // US-120 / US-121: stash the latest token count so the next
            // render emits the badge + (if elapsed >= 3s) the throughput
            // suffix in the activity bar. Today no shipping ACP wrapper
            // emits this notification -- the path stays cold until one
            // opts in, then lights up with no further plumbing change.
            RuntimeEvent::UsageUpdate { used, size } => {
                self.last_usage = Some((used, size));
                cx.notify();
            }
        }
    }

    /// US-018: forward an Allow / Deny decision to the running
    /// `SessionRuntime`. The ACP `BrokerCallback` future completes
    /// inside `paneflow_acp::map_decision` which then returns the
    /// outcome to the agent.
    pub fn send_permission_decision(
        &mut self,
        tool_call_id: String,
        decision: paneflow_acp::PermissionDecision,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.resolve_permission(tool_call_id, decision);
        }
    }

    /// Ensure a runtime is alive for the current agent. Spawns one
    /// lazily on the first prompt -- avoids burning a process when
    /// the user just opens a thread without sending anything.
    fn ensure_runtime(&mut self, cx: &mut Context<Self>) -> bool {
        if self.runtime.is_some() {
            return true;
        }
        // Backoff fence: after repeated Fatals we hold off spawning so
        // a broken CLI does not eat the user's CPU in a tight loop.
        if let Some(deadline) = self.runtime_respawn_after
            && std::time::Instant::now() < deadline
        {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            self.fatal_error = Some(format!(
                "{} kept crashing — waiting {}s before retrying.",
                self.agent_kind.display_name(),
                remaining.as_secs().max(1),
            ));
            cx.notify();
            return false;
        }
        let agents = self.discovery.list();
        let Some(discovered) = agents.iter().find(|a| a.kind == self.agent_kind) else {
            self.fatal_error = Some(format!(
                "{} is not available on PATH -- install it and refocus to retry.",
                self.agent_kind.display_name()
            ));
            cx.notify();
            return false;
        };
        let opts = SpawnOptions {
            spawn_command: discovered.spawn_command.clone(),
            cwd: self.cwd.clone(),
            // US-018: `None` installs the interactive PermissionBroker
            // so each `RequestPermissionRequest` lands on the
            // Composer's Allow / Deny buttons.
            permission_callback: None,
            // US-018: `None` installs the real
            // `AgentTerminalSpawner` (portable-pty) so the agent's
            // `terminal/create` requests run actual processes.
            terminal_spawner: None,
            // US-118: `None` keeps today's behaviour -- the runtime
            // calls `session/new` and the ThreadView replays its
            // persisted blob locally. A future wiring point will pass
            // the persisted ACP `session_id` here so the runtime can
            // take the `session/load` path when the backend advertises
            // the `supports_load_session` capability.
            resume_session_id: None,
        };
        let mut runtime = SessionRuntime::spawn(opts);
        // Push-based event flow: hand the receiver to a `cx.spawn`
        // task that awaits on it. The task wakes the moment the ACP
        // side sends a chunk -- no 16 ms poll wait. Mirrors Zed's
        // `terminal.rs:701` pattern. The task drains every queued
        // event in one entity update per wake so a burst of 50+ chunks
        // collapses into a single repaint instead of 50 notifies.
        if let Some(event_rx) = runtime.take_event_receiver() {
            self.start_event_task(event_rx, cx);
        }
        self.runtime = Some(runtime);
        // Arm a SessionReady deadline: if the CLI binds + spawns but
        // never replies to `session/new` (interactive auth prompt,
        // hung child, network stall), the pump will synthesize a
        // `Fatal` once 30 s elapse, surfacing the failure instead of
        // leaving the picker pills stuck on placeholders forever.
        self.session_ready_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
        // NOTE: `fatal_error` and `stop_status` are cleared in the
        // `SessionReady` handler instead of here. If the spawned
        // process is alive but never replies, clearing them on spawn
        // would hide the previous error before the new failure
        // surfaces.
        true
    }

    /// US-020 (fork flow): send `text` to the ACP session without
    /// re-echoing it as a user message in the ThreadView's timeline.
    /// The fork handler has already persisted the edited message
    /// into the new thread's history via
    /// `ThreadStore::replace_messages`, so the freshly-mounted
    /// ThreadView reads the user message from disk -- echoing it
    /// again here would duplicate the row.
    ///
    /// Mirrors the streaming-side of [`Self::send_prompt`] (ensure
    /// runtime, begin assistant stream, dispatch prompt) but skips
    /// the `send_user_message_blocks` call.
    pub fn send_prompt_no_echo(&mut self, text: String, cx: &mut Context<Self>) {
        let blocks = combine_prompt(&text, &self.attachments);
        if blocks.is_empty() {
            return;
        }
        if self.is_streaming {
            return;
        }
        if !self.ensure_runtime(cx) {
            return;
        }
        // Defensive: `ensure_runtime(cx) == true` implies `self.runtime.is_some()`,
        // but a future refactor could break the invariant without surfacing
        // here. Early-return is safer than `.expect()` which would panic in
        // release builds. Same guard applied at every other call site below.
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if let Some(view) = self.thread_view.upgrade() {
            // Mirror `send_prompt`: if this is the very first prompt of
            // the (forked) thread, arm the background title summarizer
            // with the current sidebar title snapshot. Without this,
            // a fork that produces the first clean TurnEnded would
            // have `title_summarization_done = false` AND
            // `title_snapshot_for_summary = None`, silently disabling
            // the LLM-generated title forever for that fork.
            let is_first_prompt = view.read(cx).items_is_empty();
            if is_first_prompt && self.title_snapshot_for_summary.is_none() {
                self.title_snapshot_for_summary = Some("New thread".to_string());
            }
            view.update(cx, |tv, cx| {
                tv.begin_assistant_stream(cx);
            });
        }
        runtime.send_prompt_blocks(blocks);
        self.post_send_cleanup(cx);
    }

    /// Shared post-send state mutation block. Used by `send_prompt`,
    /// `send_prompt_immediate` and `send_prompt_no_echo` so the three
    /// paths can't silently drift out of sync (turn-start stamp,
    /// status row clears, popup dismiss, draft deletion, deferred
    /// TextArea clear, repaint).
    fn post_send_cleanup(&mut self, cx: &mut Context<Self>) {
        self.is_streaming = true;
        // US-121 AC #1: stamp the turn-start instant so the activity
        // bar can compute elapsed time without polling a clock.
        self.turn_started_at = Some(std::time::Instant::now());
        self.fatal_error = None;
        self.stop_status = None;
        self.attach_error = None;
        self.attachments.clear();
        self.dismiss_popups(cx);
        // US-102 AC #5: the prompt has left -- drop the persisted
        // draft so the next open of this thread doesn't replay it.
        self.delete_draft_now(cx);
        // Defer the TextArea clear so its `on_change` callback can
        // run without re-entering the Composer's current `update()`
        // frame.
        let text_area = self.text_area.clone();
        cx.defer(move |cx| {
            text_area.update(cx, |ta, cx| ta.clear(cx));
        });
        cx.notify();
    }

    /// AC #1/#2/#9: handle a Send keystroke or button click. Empty
    /// input is a no-op (already guarded at the textarea level, but
    /// the second guard here protects programmatic callers).
    ///
    /// US-019: the prompt now bundles the text WITH any pending
    /// attachments (image / resource link chips). A prompt with
    /// zero text but at least one attachment is allowed -- the user
    /// pressed Send after adding an image, with no caption.
    pub fn send_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        let blocks = combine_prompt(&text, &self.attachments);
        if blocks.is_empty() {
            return;
        }
        if self.is_streaming {
            // US-106 AC #5: state 4 (streaming + content) enqueues
            // rather than dropping the prompt. The queued prompt
            // automatically dequeues on `RuntimeEvent::TurnEnded`.
            self.pending_prompts.push_back(PendingPrompt {
                blocks: blocks.clone(),
            });
            self.attach_error = None;
            self.attachments.clear();
            self.dismiss_popups(cx);
            // The persisted draft has effectively been "sent" from the
            // user's point of view, even though it sits in the queue.
            // Drop the on-disk row so an app restart does not double-
            // queue.
            self.delete_draft_now(cx);
            let text_area = self.text_area.clone();
            cx.defer(move |cx| {
                text_area.update(cx, |ta, cx| ta.clear(cx));
            });
            cx.notify();
            return;
        }
        if !self.ensure_runtime(cx) {
            return;
        }
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if let Some(view) = self.thread_view.upgrade() {
            let blocks_for_view = blocks.clone();
            // Client-side title auto-derive: if this is the very first
            // user prompt of the thread (no items yet), peel a short
            // label from the prompt text and emit `TitleSuggested`
            // with `OnlyIfDefault` so the sidebar row renames
            // immediately without waiting for an ACP `SessionInfoUpdate`
            // (Claude Code 0.16 / Codex 0.14 don't emit those).
            let is_first_prompt = view.read(cx).items_is_empty();
            let auto_title = if is_first_prompt {
                derive_title_from_prompt(&text)
            } else {
                None
            };
            // Snapshot the title we just steered the sidebar to (or
            // the literal default if derive returned None). The
            // background title summarizer (kicked off at first
            // `TurnEnded`) ships this back via
            // `TitleReplacePolicy::OnlyIfStillEqualTo` so a user
            // rename during the transient session round-trip is
            // preserved.
            if is_first_prompt {
                self.title_snapshot_for_summary = Some(
                    auto_title
                        .clone()
                        .unwrap_or_else(|| "New thread".to_string()),
                );
            }
            view.update(cx, |tv, cx| {
                tv.send_user_message_blocks(blocks_for_view, cx);
                tv.begin_assistant_stream(cx);
                if let Some(title) = auto_title {
                    cx.emit(crate::agents::thread_view::TitleSuggested {
                        title,
                        policy: crate::agents::thread_view::TitleReplacePolicy::OnlyIfDefault,
                    });
                }
            });
        }
        runtime.send_prompt_blocks(blocks);
        self.post_send_cleanup(cx);
    }

    /// US-106 AC #8: bypass the queue and send `text` immediately.
    /// Cancels the in-flight turn (if any) so the new prompt lands on
    /// the next available send window. The queue is preserved -- any
    /// previously-queued prompts will still run after the immediate
    /// one wraps up.
    pub fn send_prompt_immediate(&mut self, text: String, cx: &mut Context<Self>) {
        let blocks = combine_prompt(&text, &self.attachments);
        if blocks.is_empty() {
            return;
        }
        // Cancel the in-flight turn so the runtime can accept a new
        // prompt. `cancel` is idempotent against an idle runtime, so
        // the only effect when not streaming is a no-op + the local
        // `is_streaming` flag bookkeeping below.
        if self.is_streaming
            && let Some(runtime) = self.runtime.as_ref()
        {
            runtime.cancel();
        }
        if !self.ensure_runtime(cx) {
            return;
        }
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if let Some(view) = self.thread_view.upgrade() {
            let blocks_for_view = blocks.clone();
            // Client-side title auto-derive: if this is the very first
            // user prompt of the thread (no items yet), peel a short
            // label from the prompt text and emit `TitleSuggested`
            // with `OnlyIfDefault` so the sidebar row renames
            // immediately without waiting for an ACP `SessionInfoUpdate`
            // (Claude Code 0.16 / Codex 0.14 don't emit those).
            let is_first_prompt = view.read(cx).items_is_empty();
            let auto_title = if is_first_prompt {
                derive_title_from_prompt(&text)
            } else {
                None
            };
            // Snapshot the title we just steered the sidebar to (or
            // the literal default if derive returned None). The
            // background title summarizer (kicked off at first
            // `TurnEnded`) ships this back via
            // `TitleReplacePolicy::OnlyIfStillEqualTo` so a user
            // rename during the transient session round-trip is
            // preserved.
            if is_first_prompt {
                self.title_snapshot_for_summary = Some(
                    auto_title
                        .clone()
                        .unwrap_or_else(|| "New thread".to_string()),
                );
            }
            view.update(cx, |tv, cx| {
                tv.send_user_message_blocks(blocks_for_view, cx);
                tv.begin_assistant_stream(cx);
                if let Some(title) = auto_title {
                    cx.emit(crate::agents::thread_view::TitleSuggested {
                        title,
                        policy: crate::agents::thread_view::TitleReplacePolicy::OnlyIfDefault,
                    });
                }
            });
        }
        runtime.send_prompt_blocks(blocks);
        self.post_send_cleanup(cx);
    }

    /// US-106 AC #6: pop the next queued prompt and dispatch it. Called
    /// from the `RuntimeEvent::TurnEnded` arm so the queue drains as
    /// each turn ends. Returns the number of prompts still queued so
    /// the caller (pump) can decide whether to keep `is_streaming` true.
    fn dequeue_next_prompt(&mut self, cx: &mut Context<Self>) -> usize {
        let Some(prompt) = self.pending_prompts.pop_front() else {
            return 0;
        };
        let remaining = self.pending_prompts.len();
        // Mirror the streaming side of `send_prompt`. The blocks were
        // already baked when the user enqueued them, so re-running
        // `combine_prompt` would be redundant and would walk an
        // empty textarea.
        if !self.ensure_runtime(cx) {
            // Runtime spawn failed -- re-enqueue at the front so the
            // user's prompt is not silently dropped.
            self.pending_prompts.push_front(prompt);
            return remaining + 1;
        }
        // Without a live ThreadView the prompt would stream a reply that
        // the user can never see. Re-enqueue so a re-mount picks it up,
        // and surface a fatal_error so we don't get stuck "streaming".
        let Some(view) = self.thread_view.upgrade() else {
            self.pending_prompts.push_front(prompt);
            self.is_streaming = false;
            self.fatal_error = Some(
                "Thread view unavailable while dequeuing prompts — open the thread to retry."
                    .to_string(),
            );
            tracing::warn!(
                target: "paneflow_app::agents::composer",
                "dequeue_next_prompt: thread_view dropped — re-queued prompt",
            );
            return remaining + 1;
        };
        // Same defensive guard as the other call sites: re-enqueue the
        // prompt and bail with a logged warning if the runtime disappeared
        // between `ensure_runtime` and here. Better to retry on the next
        // tick than to panic the main thread mid-stream.
        let Some(runtime) = self.runtime.as_ref() else {
            self.pending_prompts.push_front(prompt);
            tracing::warn!(
                target: "paneflow_app::agents::composer",
                "dequeue_next_prompt: runtime missing after ensure_runtime — re-queued prompt",
            );
            return remaining + 1;
        };
        let blocks_for_view = prompt.blocks.clone();
        view.update(cx, |tv, cx| {
            tv.send_user_message_blocks(blocks_for_view, cx);
            tv.begin_assistant_stream(cx);
        });
        runtime.send_prompt_blocks(prompt.blocks);
        self.is_streaming = true;
        // US-121 AC #1: stamp the turn-start instant so the activity
        // bar can compute elapsed time without polling a clock.
        self.turn_started_at = Some(std::time::Instant::now());
        self.fatal_error = None;
        // Clear any stale "Response truncated" / "Cancelled" banner
        // from the previous turn so the new prompt starts on a clean
        // status row.
        self.stop_status = None;
        remaining
    }

    /// US-107: number of prompts queued behind the in-flight turn.
    /// Read by the activity bar so it can render "N prompts queued".
    pub fn pending_prompts_len(&self) -> usize {
        self.pending_prompts.len()
    }

    /// Working directory used to spawn the underlying ACP agent. Used
    /// by the message renderer to resolve relative markdown link paths
    /// (`[foo](src/foo.rs)`) into absolute file URIs that
    /// `cx.open_url` can pass to `xdg-open`.
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// US-107: read-only access for the activity bar's "tool running"
    /// surface. `true` while a turn is in-flight (between send and
    /// `TurnEnded` / `Fatal`).
    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }

    /// US-120 / US-121: cumulative token usage reported by the agent
    /// via ACP `session/usage_update`. `None` until the first update
    /// arrives (which is "never" under today's shipping ACP wrappers,
    /// per the divergence note on `RuntimeEvent::UsageUpdate`).
    pub fn last_usage(&self) -> Option<(u64, u64)> {
        self.last_usage
    }

    /// US-121: instant the current in-flight turn was sent. `None`
    /// outside of a turn (the field is cleared on `TurnEnded` /
    /// `Fatal`). The activity bar reads this to decide whether the
    /// 3s threshold for the elapsed-time suffix has been crossed.
    pub fn turn_started_at(&self) -> Option<std::time::Instant> {
        self.turn_started_at
    }

    /// US-106 AC #7 / US-107: drop every queued prompt. Triggered by
    /// the "x" close affordance in the activity bar -- the only
    /// path that drops the queue. Cancellation alone preserves it so
    /// the user can stop the current turn and have the queue take over.
    pub fn clear_pending_prompts(&mut self, cx: &mut Context<Self>) {
        if self.pending_prompts.is_empty() {
            return;
        }
        self.pending_prompts.clear();
        cx.notify();
    }

    /// AC #3/#8: Stop button -> `session/cancel` + flush the
    /// streaming buffer immediately.
    pub fn stop_streaming(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        runtime.cancel();
        // Defer the `flush_streaming` to the next tick. `flush_streaming`
        // runs a synchronous SQLite write via `persist_snapshot_now` —
        // running it inside this `Composer::update` would chain a disk
        // I/O onto the click handler's frame and contend with any
        // chunk that arrives on the same pump tick. Letting GPUI hand
        // the lock back first keeps the click handler short.
        let view = self.thread_view.clone();
        cx.defer(move |cx| {
            if let Some(view) = view.upgrade() {
                view.update(cx, |tv, cx| tv.flush_streaming(cx));
            }
        });
        // We leave `is_streaming = true` until TurnEnded confirms;
        // the agent still owes a stop_reason notification, so the
        // Stop button stays visible to absorb a double-click.
        cx.notify();
    }

    /// AC #4 (agent picker) + AC #7 (new ACP session on switch).
    pub fn select_agent(&mut self, new_kind: AgentKind, cx: &mut Context<Self>) {
        if new_kind == self.agent_kind {
            self.show_agent_picker = false;
            cx.notify();
            return;
        }
        // Drop the Codex-models cache: the new agent has a different
        // model list (or no list, in Claude's case).
        self.cached_codex_models = None;
        self.agent_kind = new_kind;
        // Drop the existing runtime so the new agent gets a fresh
        // ACP session. Clear everything that's CLI-specific so the
        // pills don't show stale labels during the spawn window.
        self.runtime = None;
        // Tear down the event drain task tied to the old receiver --
        // ensure_runtime on the new agent will start a fresh task
        // bound to the new SessionRuntime's receiver.
        self._event_task = None;
        self.modes.clear();
        self.current_mode_id = None;
        self.models.clear();
        self.is_streaming = false;
        self.fatal_error = None;
        // Clear any stale "Response truncated" / "Cancelled" banner
        // from the previous turn so the new prompt starts on a clean
        // status row.
        self.stop_status = None;
        // Close *every* open popup, not just the agent picker — slash
        // menu, attach menu, mention list and the sibling pills would
        // otherwise dangle over a freshly-swapped agent surface.
        self.dismiss_popups(cx);
        // US-112 AC #4: the agent picker mid-thread swap means the
        // old agent's slash commands are no longer authoritative.
        // Drop the cache so the next `/` keystroke shows only
        // built-ins until the new session's
        // `available_commands_update` arrives (within ~200ms of
        // session ready).
        self.agent_slash_commands.clear();
        // Re-hydrate the pills from the new agent's settings file.
        // Effort: both Claude and Codex have a persisted value.
        // Model: only Codex has it on disk — Claude waits for ACP.
        self.current_effort = hydrate_effort_for(new_kind);
        self.current_model_id = hydrate_model_for(new_kind);
        // Spawn the new agent eagerly so SessionReady arrives and
        // the model/mode pickers populate without waiting for the
        // user to click a pill or send a prompt.
        let _ = self.ensure_runtime(cx);
        cx.notify();
    }

    /// AC #6 (mode picker). Routes through
    /// [`SessionRuntime::set_mode`] so the change lands on the
    /// running session.
    pub fn select_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        self.current_mode_id = Some(mode_id.clone());
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.set_mode(mode_id.into());
        }
        self.show_mode_picker = false;
        cx.notify();
    }

    /// Model picker selection. Routes through
    /// [`SessionRuntime::set_model`] so the change lands on the
    /// running ACP session — mirrors [`Self::select_mode`]. The CLI
    /// applies the new model on subsequent prompts; the next
    /// `SessionReady`/Models tick refreshes `current_model_id`.
    pub fn select_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        self.current_model_id = Some(model_id.clone());
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.set_model(model_id.clone().into());
        }
        // Persist back to the CLI's settings file so the next agent
        // invocation reads the new model. No-op for CLIs without a
        // known persistence path.
        persist_model(self.agent_kind, &model_id);
        self.show_model_picker = false;
        cx.notify();
    }

    pub fn input_focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.text_area.read(cx).focus_handle_ref().clone()
    }

    // -----------------------------------------------------------------
    // US-115: profile picker (apply / save / delete)
    // -----------------------------------------------------------------

    /// Toggle the profile popover. Closes any sibling picker so only
    /// one popover is open at once. Resets the save-as text on close
    /// so the user does not land back in name-input mode on reopen.
    pub fn toggle_profile_picker(&mut self, cx: &mut Context<Self>) {
        self.show_profile_picker = !self.show_profile_picker;
        if self.show_profile_picker {
            self.show_agent_picker = false;
            self.show_model_picker = false;
            self.show_mode_picker = false;
            self.show_effort_picker = false;
            self.show_attach_menu = false;
        } else {
            self.profile_save_text = None;
        }
        cx.notify();
    }

    /// US-115: apply a named profile to the composer state. Non-None
    /// fields of the snapshot overwrite the matching pill; `None`
    /// fields preserve the user's current pick so a partial profile
    /// (e.g. "lock just the effort") does not blow away the rest.
    /// Per AC #6, an unknown agent (binary no longer on PATH) aborts
    /// the apply atomically -- no field change, a single inline
    /// error in `attach_error` so the popover stays useful.
    pub fn apply_profile(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(profile) = all_profiles().into_iter().find(|p| p.name == name) else {
            self.attach_error = Some(format!("Profile \"{name}\" no longer exists"));
            cx.notify();
            return;
        };

        // AC #6: validate agent availability BEFORE mutating any
        // field. If the saved agent isn't on PATH, surface a toast
        // and bail out (no state change).
        if let Some(target_agent) = profile.snapshot.agent {
            let available = self.discovery.list().iter().any(|d| d.kind == target_agent);
            if !available {
                self.attach_error = Some(format!(
                    "Agent {} is not available -- install it and refocus to retry",
                    target_agent.display_name(),
                ));
                cx.notify();
                return;
            }
        }

        // Apply each field. `select_agent` and `select_model` already
        // call `cx.notify()` internally; the final notify at the end
        // collapses multiple frames into one repaint.
        if let Some(agent) = profile.snapshot.agent {
            self.select_agent(agent, cx);
        }
        if let Some(model) = profile.snapshot.model.clone() {
            self.select_model(model, cx);
        }
        if let Some(mode) = profile.snapshot.mode.clone() {
            self.select_mode(mode, cx);
        }
        if let Some(effort) = profile.snapshot.effort {
            self.current_effort = effort;
            persist_effort(self.agent_kind, effort);
        }

        self.current_profile_name = Some(profile.name.clone());
        self.show_profile_picker = false;
        self.profile_save_text = None;
        self.attach_error = None;
        // Persist as the next-open default so a restart re-applies
        // the same profile (AC #8).
        super::panel_config::save_default_profile_to_disk(Some(profile.name));
        // Profile state may have changed (e.g. a built-in override
        // was newly applied); invalidate the picker cache.
        self.cached_profiles = None;
        cx.notify();
    }

    /// US-115: enter "save current as profile" mode. The popover's
    /// bottom row swaps to an inline text input until the user
    /// presses Enter (commit) or Escape (cancel). Pre-populates with
    /// the current profile's name when one is active so the user can
    /// quickly re-save with a tweaked snapshot.
    pub fn begin_profile_save(&mut self, _w: &mut Window, cx: &mut Context<Self>) {
        let initial = self.current_profile_name.clone().unwrap_or_default();
        self.profile_save_text = Some(initial);
        // Focus the input so key strokes route through
        // `handle_profile_name_key` instead of the textarea.
        cx.notify();
    }

    /// Cancel an in-flight save-as without writing.
    pub fn cancel_profile_save(&mut self, cx: &mut Context<Self>) {
        self.profile_save_text = None;
        cx.notify();
    }

    /// Commit an in-flight save-as. Trims the name; empty / whitespace
    /// names abort the commit silently (the user can keep typing).
    pub fn commit_profile_save(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.profile_save_text.take() else {
            return;
        };
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            // Re-stash the empty string so the input stays open and
            // the user does not lose visual continuity.
            self.profile_save_text = Some(String::new());
            return;
        }
        let snapshot = self.current_snapshot_for_save();
        let mut map = super::panel_config::active_profiles();
        map.insert(trimmed.clone(), snapshot.to_config());
        super::panel_config::save_profiles_to_disk(map);
        self.current_profile_name = Some(trimmed.clone());
        super::panel_config::save_default_profile_to_disk(Some(trimmed));
        self.show_profile_picker = false;
        self.cached_profiles = None;
        cx.notify();
    }

    /// Capture the composer's current pill state as a snapshot ready
    /// for serialisation. Tools list is pulled from the active
    /// `tool_permissions` map (every kind that is NOT in `always_deny`
    /// is considered "on" for picker purposes -- a coarse proxy for
    /// US-111 since the inline pattern picker has not landed yet).
    fn current_snapshot_for_save(&self) -> ProfileSnapshot {
        let tools = super::panel_config::active_tool_permissions()
            .into_iter()
            .filter(|(_, entry)| entry.always_deny.is_empty())
            .map(|(k, _)| k)
            .collect();
        ProfileSnapshot {
            agent: Some(self.agent_kind),
            model: self.current_model_id.clone(),
            mode: self.current_mode_id.clone(),
            effort: Some(self.current_effort),
            tools,
        }
    }

    /// US-115 AC #7: delete a user-saved profile. Built-ins cannot be
    /// deleted -- the picker hides the Delete affordance for them, so
    /// callers can rely on this being a custom row; we still guard
    /// here defensively.
    pub fn delete_profile(&mut self, name: &str, cx: &mut Context<Self>) {
        let built_in_names: std::collections::HashSet<String> =
            built_in_profiles().into_iter().map(|p| p.name).collect();
        if built_in_names.contains(name) {
            self.attach_error = Some(format!("Built-in profile \"{name}\" cannot be deleted"));
            cx.notify();
            return;
        }
        let mut map = super::panel_config::active_profiles();
        if map.remove(name).is_none() {
            return;
        }
        super::panel_config::save_profiles_to_disk(map);
        if self.current_profile_name.as_deref() == Some(name) {
            self.current_profile_name = None;
            super::panel_config::save_default_profile_to_disk(None);
        }
        self.cached_profiles = None;
        cx.notify();
    }

    /// US-115: append a character to the in-flight save-as name. Used
    /// by the popover's text input key handler.
    pub fn push_profile_save_char(&mut self, ch: &str, cx: &mut Context<Self>) {
        if let Some(buf) = self.profile_save_text.as_mut() {
            buf.push_str(ch);
            cx.notify();
        }
    }

    /// US-115: pop a character from the in-flight save-as name.
    pub fn pop_profile_save_char(&mut self, cx: &mut Context<Self>) {
        if let Some(buf) = self.profile_save_text.as_mut() {
            buf.pop();
            cx.notify();
        }
    }

    // -----------------------------------------------------------------
    // US-019: attachment menu + image / file pickers + chip removal
    // -----------------------------------------------------------------

    /// Toggle the `+` button's popover open / closed.
    pub fn toggle_attach_menu(&mut self, cx: &mut Context<Self>) {
        self.show_attach_menu = !self.show_attach_menu;
        if self.show_attach_menu {
            self.show_agent_picker = false;
            self.show_model_picker = false;
            self.show_mode_picker = false;
        }
        cx.notify();
    }

    /// Open the native image picker (rfd). On success, base64-encode
    /// the bytes and add a chip. Enforces the 10 MB cap from the AC.
    pub fn open_image_picker(&mut self, cx: &mut Context<Self>) {
        self.show_attach_menu = false;
        let start_dir = self.cwd.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async| {
            let dialog = rfd::AsyncFileDialog::new()
                .set_title("Attach image")
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif"])
                .set_directory(&start_dir);
            let Some(handle) = dialog.pick_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            // Read the image off-thread — a 10 MB file on a slow disk
            // would otherwise stall the GPUI main thread for the full
            // duration of the read.
            let read_path = path.clone();
            let read_result = smol::unblock(move || std::fs::read(&read_path)).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            cx_async.update(|cx| {
                this.update(cx, |composer, cx| match read_result {
                    Ok(bytes) => composer.complete_image_attach(path, bytes, cx),
                    Err(err) => {
                        composer.attach_error = Some(format!("Couldn't read image: {err}"));
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    /// Encode the already-read image bytes and stash a chip. Called
    /// from the async picker / drop pipelines so the disk read never
    /// runs on the GPUI main thread.
    fn complete_image_attach(&mut self, path: PathBuf, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            self.attach_error = Some(attachment_limit_message());
            cx.notify();
            return;
        }
        if bytes.len() as u64 > super::composer_ext::MAX_IMAGE_BYTES {
            self.attach_error = Some(image_too_large_message());
            cx.notify();
            return;
        }
        let mime = detect_image_mime(&path).unwrap_or("application/octet-stream");
        let Some(block) = image_block_from_bytes(&bytes, mime) else {
            self.attach_error = Some(image_too_large_message());
            cx.notify();
            return;
        };
        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.display().to_string());
        self.attachments.push(PendingAttachment {
            label,
            kind: AttachmentKind::Image,
            block,
        });
        self.attach_error = None;
        self.refresh_has_content_cache(cx);
        cx.notify();
    }

    /// Open the native file picker (rfd) for the "Attach file"
    /// action. The picked path becomes a `ContentBlock::ResourceLink`
    /// the agent can read via `ReadTextFile` (US-018).
    pub fn open_file_picker(&mut self, cx: &mut Context<Self>) {
        self.show_attach_menu = false;
        let start_dir = self.cwd.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async| {
            let dialog = rfd::AsyncFileDialog::new()
                .set_title("Attach file")
                .set_directory(&start_dir);
            let Some(handle) = dialog.pick_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            let Some(this) = weak.upgrade() else {
                return;
            };
            cx_async.update(|cx| {
                this.update(cx, |composer, cx| {
                    composer.complete_file_attach(path, cx);
                })
            });
        })
        .detach();
    }

    fn complete_file_attach(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            self.attach_error = Some(attachment_limit_message());
            cx.notify();
            return;
        }
        let block = resource_block_for_path(&path);
        let label = label_for_path(&path, &self.cwd);
        self.attachments.push(PendingAttachment {
            label,
            kind: AttachmentKind::File,
            block,
        });
        self.attach_error = None;
        self.refresh_has_content_cache(cx);
        cx.notify();
    }

    /// US-114: route a batch of dropped OS paths through the existing
    /// attachment plumbing. Each path is classified at drop time:
    /// - directory  -> rejected with a single combined toast (AC #3);
    /// - image ext  -> read into memory, gated on the 10 MB cap, and
    ///   pushed as an `Image` chip; over-cap images surface a warning
    ///   and are skipped (AC #2);
    /// - anything else -> pushed as a `File` chip referencing the
    ///   absolute path (AC #6 -- worktree membership is not enforced).
    ///
    /// Errors are aggregated into one `attach_error` line so a multi-
    /// file drop that contains a mix of valid + invalid entries does
    /// not flood the composer with multiple toasts. Cross-platform by
    /// construction: the underlying file APIs are `std::fs` only and
    /// GPUI's drop event abstracts away Wayland / X11 / macOS / Windows
    /// differences (AC #7).
    pub fn complete_drop_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        // Phase 1 (sync, main thread): classify each path and apply
        // anything that doesn't need I/O — folders, file resource
        // links, classification failures. Image paths are collected
        // for the async read below.
        let mut errors: Vec<String> = Vec::new();
        let mut folder_count = 0_usize;
        let mut skipped_for_limit = 0_usize;
        let mut images_to_read: Vec<(PathBuf, &'static str)> = Vec::new();
        let cwd_snapshot = self.cwd.clone();
        for path in paths {
            if self.attachments.len() + images_to_read.len() >= MAX_ATTACHMENTS {
                skipped_for_limit += 1;
                continue;
            }
            match classify_dropped_path(&path) {
                DropClassification::DirectoryRejected => {
                    folder_count += 1;
                }
                DropClassification::Unreadable { reason } => {
                    errors.push(format!(
                        "Couldn't read {}: {reason}",
                        path.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("dropped path"),
                    ));
                }
                DropClassification::Image { mime } => {
                    images_to_read.push((path, mime));
                }
                DropClassification::File => {
                    let block = resource_block_for_path(&path);
                    let label = label_for_path(&path, &cwd_snapshot);
                    self.attachments.push(PendingAttachment {
                        label,
                        kind: AttachmentKind::File,
                        block,
                    });
                }
            }
        }
        if folder_count > 0 {
            errors.push(if folder_count == 1 {
                "Folders are not supported as attachments".to_string()
            } else {
                format!("{folder_count} folders skipped (folders are not supported as attachments)")
            });
        }
        if skipped_for_limit > 0 {
            errors.push(format!(
                "{skipped_for_limit} attachment(s) skipped — limit of {MAX_ATTACHMENTS} reached",
            ));
        }
        // Surface the synchronous-phase errors first so the user sees
        // folder/limit feedback immediately. Image-read errors will be
        // merged in once the async reads complete.
        self.attach_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join(" · "))
        };
        self.refresh_has_content_cache(cx);
        cx.notify();

        // Phase 2 (async, background): read all dropped images off
        // the GPUI main thread. A 9 MB drop on a slow disk would
        // otherwise jank the textarea for the duration of the read.
        if images_to_read.is_empty() {
            return;
        }
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async: &mut AsyncApp| {
            let cwd_for_async = cwd_snapshot.clone();
            // Read every image off-thread and pair with its mime.
            let results = smol::unblock(move || {
                images_to_read
                    .into_iter()
                    .map(|(path, mime)| {
                        let bytes = std::fs::read(&path);
                        (path, mime, bytes)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            cx_async.update(|cx| {
                let _ = weak.update(cx, |composer, cx| {
                    let mut extra_errors: Vec<String> = Vec::new();
                    for (path, mime, read) in results {
                        if composer.attachments.len() >= MAX_ATTACHMENTS {
                            extra_errors.push(format!(
                                "{}: attachment limit reached",
                                label_for_path(&path, &cwd_for_async)
                            ));
                            continue;
                        }
                        let bytes = match read {
                            Ok(b) => b,
                            Err(err) => {
                                extra_errors.push(format!(
                                    "Couldn't read image {}: {err}",
                                    label_for_path(&path, &cwd_for_async)
                                ));
                                continue;
                            }
                        };
                        if bytes.len() as u64 > super::composer_ext::MAX_IMAGE_BYTES {
                            extra_errors.push(format!(
                                "{}: {}",
                                label_for_path(&path, &cwd_for_async),
                                image_too_large_message(),
                            ));
                            continue;
                        }
                        let Some(block) = image_block_from_bytes(&bytes, mime) else {
                            extra_errors.push(format!(
                                "{}: {}",
                                label_for_path(&path, &cwd_for_async),
                                image_too_large_message(),
                            ));
                            continue;
                        };
                        let label = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| path.display().to_string());
                        composer.attachments.push(PendingAttachment {
                            label,
                            kind: AttachmentKind::Image,
                            block,
                        });
                    }
                    if !extra_errors.is_empty() {
                        let prefix = composer
                            .attach_error
                            .clone()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!("{s} · "))
                            .unwrap_or_default();
                        composer.attach_error =
                            Some(format!("{prefix}{}", extra_errors.join(" · ")));
                    }
                    composer.refresh_has_content_cache(cx);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// AC: "Insert file from current project tree" -- opens the file
    /// picker rooted at the thread cwd and splices the relative path
    /// into the textarea at the cursor (as plain text, no chip).
    /// Distinct from "Attach file" which adds a `ContentBlock` chip.
    pub fn open_project_file_inserter(&mut self, cx: &mut Context<Self>) {
        self.show_attach_menu = false;
        let start_dir = self.cwd.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async| {
            let dialog = rfd::AsyncFileDialog::new()
                .set_title("Insert file path")
                .set_directory(&start_dir);
            let Some(handle) = dialog.pick_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            let Some(this) = weak.upgrade() else {
                return;
            };
            cx_async.update(|cx| {
                this.update(cx, |composer, cx| {
                    let rel = path
                        .strip_prefix(&composer.cwd)
                        .unwrap_or(&path)
                        .to_path_buf();
                    let token = rel.display().to_string();
                    composer.insert_text_at_cursor(&token, cx);
                })
            });
        })
        .detach();
    }

    /// AC: "Insert current terminal selection (when there is an
    /// active terminal in CLI mode)". For v1 we read the system
    /// clipboard -- the terminal layer copies its selection to the
    /// clipboard on release, so the clipboard contents are the
    /// authoritative source. When the clipboard is empty we surface
    /// a hint in `attach_error` instead of silently no-oping.
    pub fn insert_terminal_selection(&mut self, cx: &mut Context<Self>) {
        self.show_attach_menu = false;
        let Some(item) = cx.read_from_clipboard() else {
            self.attach_error =
                Some("No terminal selection available (clipboard is empty)".to_string());
            cx.notify();
            return;
        };
        let Some(text) = item.text() else {
            self.attach_error =
                Some("No terminal selection available (clipboard has no text)".to_string());
            cx.notify();
            return;
        };
        if text.is_empty() {
            self.attach_error =
                Some("No terminal selection available (clipboard is empty)".to_string());
            cx.notify();
            return;
        }
        self.insert_text_at_cursor(&text, cx);
    }

    /// Remove the chip at `idx`. Bounds-safe (no panic on stale id).
    pub fn remove_attachment(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.attachments.len() {
            self.attachments.remove(idx);
            self.refresh_has_content_cache(cx);
            cx.notify();
        }
    }

    /// Splice `text` at the textarea's current cursor position
    /// (replaces any selection). Used by the project-tree inserter
    /// and the terminal-selection inserter.
    fn insert_text_at_cursor(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.text_area.update(cx, move |ta, cx| {
            ta.insert_char(&text, cx);
        });
    }

    // -----------------------------------------------------------------
    // US-019: `@` / `/` popup pick handlers
    // -----------------------------------------------------------------

    /// Pick the `idx`-th file in the current `@`-mention popup. The
    /// `@<query>` token in the textarea is replaced with `@<path> `
    /// (trailing space so the user can keep typing without a manual
    /// separator).
    pub fn pick_mention(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(state) = self.mention_state.clone() else {
            return;
        };
        let Some(rel) = state.results.get(idx).cloned() else {
            return;
        };
        let token = format!("@{} ", rel.display());
        let anchor = state.anchor;
        let cursor = anchor + 1 + state.query.len();
        // US-108b: byte range covering the `@path` token (without
        // the trailing space) so the chip overlay sits exactly on
        // the resolved mention. The trailing space is left visible
        // so the user can continue typing without a manual
        // separator -- the chip-overlay rectangle stops one byte
        // short of it.
        let token_end = anchor + 1 + rel.display().to_string().len();
        // Display the resolved file basename inside the chip; the
        // underlying token keeps the full path so the prompt
        // serializer can recover the resource.
        let chip_label = rel
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| rel.display().to_string());
        self.text_area.update(cx, |ta, cx| {
            ta.replace_range(anchor..cursor, &token, cx);
            ta.insert_decoration(anchor..token_end, chip_label);
        });
        // The replace_range fires on_change which resets
        // mention_state, but be explicit so the popup closes
        // immediately even before the change callback re-runs.
        self.mention_state = None;
        cx.notify();
    }

    /// Pick the `idx`-th slash command from the merged list.
    ///
    /// Built-in commands (US-019) run a local action inside the
    /// composer:
    /// - `/clear`: wipes the textarea content + the in-memory
    ///   transcript and dismisses the popup.
    /// - `/export`: triggers the markdown export flow (see
    ///   [`Self::run_export`]).
    ///
    /// Agent commands (US-112) are sent verbatim to the agent on the
    /// next prompt. The picker writes `/<name>` into the textarea at
    /// the cursor position; the textarea then either auto-submits
    /// (no argument required) or waits for the user to type the
    /// argument and press Enter (matches AC #2 + #5).
    pub fn pick_slash_command(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(state) = self.slash_state.clone() else {
            return;
        };
        let Some(cmd) = state.results.get(idx).cloned() else {
            return;
        };
        let anchor = state.anchor;
        let cursor = anchor + 1 + state.query.len();
        self.slash_state = None;

        match cmd.source {
            SlashCommandSource::BuiltIn => {
                // Drop the `/<query>` token before dispatching the
                // built-in action so the textarea is in a sane state
                // afterwards (matches the pre-US-112 behavior).
                self.text_area.update(cx, |ta, cx| {
                    ta.replace_range(anchor..cursor, "", cx);
                });
                match cmd.name.as_str() {
                    "clear" => {
                        if let Some(view) = self.thread_view.upgrade() {
                            view.update(cx, |tv, cx| tv.clear_local_display(cx));
                        }
                        self.text_area.update(cx, |ta, cx| ta.clear(cx));
                    }
                    "export" => {
                        self.run_export(cx);
                    }
                    _ => {}
                }
            }
            SlashCommandSource::Agent => {
                // AC #2: replace the `/<query>` partial with the full
                // `/<name>` token so the user can read what they're
                // about to send. When the command requires an argument
                // (AC #5), append a trailing space and leave the
                // textarea focused so the user can type the argument
                // and press Enter on their own schedule. Otherwise,
                // auto-submit -- a no-argument slash command is a
                // single click affordance and waiting for an extra
                // Enter would feel unfinished.
                let replacement = format!("/{}", cmd.name);
                self.text_area.update(cx, |ta, cx| {
                    ta.replace_range(anchor..cursor, &replacement, cx);
                });
                if cmd.argument_hint.is_some() {
                    // Park a trailing space so the agent's argument
                    // parser sees `/<name> <user-input>` once the user
                    // types. Cursor lands right after the space.
                    self.text_area.update(cx, |ta, cx| {
                        ta.insert_char(" ", cx);
                    });
                } else {
                    // AC #2 + AC #6: send the verbatim `/<name>` as a
                    // ContentBlock::Text. The agent handles the slash
                    // semantics natively -- we do NOT also run any
                    // built-in side effect even if the names collided
                    // (the agent's version won the merge filter).
                    let text = self.text_area.read(cx).value();
                    self.send_prompt(text, cx);
                }
            }
        }
        cx.notify();
    }

    /// `/export` implementation: ask for a destination via rfd and
    /// write the active thread as Markdown. Asynchronous; failures
    /// surface in `attach_error`.
    fn run_export(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.thread_view.upgrade() else {
            return;
        };
        let markdown = view.read(cx).export_markdown();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx_async| {
            let dialog = rfd::AsyncFileDialog::new()
                .set_title("Export thread")
                .set_file_name("thread.md")
                .add_filter("Markdown", &["md"]);
            let Some(handle) = dialog.save_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            let res = std::fs::write(&path, markdown.as_bytes());
            let Some(this) = weak.upgrade() else {
                return;
            };
            cx_async.update(|cx| {
                this.update(cx, |composer, cx| {
                    if let Err(err) = res {
                        composer.attach_error = Some(format!("Export failed: {err}"));
                    } else {
                        composer.attach_error = None;
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn render_pill(
        &self,
        id: SharedString,
        label: String,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .bg(ui.subtle)
            .text_color(ui.text)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|d| d.bg(ui.border))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .child(label)
            .child(
                svg()
                    .size(px(10.))
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| on_click(this, cx)))
            .into_any_element()
    }

    /// Model picker pill: agent-flavoured icon + model name + chevron.
    /// Chrome-less by design (no background) — sits as a trailing
    /// affordance next to the send button, blending with the composer
    /// surface. Selecting a model implicitly carries its provider
    /// (Claude vs OpenAI), surfaced via the leading brand icon.
    fn render_model_pill(
        &self,
        label: String,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("composer-pill-model")
            .px(px(6.))
            .py(px(3.))
            .rounded(px(5.))
            .text_color(ui.text)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|d| d.bg(ui.subtle))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .child(label)
            .child(
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.show_model_picker = !this.show_model_picker;
                this.show_agent_picker = false;
                this.show_mode_picker = false;
                // Lazily spawn the ACP runtime so `SessionReady`
                // populates `self.models` — otherwise the picker
                // opens onto an empty list (the runtime only spawned
                // on the first prompt send before).
                if this.show_model_picker && this.runtime.is_none() {
                    let _ = this.ensure_runtime(cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// US-106: 4-state send button.
    ///
    /// - State 1 (idle + empty): muted ghost paper-plane, disabled.
    /// - State 2 (idle + content): filled accent paper-plane, sends.
    /// - State 3 (streaming + empty): solid red Stop square, cancels.
    /// - State 4 (streaming + content): filled accent queue glyph,
    ///   enqueues. `Ctrl+Shift+Enter` (handled by the TextArea) bypasses
    ///   the queue and sends immediately.
    fn render_send_button(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        // Use the cached flag instead of re-reading the TextArea entity
        // from the render pass. The cache is kept in sync by
        // `on_text_change` (text edits) and by `refresh_has_content_cache`
        // at every attachment mutation point.
        let _ = cx; // cx no longer needed for has_content; kept for symmetry.
        let has_content = self.has_content_cached;
        let state = send_button_state(self.is_streaming, has_content);

        // All four states share the same ghost look (no bg fill,
        // muted icon, 26x26 rounded box). Only the icon, hover
        // behaviour, cursor, and click handler change. Per user
        // preference 2026-05-25: keep the colour identical to the
        // pre-input "disabled" idle so the composer never flashes a
        // saturated accent/error block as you type or stream.
        let base = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .w(px(26.))
            .h(px(26.))
            .rounded(px(5.));

        match state {
            SendButtonState::IdleEmpty => base
                .id("composer-send-disabled")
                .opacity(0.45)
                .child(
                    svg()
                        .size(px(15.))
                        .path("icons/send.svg")
                        .text_color(ui.muted),
                )
                .into_any_element(),
            SendButtonState::IdleContent => base
                .id("composer-send")
                .cursor_pointer()
                .hover(|d| d.bg(ui.subtle))
                .child(
                    svg()
                        .size(px(15.))
                        .path("icons/send.svg")
                        .text_color(ui.muted),
                )
                .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    let text = this.text_area.read(cx).value();
                    this.send_prompt(text, cx);
                }))
                .into_any_element(),
            SendButtonState::StreamingEmpty => base
                .id("composer-stop")
                .cursor_pointer()
                .bg(ui.subtle)
                .hover(|d| {
                    let ui = crate::theme::ui_colors();
                    d.bg(ui.surface)
                })
                .child(
                    svg()
                        .size(px(13.))
                        .path("icons/player-stop.svg")
                        .text_color(ui.text),
                )
                .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    this.stop_streaming(cx);
                }))
                .into_any_element(),
            SendButtonState::StreamingContent => base
                .id("composer-queue")
                .cursor_pointer()
                .hover(|d| d.bg(ui.subtle))
                .tooltip(move |_w, cx| cx.new(|_| QueueTooltip::new()).into())
                .child(
                    svg()
                        .size(px(15.))
                        .path("icons/send-plus.svg")
                        .text_color(ui.muted),
                )
                .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    let text = this.text_area.read(cx).value();
                    this.send_prompt(text, cx);
                }))
                .into_any_element(),
        }
    }

    // -----------------------------------------------------------------
    // US-019: render helpers for chips + popups + `+` button
    // -----------------------------------------------------------------

    /// Render the row of attachment chips above the textarea. Hidden
    /// when `self.attachments` is empty.
    fn render_attachment_chips(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let mut row = div().flex().flex_row().flex_wrap().gap(px(4.)).pb(px(4.));
        for (idx, att) in self.attachments.iter().enumerate() {
            let icon_path = match att.kind {
                AttachmentKind::Image => "icons/eye.svg",
                AttachmentKind::File => "icons/file-text.svg",
            };
            let chip_id: SharedString = format!("composer-chip-{idx}").into();
            let close_id: SharedString = format!("composer-chip-close-{idx}").into();
            let label = att.label.clone();
            // US-117: build the tooltip payload once per chip render. For
            // image attachments, decode the base64 lazily inside the
            // tooltip Entity so a hover that never materialises (e.g.
            // user scrolls past) costs nothing. For file attachments,
            // capture path + size now -- both are cheap and the stat
            // would otherwise repeat per hover.
            let tooltip_payload = attachment_tooltip_payload(att);
            // US-005 (visual-parity): attachment chips share the
            // mention-crease outlined style — Zed routes attachments
            // through the same `MentionCrease` component (Outlined
            // border + transparent fill). Paneflow keeps the chip
            // row above the textarea (Paneflow-specific design with
            // an inline close button) but matches the chip's outer
            // visual register so the two "below the cursor" surfaces
            // read uniformly. Acceptable divergence: no inline
            // thumbnail or byte-size sub-label — Paneflow's
            // `PendingAttachment` carries only `label` + `kind` +
            // `block`; surfacing thumb/size would need metadata work
            // outside this story.
            let chip = div()
                .id(chip_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .py(px(3.))
                .rounded(px(4.))
                .border_1()
                .border_color(ui.border)
                .text_size(px(11.))
                .text_color(ui.text)
                .child(svg().size(px(10.)).path(icon_path).text_color(ui.muted))
                .child(label)
                .child(
                    div()
                        .id(close_id)
                        .cursor_pointer()
                        .px(px(2.))
                        .text_color(ui.muted)
                        .hover(|d| d.text_color(ui.text))
                        .child("×")
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                            this.remove_attachment(idx, cx);
                        })),
                )
                .tooltip(move |_w, cx| {
                    let payload = tooltip_payload.clone();
                    cx.new(|_| AttachmentTooltip::new(payload)).into()
                });
            row = row.child(chip);
        }
        Some(row.into_any_element())
    }

    /// Render the `+` button on the left of the textarea. Clicking
    /// toggles the attach menu (see [`Self::render_attach_menu`]).
    fn render_plus_button(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("composer-plus")
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .w(px(26.))
            .h(px(26.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(ui.muted)
            .hover(|d| {
                let ui = crate::theme::ui_colors();
                d.bg(ui.subtle).text_color(ui.text)
            })
            .child(
                svg()
                    .size(px(14.))
                    .path("icons/plus.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.toggle_attach_menu(cx);
            }))
            .into_any_element()
    }

    /// Agent picker pill. Switching the active agent is the most
    /// disruptive choice in the composer — it drops the running ACP
    /// session and resets the model / mode / effort pills to whatever
    /// the new CLI advertises. Sits leftmost on the trailing cluster
    /// so it visually anchors the chain "which AI → which model →
    /// send".
    fn render_agent_pill(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        let icon_path = match self.agent_kind {
            AgentKind::ClaudeCode => "icons/claude-color.svg",
            AgentKind::Codex => "icons/openai.svg",
        };
        let label = self.agent_kind.display_name().to_string();
        div()
            .id("composer-pill-agent")
            .px(px(6.))
            .py(px(3.))
            .rounded(px(5.))
            .text_color(ui.text)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|d| d.bg(ui.subtle))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(ui.text),
            )
            .child(label)
            .child(
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.show_agent_picker = !this.show_agent_picker;
                this.show_model_picker = false;
                this.show_mode_picker = false;
                this.show_effort_picker = false;
                cx.notify();
            }))
            .into_any_element()
    }

    /// Thinking-effort pill. Independent from ACP session modes:
    /// surfaces a static Low / Medium / High / Extra High choice with
    /// a leading lightbulb icon. Mirrors the dedicated "Change
    /// Thinking Effort" affordance other polished agent UIs expose
    /// next to the composer.
    fn render_effort_pill(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        let label = self.current_effort.label();
        div()
            .id("composer-pill-effort")
            .px(px(6.))
            .py(px(3.))
            .rounded(px(5.))
            .text_color(ui.text)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|d| d.bg(ui.subtle))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/bulb.svg")
                    .text_color(ui.muted),
            )
            .child(label)
            .child(
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.show_effort_picker = !this.show_effort_picker;
                this.show_model_picker = false;
                this.show_agent_picker = false;
                this.show_mode_picker = false;
                cx.notify();
            }))
            .into_any_element()
    }

    /// Static picker panel for [`ThinkingEffort`]. Carries a small
    /// "Change Thinking Effort" header above the four options, with
    /// a check mark on the active row.
    fn render_effort_picker(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut col = div()
            .id("composer-effort-panel")
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.base)
            .text_size(px(12.))
            .text_color(ui.text);

        col = col.child(
            div()
                .px(px(6.))
                .pb(px(4.))
                .text_size(px(11.))
                .text_color(ui.muted)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Change Thinking Effort"),
        );

        for effort in ThinkingEffort::all() {
            let is_current = effort == self.current_effort;
            let row_id: SharedString = format!("effort-row-{}", effort.label()).into();
            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .when(is_current, |d| d.bg(ui.subtle))
                .hover(|d| d.bg(ui.subtle))
                .child(if is_current {
                    svg()
                        .size(px(10.))
                        .flex_none()
                        .path("icons/checks.svg")
                        .text_color(ui.text)
                        .into_any_element()
                } else {
                    div().w(px(10.)).flex_none().into_any_element()
                })
                .child(effort.label())
                .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                    this.current_effort = effort;
                    this.show_effort_picker = false;
                    // Mirror the change into the active CLI's settings
                    // (Claude → settings.json effortLevel,
                    //  Codex → config.toml model_reasoning_effort).
                    persist_effort(this.agent_kind, effort);
                    cx.notify();
                }));
            col = col.child(row);
        }

        col.into_any_element()
    }

    /// US-115: profile pill. Bookmark icon + current profile name
    /// (or "Custom" when none is active) + chevron. Sits to the
    /// left of the Send button so the user's most-recent pick is
    /// adjacent to the action that uses it.
    fn render_profile_pill(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = self
            .current_profile_name
            .clone()
            .unwrap_or_else(|| "Custom".to_string());
        div()
            .id("composer-pill-profile")
            .px(px(6.))
            .py(px(3.))
            .rounded(px(5.))
            .text_color(ui.text)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|d| d.bg(ui.subtle))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/profile.svg")
                    .text_color(ui.muted),
            )
            .child(label)
            .child(
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            )
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.toggle_profile_picker(cx);
            }))
            .into_any_element()
    }

    /// US-115: profile picker popover. Two sections (built-in vs
    /// custom), one selectable row each, and a bottom row that
    /// switches between "Save current as profile..." (default) and
    /// an inline text input when the user clicks it (save-as mode).
    /// Custom rows carry a trailing "×" delete button per AC #7;
    /// built-ins do not.
    fn render_profile_picker(
        &mut self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Hydrate the cache on first open. Hot path (60 fps while the
        // popover is visible) becomes a no-op after the initial fill;
        // invalidations happen on commit/delete/apply/agent-switch.
        if self.cached_profiles.is_none() {
            self.cached_profiles = Some(all_profiles());
        }
        let all = self.cached_profiles.clone().unwrap_or_else(all_profiles);
        let mut col = div()
            .id("composer-profile-panel")
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.base)
            .text_size(px(12.))
            .text_color(ui.text);

        col = col.child(
            div()
                .px(px(6.))
                .pb(px(4.))
                .text_size(px(11.))
                .text_color(ui.muted)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Built-in"),
        );
        for profile in all.iter().filter(|p| p.is_builtin) {
            col = col.child(self.render_profile_row(profile, ui, cx));
        }

        let customs: Vec<&ProfileSummary> = all.iter().filter(|p| !p.is_builtin).collect();
        if !customs.is_empty() {
            col = col.child(
                div()
                    .px(px(6.))
                    .pt(px(6.))
                    .pb(px(4.))
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Custom"),
            );
            for profile in customs {
                col = col.child(self.render_profile_row(profile, ui, cx));
            }
        }

        col = col.child(div().h(px(1.)).my(px(4.)).bg(ui.border));
        col = col.child(self.render_profile_save_row(ui, cx));
        col.into_any_element()
    }

    fn render_profile_row(
        &self,
        profile: &ProfileSummary,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = profile.name.clone();
        let is_current = self.current_profile_name.as_deref() == Some(name.as_str());
        let is_builtin = profile.is_builtin;
        let row_id: SharedString = format!("profile-row-{name}").into();
        let mut row = div()
            .id(row_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(is_current, |d| d.bg(ui.subtle))
            .hover(|d| d.bg(ui.subtle));

        row = row
            .child(if is_current {
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/checks.svg")
                    .text_color(ui.accent)
                    .into_any_element()
            } else {
                div().w(px(10.)).flex_none().into_any_element()
            })
            .child(div().flex_1().child(name.clone()));

        if !is_builtin {
            let delete_id: SharedString = format!("profile-row-del-{name}").into();
            let del_name = name.clone();
            row = row.child(
                div()
                    .id(delete_id)
                    .px(px(6.))
                    .text_color(ui.muted)
                    .hover(|d| d.text_color(crate::theme::ui_colors().text))
                    .child("×")
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                        this.delete_profile(&del_name, cx);
                        // Stop propagation so the parent row's
                        // click doesn't immediately re-apply the
                        // profile we just deleted.
                        cx.stop_propagation();
                    })),
            );
        }

        let pick_name = name.clone();
        row.on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
            this.apply_profile(&pick_name, cx);
        }))
        .into_any_element()
    }

    /// Bottom row of the profile picker -- swaps between the
    /// "Save current as profile..." trigger and an inline text input
    /// once the trigger is clicked. The text input owns its own
    /// focus handle so key events route through
    /// [`Self::handle_profile_name_key`] (registered on the input
    /// element directly).
    fn render_profile_save_row(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.profile_save_text.as_ref() {
            None => div()
                .id("composer-profile-save-trigger")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|d| d.bg(ui.subtle))
                .child(
                    svg()
                        .size(px(11.))
                        .flex_none()
                        .path("icons/plus.svg")
                        .text_color(ui.muted),
                )
                .child("Save current as profile...")
                .on_click(cx.listener(|this, _ev: &ClickEvent, w, cx| {
                    this.begin_profile_save(w, cx);
                }))
                .into_any_element(),
            Some(text) => {
                let display = format!("{text}|");
                div()
                    .id("composer-profile-save-input")
                    .track_focus(&self.profile_save_focus)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(ui.surface)
                    .border_1()
                    .border_color(ui.accent)
                    .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _w, cx| {
                        handle_profile_name_key(this, e, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_color(ui.text)
                            .child(if text.is_empty() {
                                "Name this profile...".to_string()
                            } else {
                                display
                            }),
                    )
                    .child(
                        div()
                            .id("composer-profile-save-commit")
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(ui.accent)
                            .text_color(rgb(0xffffff))
                            .text_size(px(11.))
                            .cursor_pointer()
                            .hover(|d| d.opacity(0.9))
                            .child("Save")
                            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                                this.commit_profile_save(cx);
                            })),
                    )
                    .into_any_element()
            }
        }
    }

    /// Render the attach popover (four actions). Returned wrapped in
    /// `Option` so the render() can `when_some` it above the card.
    fn render_attach_menu(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        let mut col = div()
            .id("composer-attach-menu")
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.overlay)
            .text_size(px(12.))
            .text_color(ui.text);

        let entries: [(SharedString, &str, &str); 4] = [
            ("attach-image".into(), "icons/eye.svg", "Attach image"),
            ("attach-file".into(), "icons/file-text.svg", "Attach file"),
            (
                "insert-terminal".into(),
                "icons/terminal.svg",
                "Insert current terminal selection",
            ),
            (
                "insert-project-file".into(),
                "icons/folder_open.svg",
                "Insert file from current project tree",
            ),
        ];

        for (id, icon, label) in entries {
            let row = div()
                .id(id.clone())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|d| d.bg(ui.subtle))
                .child(svg().size(px(12.)).path(icon).text_color(ui.muted))
                .child(label.to_string());
            let row = match id.as_ref() {
                "attach-image" => row.on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    this.open_image_picker(cx);
                })),
                "attach-file" => row.on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    this.open_file_picker(cx);
                })),
                "insert-terminal" => row.on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    this.insert_terminal_selection(cx);
                })),
                "insert-project-file" => {
                    row.on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                        this.open_project_file_inserter(cx);
                    }))
                }
                _ => row,
            };
            col = col.child(row);
        }
        col.into_any_element()
    }

    /// Render the `@`-mention popup. Shows the gitignore-respecting
    /// file list (debounced 200ms; an "(searching ...)" hint appears
    /// during the debounce window). Empty results render "No files
    /// match" per the AC.
    ///
    /// US-003 (visual-parity): row layout mirrors Zed's
    /// `code_context_menus.rs:1070-1150` — filename rendered as the
    /// primary label, parent directory truncated and rendered as a
    /// muted suffix to its right. Container widths pinned to Zed's
    /// `COMPLETION_MENU_MIN_WIDTH` / `MAX_WIDTH` (280 / 540 px) so the
    /// popup doesn't reflow on every keystroke. Match-highlight
    /// bolding (Zed bolds the fuzzy-match positions) is an acceptable
    /// divergence: Paneflow's `scan_files` returns plain `PathBuf`s
    /// with no match indices and adding a fuzzy matcher is out of
    /// scope for this story (documented in PRD substitution table).
    fn render_mention_popup(
        &self,
        state: &MentionState,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut col = div()
            .id("composer-mention-popup")
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w(px(280.))
            .max_w(px(540.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.overlay)
            .text_size(px(12.))
            .text_color(ui.text);

        if !state.scanned {
            col = col.child(
                div()
                    .px(px(6.))
                    .py(px(4.))
                    .text_color(ui.muted)
                    .child(format!("Searching for \"{}\" ...", state.query)),
            );
            return col.into_any_element();
        }

        if state.results.is_empty() {
            col = col.child(
                div()
                    .px(px(6.))
                    .py(px(4.))
                    .text_color(ui.muted)
                    .child(format!(
                        "No files match \"{}\". Press Esc to clear.",
                        state.query
                    )),
            );
            return col.into_any_element();
        }

        for (idx, path) in state.results.iter().enumerate() {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let parent_dir = path
                .parent()
                .map(|p| p.display().to_string())
                .filter(|s| !s.is_empty() && s != ".");
            let row_id: SharedString = format!("mention-row-{idx}").into();
            let mut row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|d| d.bg(ui.subtle))
                .child(
                    svg()
                        .size(px(10.))
                        .path("icons/file-text.svg")
                        .text_color(ui.muted),
                )
                .child(
                    // Primary label: filename only. `flex_none` so
                    // long directories don't push the filename out
                    // of view (matches Zed's row layout where the
                    // suffix label is the one allowed to truncate).
                    div().flex_none().text_color(ui.text).child(file_name),
                );
            if let Some(dir) = parent_dir {
                row = row.child(
                    // Muted suffix: parent directory, truncated. Zed
                    // uses the syntax-`variable` highlight color for
                    // this; Paneflow uses `ui.muted` as the closest
                    // analog (no syntax theme on mention rows).
                    div()
                        .min_w_0()
                        .text_color(ui.muted)
                        .text_size(px(11.))
                        .child(dir),
                );
            }
            row = row.on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                this.pick_mention(idx, cx);
            }));
            col = col.child(row);
        }
        col.into_any_element()
    }

    /// Render the `/`-slash command popup.
    ///
    /// US-006 (visual-parity): row layout mirrors Zed's slash menu —
    /// name (mono via the global text style) + argument hint suffix
    /// in muted, with the description on a second line in muted
    /// small. Entries are grouped by source with a section label
    /// (`Built-in commands` / `Agent commands`) — Zed's slash
    /// completion glues the source name to the command label as a
    /// muted suffix (`build_slash_item_label` in
    /// `completion_provider.rs:2356`); section headers are
    /// Paneflow's closest readable analog. Width is pinned to the
    /// same min 280 / max 540 as the mention popup so both popovers
    /// share a consistent footprint.
    fn render_slash_popup(
        &self,
        state: &SlashState,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut col = div()
            .id("composer-slash-popup")
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w(px(280.))
            .max_w(px(540.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.overlay)
            .text_size(px(12.))
            .text_color(ui.text);

        if state.results.is_empty() {
            col = col.child(
                div()
                    .px(px(6.))
                    .py(px(4.))
                    .text_color(ui.muted)
                    .child(format!(
                        "No commands match \"{}\". Press Esc to clear.",
                        state.query
                    )),
            );
            return col.into_any_element();
        }

        // US-006: partition by source so the popup reads "Built-in
        // first, then Agent-advertised" — matches Zed's grouping
        // (built-ins live in `agent_ui` defaults, agent commands
        // come from `available_commands_update`). v1.0 US-112
        // already merges them; we only reorder for display here.
        let mut built_in: Vec<(usize, &SlashCommand)> = Vec::new();
        let mut agent: Vec<(usize, &SlashCommand)> = Vec::new();
        for (idx, cmd) in state.results.iter().enumerate() {
            match cmd.source {
                SlashCommandSource::BuiltIn => built_in.push((idx, cmd)),
                SlashCommandSource::Agent => agent.push((idx, cmd)),
            }
        }

        let groups: [(&str, &[(usize, &SlashCommand)]); 2] = [
            ("Built-in commands", built_in.as_slice()),
            ("Agent commands", agent.as_slice()),
        ];

        for (header, entries) in groups {
            if entries.is_empty() {
                continue;
            }
            col = col.child(
                div()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .child(header.to_string()),
            );
            for (idx, cmd) in entries {
                let row_idx = *idx;
                let mut header_line = format!("/{}", cmd.name);
                if let Some(hint) = cmd.argument_hint.as_ref() {
                    header_line.push(' ');
                    header_line.push_str(hint);
                }
                let description = cmd.description.to_string();
                let row_id: SharedString = format!("slash-row-{row_idx}").into();
                // US-006: two-row item (name+hint on top, description
                // below muted-small). Mirrors Zed's slash row which
                // emits the description into the `end_slot` muted
                // label; Paneflow's plain `v_flex` per row keeps the
                // popup compact without a virtualized list.
                let row = div()
                    .id(row_id)
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|d| d.bg(ui.subtle))
                    .child(div().text_color(ui.text).child(header_line))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(ui.muted)
                            .child(description),
                    )
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                        this.pick_slash_command(row_idx, cx);
                    }));
                col = col.child(row);
            }
        }
        col.into_any_element()
    }

    fn render_picker_panel<F>(
        &self,
        id: SharedString,
        options: Vec<(String, String)>, // (id, label)
        current_id: Option<&str>,
        on_pick: F,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Self, String, &mut Context<Self>) + Clone + 'static,
    {
        let mut col = div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(6.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.overlay)
            .text_size(px(12.))
            .text_color(ui.text);

        if options.is_empty() {
            col = col.child(
                div()
                    .px(px(6.))
                    .py(px(4.))
                    .text_color(ui.muted)
                    .child("Loading…"),
            );
            return col.into_any_element();
        }

        for (opt_id, label) in options {
            let is_current = current_id == Some(opt_id.as_str());
            let cb = on_pick.clone();
            let row_id: SharedString = format!("picker-row-{opt_id}").into();
            let opt_id_for_callback = opt_id.clone();
            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .when(is_current, |d| d.bg(ui.subtle))
                .hover(|d| d.bg(ui.subtle))
                .child(if is_current {
                    svg()
                        .size(px(10.))
                        .path("icons/checks.svg")
                        .text_color(ui.accent)
                        .into_any_element()
                } else {
                    div().w(px(10.)).h(px(10.)).into_any_element()
                })
                .child(label)
                .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                    cb(this, opt_id_for_callback.clone(), cx);
                }));
            col = col.child(row);
        }
        col.into_any_element()
    }
}

impl Render for Composer {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Snapshot theme once for this render. The composer's many
        // sub-render helpers each used to call `ui_colors()` (which
        // re-locks the global theme cache); now they receive `ui` as
        // a parameter. Cuts ~5+ mutex acquisitions per frame.
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors_with(&theme);
        let model_label = self
            .current_model_id
            .as_ref()
            .and_then(|id| self.models.iter().find(|m| m.id == *id))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "default".to_string());

        // US-019: only one popup is visible at any time; mention >
        // slash > attach-menu > picker panels. Mention + slash own the
        // textarea's current `@` / `/` token so they take priority
        // over a user-clicked attach menu. We render the resolved
        // panel as a single `AnyElement` above the composer card.
        let mut popup_panel: Option<AnyElement> = None;
        if let Some(state) = self.mention_state.clone() {
            popup_panel = Some(self.render_mention_popup(&state, ui, cx));
        } else if let Some(state) = self.slash_state.clone() {
            popup_panel = Some(self.render_slash_popup(&state, ui, cx));
        } else if self.show_attach_menu {
            popup_panel = Some(self.render_attach_menu(ui, cx));
        }

        // Optional picker panel, rendered ABOVE the composer card
        // when open. Click a row to pick + close.
        let mut picker_panel: Option<AnyElement> = None;
        if self.show_agent_picker {
            let options: Vec<(String, String)> = self
                .discovery
                .list()
                .iter()
                .map(|d| {
                    (
                        d.kind.binary_name().to_string(),
                        d.kind.display_name().to_string(),
                    )
                })
                .collect();
            let current = self.agent_kind.binary_name().to_string();
            picker_panel = Some(self.render_picker_panel(
                "composer-agent-panel".into(),
                options,
                Some(&current),
                |this, id, cx| {
                    let kind = if id == "claude" {
                        AgentKind::ClaudeCode
                    } else {
                        AgentKind::Codex
                    };
                    this.select_agent(kind, cx);
                },
                ui,
                cx,
            ));
        } else if self.show_model_picker {
            // Codex: use the deduplicated base-model list cached on
            // disk by the CLI (`~/.codex/models_cache.json`). ACP's
            // `self.models` for Codex includes one entry per
            // (model × reasoning_effort) pair — irrelevant here
            // because effort lives on its own pill. Falling back to
            // `self.models` keeps Claude (and any agent without a
            // disk cache) working through the original ACP path.
            let options: Vec<(String, String)> = match self.agent_kind {
                AgentKind::Codex => {
                    // Codex: read the deduplicated base-model list from
                    // `~/.codex/models_cache.json` once per SessionReady
                    // and reuse it for every subsequent picker frame.
                    // Without the in-memory cache, the model popover
                    // would re-parse the JSON file 60× / second.
                    if self.cached_codex_models.is_none() {
                        let raw = read_codex_models();
                        let parsed: Vec<ModelChoice> = raw
                            .into_iter()
                            .map(|(id, name)| ModelChoice { id, name })
                            .collect();
                        self.cached_codex_models = Some(parsed);
                    }
                    let cached = self.cached_codex_models.as_ref().unwrap();
                    if cached.is_empty() {
                        self.models
                            .iter()
                            .map(|m| (m.id.clone(), m.name.clone()))
                            .collect()
                    } else {
                        cached
                            .iter()
                            .map(|m| (m.id.clone(), m.name.clone()))
                            .collect()
                    }
                }
                _ => self
                    .models
                    .iter()
                    .map(|m| (m.id.clone(), m.name.clone()))
                    .collect(),
            };
            let current = self.current_model_id.clone();
            picker_panel = Some(self.render_picker_panel(
                "composer-model-panel".into(),
                options,
                current.as_deref(),
                |this, id, cx| this.select_model(id, cx),
                ui,
                cx,
            ));
        } else if self.show_mode_picker {
            let options: Vec<(String, String)> = self
                .modes
                .iter()
                .map(|m| (m.id.to_string(), m.name.clone()))
                .collect();
            let current = self.current_mode_id.clone();
            picker_panel = Some(self.render_picker_panel(
                "composer-mode-panel".into(),
                options,
                current.as_deref(),
                |this, id, cx| this.select_mode(id, cx),
                ui,
                cx,
            ));
        } else if self.show_effort_picker {
            picker_panel = Some(self.render_effort_picker(ui, cx));
        } else if self.show_profile_picker {
            // US-115: profile popover. Surfaces every built-in +
            // user-saved profile with a bottom "Save current as
            // profile..." row.
            picker_panel = Some(self.render_profile_picker(ui, cx));
        }

        let status_row: Option<AnyElement> = self.fatal_error.as_ref().map(|err| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(rgb(0xff8080))
                .px(px(2.))
                .py(px(2.))
                .child(div().flex_1().child(err.clone()))
                .child(
                    div()
                        .id("composer-fatal-retry")
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(rgb(0xff8080))
                        .text_color(rgb(0xff8080))
                        .child("Retry")
                        .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                            // Wipe the message so the next state transition
                            // can render cleanly; ensure_runtime sets a new
                            // fatal_error if the respawn itself fails.
                            this.fatal_error = None;
                            let _ = this.ensure_runtime(cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("composer-fatal-dismiss")
                        .w(px(18.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_color(rgb(0xff8080))
                        .child("×")
                        .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                            this.fatal_error = None;
                            cx.notify();
                        })),
                )
                .into_any_element()
        });

        // US-019: attachment-specific errors (e.g. image too large)
        // render in their own row. Independent from `fatal_error` so
        // a runtime crash never hides an attach hiccup or vice versa.
        let attach_error_row: Option<AnyElement> = self.attach_error.as_ref().map(|err| {
            div()
                .text_size(px(11.))
                .text_color(rgb(0xff8080))
                .px(px(2.))
                .py(px(2.))
                .child(err.clone())
                .into_any_element()
        });

        // Non-error stop status (MaxTokens / MaxTurnRequests /
        // Cancelled). Muted tone -- this is informational, not an
        // error. Crucial UX: when Claude / Codex truncate a response
        // because of the output-token budget, the assistant text just
        // cuts off mid-word with no native indication. Showing
        // "Response truncated: max output tokens reached" lets the
        // user understand the missing characters aren't a Paneflow
        // drop. Cleared on the next prompt submission.
        let stop_status_row: Option<AnyElement> = self.stop_status.map(|msg| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(ui.muted)
                .px(px(2.))
                .py(px(2.))
                .child(div().flex_1().child(msg))
                .child(
                    div()
                        .id("composer-stop-status-dismiss")
                        .w(px(18.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_color(ui.muted)
                        .child("×")
                        .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                            this.stop_status = None;
                            cx.notify();
                        })),
                )
                .into_any_element()
        });

        let attachment_chips: Option<AnyElement> = self.render_attachment_chips(ui, cx);

        // Toolbar layout: two-cluster row with `justify_between` —
        // attach/effort on the leading edge, AI cluster + send on the
        // trailing edge. Mirrors Zed's `thread_view.rs:3638-3665`
        // bottom-controls row: outer `h_flex().w_full().flex_none()
        // .flex_wrap().justify_between()`, left cluster `gap_0p5()`
        // (= 2px), right cluster `gap_1()` (= 4px). Keeping the model
        // selector glued to the send button mirrors the pattern in
        // most polished agent UIs (the metadata you're about to send
        // sits visually adjacent to the send action).
        //
        // US-001 (visual-parity PRD): port of the Zed builder chain
        // 1:1. Substitutions: Zed `ui::IconButton` / `ui::Button` /
        // `PopoverMenu` -> Paneflow `render_*_pill` helpers (already
        // shipped, behavior unchanged). Pill ordering kept as Paneflow
        // shipped it (agent / model / profile / send) because Paneflow
        // has no separate `mode` pill — `current_mode_id` is tracked
        // but never exposed via a trigger today. Documented in the PRD
        // substitution table.
        let bottom_toolbar = div()
            .flex()
            .flex_row()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .child(
                // LEFT cluster — gap_0p5 (Zed convention = 2px).
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .child(self.render_plus_button(ui, cx))
                    // Lightbulb thinking-effort pill glued to the `+`.
                    // Independent from ACP session modes (permissions):
                    // this is the per-message reasoning effort
                    // selector. Closest analog to Zed's `thinking_control`.
                    .child(self.render_effort_pill(ui, cx)),
            )
            .child(
                // RIGHT cluster — gap_1 (Zed convention = 4px).
                // Order: agent -> model -> profile -> send. Agent comes
                // before model so the reading order matches the
                // dependency (agent picks the backend; model picks the
                // variant). Profile sits glued to the send button so
                // the user's last-pick context is visually adjacent to
                // the action (US-115).
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .child(self.render_agent_pill(ui, cx))
                    .child(self.render_model_pill(model_label, ui, cx))
                    .child(self.render_profile_pill(ui, cx))
                    .child(self.render_send_button(ui, cx)),
            );

        // US-002 (visual-parity): textarea wrapper mirrors Zed's
        // `thread_view.rs:3573-3625` shape — `relative()` anchor with
        // an absolute top-right expand button (opacity 0.5 → 1.0 on
        // hover). When `editor_expanded` is set, the wrapper grows to
        // 80% of viewport height (Zed's `vh(0.8, window)` inlined as
        // `viewport_size().height * 0.8`).
        //
        // The whole region acts as a single editable surface:
        // hovering anywhere shows the I-beam cursor, and clicking on
        // the empty space below the actual text routes focus into the
        // TextArea (so the user doesn't have to land their click on
        // the narrow text row to start typing).
        let expanded_height_px = f32::from(w.viewport_size().height) * 0.8;
        let expand_icon_path = if self.editor_expanded {
            "icons/chevron_down.svg"
        } else {
            "icons/arrows-diagonal.svg"
        };
        let textarea_row = div()
            .id("composer-textarea-area")
            .relative()
            .flex()
            .flex_row()
            .items_start()
            .w_full()
            .when(self.editor_expanded, |d| d.h(px(expanded_height_px)))
            .when(!self.editor_expanded, |d| d.min_h(px(28.)))
            .cursor_text()
            .on_click(cx.listener(|this, _ev: &ClickEvent, w, cx| {
                let handle = this.text_area.read(cx).focus_handle_ref().clone();
                handle.focus(w, cx);
            }))
            .child(div().flex_1().pr(px(10.)).child(self.text_area.clone()))
            .child(
                // Absolute top-right expand button. Opacity 0.5 →
                // 1.0 on hover mirrors Zed's
                // `thread_view.rs:3611-3612`. Container-level hover
                // so the affordance reveals smoothly when the user
                // approaches the corner.
                div()
                    .id("composer-expand-toggle")
                    .absolute()
                    .top_0()
                    .right_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .w(px(20.))
                    .h(px(20.))
                    .opacity(0.5)
                    .hover(|s| s.opacity(1.0))
                    .cursor_pointer()
                    .child(
                        svg()
                            .size(px(12.))
                            .path(expand_icon_path)
                            .text_color(ui.muted),
                    )
                    .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                        this.editor_expanded = !this.editor_expanded;
                        cx.notify();
                    })),
            );

        // US-002 (visual-parity port, 2026-05-24 follow-up): match
        // Zed's `render_message_editor` shell at
        // `~/dev/zed/crates/agent_ui/src/conversation_view/thread_view.rs:3573-3669`.
        // Zed's chain:
        //   h_flex().p_2().bg(editor_bg_color).justify_center()
        //     .border_t_1().border_color(border)         // when messages above
        //     .child(v_flex().w_full().justify_between().gap_2()
        //         .child(<textarea region w/ pt_1 pr_2p5>)
        //         .child(<bottom controls row>))
        //
        // The previous Paneflow render wrapped this in a rounded
        // `ui.surface` card with `px(16) py(12) rounded(10)` plus an
        // additional outer `px(16) py(10)` wrapper -- which produced
        // the visible bordered "card-in-a-card" chrome that doesn't
        // exist in Zed. Drop the rounding + double padding and use the
        // Zed-style flat band with only a top-border separator.
        let drop_accent_bg = ui.accent;
        let composer_body = div()
            .id("composer-card")
            .flex()
            .flex_col()
            .w_full()
            .gap(px(8.)) // Zed's `gap_2()` between textarea and toolbar.
            .drag_over::<ExternalPaths>(move |style, _paths, _w, _cx| {
                // Soft accent overlay -- matches Zed §24's
                // `drop_target_background` token. Subtle enough that
                // the textarea content remains readable, distinct
                // enough that the affordance is obvious.
                style.bg(drop_accent_bg.opacity(0.18))
            })
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _w, cx| {
                let owned: Vec<PathBuf> = paths.paths().to_vec();
                this.complete_drop_paths(owned, cx);
            }))
            .when_some(attachment_chips, |d, el| d.child(el))
            .child(textarea_row)
            .child(bottom_toolbar)
            .when_some(attach_error_row, |d, el| d.child(el))
            .when_some(status_row, |d, el| d.child(el))
            .when_some(stop_status_row, |d, el| d.child(el));

        // Floating composer card: lifted off all edges with
        // horizontal + bottom margin so it reads as a discrete
        // surface rather than a band glued to the panel chrome.
        // `border_1` now wraps all four sides, `rounded(px(14.))`
        // gives every corner a generous radius. The previous
        // edge-to-edge band (`p_2` + `border_t_1` + `rounded_t`) is
        // replaced by a true floating card.
        div()
            .flex()
            .flex_col()
            .when_some(picker_panel, |d, panel| d.child(panel))
            .when_some(popup_panel, |d, panel| d.child(panel))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .w_full()
                    .px(px(12.))
                    .pb(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_center()
                            .w_full()
                            .p(px(8.))
                            .bg(ui.surface)
                            .border_1()
                            .border_color(ui.border)
                            .rounded(px(14.))
                            .child(composer_body),
                    ),
            )
    }
}

/// US-102: split a persisted draft's `Vec<ContentBlock>` back into
/// (joined-text, attachments) so the composer can pre-populate the
/// TextArea + chip row. Inverse of [`combine_prompt`]: text blocks
/// fold into one string with `\n` separators (matches the order the
/// user typed), image/resource-link blocks become `PendingAttachment`
/// chips with the same `kind` + label they would have had if the user
/// just attached them. Unknown blocks are skipped silently — the
/// agent-client-protocol schema is `#[non_exhaustive]` and a future
/// variant should not block draft restore.
fn split_draft_blocks(blocks: &[ThreadsContentBlock]) -> (String, Vec<PendingAttachment>) {
    use agent_client_protocol::schema::ContentBlock;
    let mut text = String::new();
    let mut attachments = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
            ContentBlock::Image(img) => {
                attachments.push(PendingAttachment {
                    label: "image".to_string(),
                    kind: AttachmentKind::Image,
                    block: ContentBlock::Image(img.clone()),
                });
            }
            ContentBlock::ResourceLink(link) => {
                let label = std::path::Path::new(&link.uri)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| link.uri.clone());
                attachments.push(PendingAttachment {
                    label,
                    kind: AttachmentKind::File,
                    block: ContentBlock::ResourceLink(link.clone()),
                });
            }
            _ => {
                // Audio / Resource (embedded) / unknown variants are
                // not surfaced as chips today; their persistence
                // happens transparently via combine_prompt -> draft
                // and a future renderer can pick them up without a
                // schema bump.
            }
        }
    }
    (text, attachments)
}

/// US-106: tooltip body for the queue-state send button. Two-line
/// hint: primary action + secondary action with the immediate-send
/// keybind. Mirrors [`crate::window_chrome::title_bar::AgentsTooltip`]
/// since Paneflow has no shared tooltip primitive yet.
struct QueueTooltip;

impl QueueTooltip {
    fn new() -> Self {
        Self
    }

    /// Per-OS shortcut hint string. macOS uses Cmd, others Ctrl. The
    /// keybind matches the TextArea binding registered in
    /// `register_keybindings`.
    fn immediate_shortcut() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "Cmd+Shift+Enter"
        }
        #[cfg(not(target_os = "macos"))]
        {
            "Ctrl+Shift+Enter"
        }
    }
}

impl Render for QueueTooltip {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors();
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(theme.title_bar_background)
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .text_size(px(11.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(div().child("Queue and Send"))
                    .child(div().text_color(ui.muted).text_size(px(10.)).child("Enter")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_color(ui.muted).child("Send Immediately"))
                    .child(
                        div()
                            .text_color(ui.muted)
                            .text_size(px(10.))
                            .child(Self::immediate_shortcut()),
                    ),
            )
    }
}

/// US-117: payload captured at chip-render time and handed to the
/// hover tooltip. Images carry the raw bytes + format so the tooltip
/// Entity can wrap them in a `gpui::Image`; files carry the absolute
/// path + on-disk size in bytes (computed via `std::fs::metadata`).
/// `Cloneable` so the same payload can be reused if the tooltip is
/// torn down and rebuilt on a subsequent hover.
#[derive(Debug, Clone)]
enum AttachmentTooltipPayload {
    /// `bytes` is the decoded image body (NOT the base64 string),
    /// ready for `gpui::Image::from_bytes`. `label` is the chip's
    /// human-readable filename used as the tooltip caption.
    Image {
        label: String,
        bytes: Vec<u8>,
        format: ImageFormat,
    },
    /// Files render the absolute path + size in bytes. Both fields
    /// are `Option` because the underlying `ContentBlock::ResourceLink`
    /// may carry a non-filesystem URI (e.g. an MCP `resource:` link)
    /// in which case the stat fails -- the tooltip then shows the URI
    /// alone (graceful degradation per AC #4).
    File {
        label: String,
        absolute_path: Option<String>,
        size_bytes: Option<u64>,
    },
}

/// US-117: build the tooltip payload from a pending attachment.
/// Cheap operations only -- the base64 decode happens here so the
/// hover does not pay it, but a 10 MB image stays well under 50 ms
/// even on a low-end laptop.
fn attachment_tooltip_payload(att: &PendingAttachment) -> AttachmentTooltipPayload {
    match att.kind {
        AttachmentKind::Image => {
            if let Some((bytes, mime)) = super::composer_ext::decode_image_block(&att.block) {
                let format = image_format_from_mime(mime).unwrap_or(ImageFormat::Png);
                AttachmentTooltipPayload::Image {
                    label: att.label.clone(),
                    bytes,
                    format,
                }
            } else {
                // Malformed image block (corrupted base64 or an
                // unexpected variant): fall back to file-style tooltip
                // rather than failing the chip render outright.
                AttachmentTooltipPayload::File {
                    label: att.label.clone(),
                    absolute_path: None,
                    size_bytes: None,
                }
            }
        }
        AttachmentKind::File => {
            let (absolute_path, size_bytes) = resource_link_path_and_size(&att.block);
            AttachmentTooltipPayload::File {
                label: att.label.clone(),
                absolute_path,
                size_bytes,
            }
        }
    }
}

/// Best-effort mime -> `gpui::ImageFormat` mapping. `None` for unknown
/// mimes -- the caller falls back to PNG (the most lenient decoder).
fn image_format_from_mime(mime: &str) -> Option<ImageFormat> {
    match mime {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::Webp),
        "image/gif" => Some(ImageFormat::Gif),
        "image/svg+xml" => Some(ImageFormat::Svg),
        "image/bmp" => Some(ImageFormat::Bmp),
        _ => None,
    }
}

/// Resolve the absolute path + on-disk size for a `ResourceLink`
/// attachment. Returns `(None, None)` for non-`file://` URIs or when
/// the path does not stat (e.g. user deleted the file between attach
/// and hover).
fn resource_link_path_and_size(
    block: &agent_client_protocol::schema::ContentBlock,
) -> (Option<String>, Option<u64>) {
    use agent_client_protocol::schema::ContentBlock;
    let ContentBlock::ResourceLink(link) = block else {
        return (None, None);
    };
    let raw = link.uri.as_str();
    let path_str = raw.strip_prefix("file://").unwrap_or(raw);
    let path = std::path::Path::new(path_str);
    let size = std::fs::metadata(path).ok().map(|m| m.len());
    (Some(path_str.to_string()), size)
}

/// US-117: format a byte count as a short human-readable string.
/// Uses binary units (KiB/MiB/...) for parity with `ls -lh`. Returns
/// "?" when the size is unknown (unhappy path: stat failed).
fn format_byte_size(size: Option<u64>) -> String {
    let Some(bytes) = size else {
        return "?".to_string();
    };
    const UNITS: [(&str, u64); 4] = [
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
        ("B", 1),
    ];
    for (label, scale) in UNITS {
        if bytes >= scale {
            // Two decimals for sub-GiB amounts; whole bytes for the
            // smallest tier so "412 B" is not rendered as "412.00 B".
            if scale == 1 {
                return format!("{bytes} {label}");
            }
            let value = bytes as f64 / scale as f64;
            return format!("{value:.2} {label}");
        }
    }
    "0 B".to_string()
}

/// US-117: hover tooltip for an attachment chip. Renders the
/// 320x320 image preview (with object-fit contain so portrait /
/// landscape source images stay un-cropped) or a path-and-size row
/// for file chips.
struct AttachmentTooltip {
    payload: AttachmentTooltipPayload,
}

impl AttachmentTooltip {
    fn new(payload: AttachmentTooltipPayload) -> Self {
        Self { payload }
    }
}

impl Render for AttachmentTooltip {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors();
        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(theme.title_bar_background)
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .text_size(px(11.));
        match &self.payload {
            AttachmentTooltipPayload::Image {
                label,
                bytes,
                format,
            } => {
                let image = Arc::new(Image::from_bytes(*format, bytes.clone()));
                // AC #1: 320x320 box, aspect ratio preserved via
                // `ObjectFit::Contain`. The bg under the image picks up
                // the tooltip's own background, so portrait images
                // letterbox cleanly against the tooltip surface.
                root = root.child(
                    div()
                        .w(px(320.))
                        .h(px(320.))
                        .child(img(image).object_fit(ObjectFit::Contain).size_full()),
                );
                root = root.child(div().text_color(ui.muted).child(label.clone()));
            }
            AttachmentTooltipPayload::File {
                label,
                absolute_path,
                size_bytes,
            } => {
                // AC #4: no preview for file chips; show label + path
                // + size as a 2-line block.
                root = root.child(div().child(label.clone()));
                if let Some(path) = absolute_path.as_ref() {
                    root = root.child(
                        div()
                            .text_color(ui.muted)
                            .text_size(px(10.))
                            .child(path.clone()),
                    );
                }
                root = root.child(
                    div()
                        .text_color(ui.muted)
                        .text_size(px(10.))
                        .child(format_byte_size(*size_bytes)),
                );
            }
        }
        root
    }
}

/// US-115: key handler for the profile save-as inline text input.
/// Enter commits, Escape cancels, Backspace pops, printable char
/// pushes. Mirrors the sidebar's `handle_rename_key` pattern so the
/// UX stays consistent across inline-rename surfaces.
fn handle_profile_name_key(
    this: &mut Composer,
    e: &gpui::KeyDownEvent,
    cx: &mut Context<Composer>,
) {
    let key = e.keystroke.key.as_str();
    match key {
        "enter" => this.commit_profile_save(cx),
        "escape" => this.cancel_profile_save(cx),
        "backspace" => this.pop_profile_save_char(cx),
        _ => {
            if let Some(ch) = &e.keystroke.key_char
                && !ch.is_empty()
                && !e.keystroke.modifiers.control
                && !e.keystroke.modifiers.platform
            {
                this.push_profile_save_char(ch, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{ContentBlock, TextContent};
    use std::collections::VecDeque;

    /// US-106 AC #1-#4 + #9: the 4 state-transition matrix is pure and
    /// belongs to `send_button_state`. Render-layer tests would need a
    /// GPUI App context that this crate does not maintain (the project
    /// CLAUDE.md notes "No tests in the app crate"). Covering the pure
    /// branching here keeps the AC honest without that infrastructure.
    #[test]
    fn send_button_states_cover_all_four_transitions() {
        assert_eq!(
            send_button_state(false, false),
            SendButtonState::IdleEmpty,
            "state 1: idle + empty -> disabled ghost",
        );
        assert_eq!(
            send_button_state(false, true),
            SendButtonState::IdleContent,
            "state 2: idle + content -> filled send",
        );
        assert_eq!(
            send_button_state(true, false),
            SendButtonState::StreamingEmpty,
            "state 3: streaming + empty -> red stop",
        );
        assert_eq!(
            send_button_state(true, true),
            SendButtonState::StreamingContent,
            "state 4: streaming + content -> queue",
        );
    }

    /// US-106 AC #5 + #6: the queue is FIFO (push_back / pop_front) and
    /// preserves order. This is the contract the dequeue path relies
    /// on -- TurnEnded calls pop_front, which must produce prompts in
    /// the same order the user enqueued them.
    #[test]
    fn pending_prompt_queue_is_fifo() {
        let mut queue: VecDeque<PendingPrompt> = VecDeque::new();
        let make = |s: &str| PendingPrompt {
            blocks: vec![ContentBlock::Text(TextContent::new(s))],
        };
        queue.push_back(make("first"));
        queue.push_back(make("second"));
        queue.push_back(make("third"));
        assert_eq!(queue.len(), 3);

        // pop_front order matches push_back order
        let head_blocks = queue.pop_front().expect("non-empty").blocks;
        let head_text = match &head_blocks[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert_eq!(head_text, "first");
        assert_eq!(queue.len(), 2);

        let next = match &queue.pop_front().expect("non-empty").blocks[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!(),
        };
        assert_eq!(next, "second");
    }

    /// US-106 AC #7: explicit clear drops every queued prompt. Cancel
    /// alone (Stop button) preserves the queue, so the only path to a
    /// drop is `clear_pending_prompts` (called by the activity bar's
    /// "x" affordance in US-107).
    #[test]
    fn pending_prompt_queue_clear_empties_it() {
        let mut queue: VecDeque<PendingPrompt> = VecDeque::new();
        for s in ["a", "b", "c"] {
            queue.push_back(PendingPrompt {
                blocks: vec![ContentBlock::Text(TextContent::new(s))],
            });
        }
        assert_eq!(queue.len(), 3);
        queue.clear();
        assert!(queue.is_empty());
    }

    /// US-115 AC #1: the three built-in profiles are always seeded
    /// in the picker even when `paneflow.json` is empty. Order is
    /// stable: Write, Ask, Minimal -- mirrors the order documented
    /// in `docs/ZED_AGENT_REFERENCE.md` §16.
    #[test]
    fn built_in_profiles_seed_three_in_order() {
        let profiles = built_in_profiles();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Write", "Ask", "Minimal"]);
        assert!(profiles.iter().all(|p| p.is_builtin));
    }

    /// US-115 AC: snapshot round-trips through ProfileConfig without
    /// dropping any field. The wire format uses owned strings so the
    /// HashMap can carry across processes; the runtime form uses
    /// typed enums so the apply path is strict.
    #[test]
    fn profile_snapshot_round_trips_through_config() {
        let snapshot = ProfileSnapshot {
            agent: Some(AgentKind::ClaudeCode),
            model: Some("claude-sonnet-4-5".to_string()),
            mode: Some("default".to_string()),
            effort: Some(ThinkingEffort::High),
            tools: vec!["read".to_string(), "edit".to_string()],
        };
        let config = snapshot.to_config();
        assert_eq!(config.agent.as_deref(), Some("claude_code"));
        assert_eq!(config.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(config.effort.as_deref(), Some("high"));
        let back = ProfileSnapshot::from_config(&config);
        assert_eq!(back, snapshot);
    }

    /// US-115: an unknown agent string in a persisted profile drops
    /// the field rather than crashing the apply path. The composer's
    /// missing-agent toast triggers off `snapshot.agent == None`, so
    /// the lowered form is what guards the apply.
    #[test]
    fn profile_snapshot_drops_unknown_agent() {
        let config = paneflow_config::schema::ProfileConfig {
            agent: Some("opencode".to_string()), // not in AgentKind yet
            ..Default::default()
        };
        let snapshot = ProfileSnapshot::from_config(&config);
        assert!(snapshot.agent.is_none());
    }

    /// US-115 AC #1: `agent_panel.profiles` deserialises cleanly from
    /// the documented JSON shape; the schema honors `#[serde(default)]`
    /// so a missing or empty map is fine.
    #[test]
    fn agent_panel_profiles_deserialise_from_json() {
        let raw = r#"{
            "profiles": {
                "Code Review": {
                    "agent": "claude_code",
                    "effort": "high",
                    "tools": ["read", "search"]
                }
            },
            "default_profile": "Code Review"
        }"#;
        let cfg: paneflow_config::schema::AgentPanelConfig = serde_json::from_str(raw).unwrap();
        let cr = cfg.profiles.get("Code Review").expect("profile present");
        assert_eq!(cr.agent.as_deref(), Some("claude_code"));
        assert_eq!(cr.effort.as_deref(), Some("high"));
        assert_eq!(cr.tools, vec!["read".to_string(), "search".to_string()]);
        assert_eq!(cfg.default_profile.as_deref(), Some("Code Review"));
    }
}
