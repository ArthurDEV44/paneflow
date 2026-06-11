// US-017: Config schema types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current on-disk schema version for [`SessionState`].
pub const SESSION_SCHEMA_VERSION: u32 = 1;

/// Top-level PaneFlow configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneFlowConfig {
    /// Key-action shortcut mappings (e.g. "ctrl+t" -> "new_tab").
    pub shortcuts: HashMap<String, String>,
    /// Default shell binary path. `None` uses the system default.
    pub default_shell: Option<String>,
    /// Terminal color theme name (e.g. "One Dark", "PaneFlow Light").
    pub theme: Option<String>,
    /// Workspace command definitions (cmux-compatible format).
    pub commands: Vec<CommandDefinition>,
    /// Window decoration mode: `"client"` (CSD, default) or `"server"` (SSD).
    pub window_decorations: Option<String>,
    /// Terminal line height multiplier (default: 1.3, valid range: 1.0–2.5).
    pub line_height: Option<f32>,
    /// Terminal font family (default: platform-specific monospace fallback).
    pub font_family: Option<String>,
    /// Terminal font size in pixels (default: 14.0, valid range: 8.0–32.0).
    pub font_size: Option<f32>,
    /// Treat Alt key as Meta (send ESC prefix). Default: true on Linux.
    /// Set to false for future macOS where Option produces Unicode characters.
    pub option_as_meta: Option<bool>,
    /// EP-003 US-007 (cli-cockpit): master switch for the per-shell rc
    /// injection (OSC 7 CWD reporting + OSC 133 command marks). `None`/`true`
    /// = enabled (the long-standing default behavior); `false` = no snippet
    /// is written or wired — the shell starts exactly as it would outside
    /// Paneflow.
    pub shell_integration: Option<bool>,
    /// EP-004 US-011 (cli-cockpit): master switch for Stalled detection.
    /// `None`/`true` = enabled (default ON): a `Thinking` agent session with
    /// no hook activity past the silence threshold is flagged `Stalled` and
    /// notified ONCE per stall episode (the flag clears on the next hook
    /// event, so a legitimately long turn costs at most one notification).
    /// `false` = kill switch — no `Stalled` state is ever produced.
    pub agent_stall_detection: Option<bool>,
    /// EP-004 US-011: silence threshold in seconds before a `Thinking`
    /// session is flagged `Stalled`. `None` resolves to 300 s; values are
    /// clamped to `[30, 86400]`. Checked by the 30 s sweep, so the
    /// effective granularity is threshold ± 30 s.
    pub agent_stall_threshold_secs: Option<u64>,
    /// External editor used to open markdown links (file paths shipped
    /// by the agent as `[foo](src/foo.rs)` or `[foo](src/foo.rs:42)`).
    ///
    /// Accepted values:
    /// - `"auto"` (default when absent): detect the first CLI present
    ///   on PATH from the preferred order `zed`, `cursor`, `windsurf`,
    ///   `code`. Falls back to the system opener (xdg-open / open /
    ///   start) when none are installed.
    /// - `"system"`: always defer to the OS-level opener.
    /// - `"zed"` | `"cursor"` | `"windsurf"` | `"code"`: force the
    ///   named CLI even if other editors are also installed.
    ///
    /// The chosen CLI is spawned with `<editor> <abs_path>[:line[:col]]`;
    /// all four support that suffix natively to jump to the target
    /// position.
    pub external_editor: Option<String>,
    /// When `Some(true)`, every permission gate is disabled:
    /// 1. The "Claude Code" tab-bar terminal launcher adds
    ///    `--permission-mode bypassPermissions` to the spawned CLI.
    /// 2. The Agents view auto-approves every ACP `RequestPermission`
    ///    for both Claude Code and Codex sessions (any tool kind:
    ///    Read / Edit / Delete / Move / Execute / Search / Fetch).
    ///
    /// `Some(false)` or `None` (the default) keeps the per-tool confirmation
    /// prompts enabled.
    /// Per Anthropic's docs bypass mode offers no protection against
    /// prompt injection — opt out (toggle off in Settings -> AI Agent)
    /// if you want explicit confirmation for every tool call. The key
    /// retains its `claude_code_` prefix for backwards compatibility
    /// with existing user configs even though the scope now covers
    /// Codex too.
    pub claude_code_bypass_permissions: Option<bool>,
    /// Show the built-in "Claude Code" command button in the tab bar.
    /// `Some(true)` always renders the button, `Some(false)` hides it, and
    /// `None` (default) renders it only when the CLI binary is installed.
    pub claude_code_button_visible: Option<bool>,
    /// Show the built-in "Codex" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub codex_button_visible: Option<bool>,
    /// Show the built-in "Opencode" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub opencode_button_visible: Option<bool>,
    /// Show the built-in "Pi" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub pi_button_visible: Option<bool>,
    /// Show the built-in "Hermes Agent" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub hermes_agent_button_visible: Option<bool>,
    /// Show the built-in "Grok" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub grok_button_visible: Option<bool>,
    /// Show the built-in "Amp" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub amp_button_visible: Option<bool>,
    /// Show the built-in "Cursor" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub cursor_button_visible: Option<bool>,
    /// Show the built-in "Gemini" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub gemini_button_visible: Option<bool>,
    /// Show the built-in "Kiro" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub kiro_button_visible: Option<bool>,
    /// Show the built-in "Antigravity" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub antigravity_button_visible: Option<bool>,
    /// Show the built-in "Copilot" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub copilot_button_visible: Option<bool>,
    /// Show the built-in "CodeBuddy" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub codebuddy_button_visible: Option<bool>,
    /// Show the built-in "Factory" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub factory_button_visible: Option<bool>,
    /// Show the built-in "Qoder" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub qoder_button_visible: Option<bool>,
    /// Show the built-in "Openclaw" command button in the tab bar.
    /// Same semantics as `claude_code_button_visible`.
    pub openclaw_button_visible: Option<bool>,
    /// Opt-in desktop telemetry block.
    ///
    /// Tri-state semantics:
    /// - `None` (block missing from config): user has never been prompted.
    /// - `Some(TelemetryConfig { enabled: None })`: block exists but the
    ///   consent question is still unanswered (e.g. user dismissed the
    ///   first-run modal without choosing).
    /// - `Some(TelemetryConfig { enabled: Some(true|false) })`: explicit
    ///   user answer — consent granted or refused.
    ///
    /// The consent modal (US-011) only appears when `telemetry.enabled`
    /// resolves to `None` under both the outer and inner Option layers.
    /// No event is ever sent unless `enabled == Some(true)`.
    pub telemetry: Option<TelemetryConfig>,
    /// Terminal-scoped settings block. Currently exposes `ligatures`
    /// (US-008); future renderer toggles will land here without schema
    /// churn at the top level.
    pub terminal: Option<TerminalConfig>,
    /// Agents-view-scoped settings block (US-103 + future Phase B-E
    /// stories of `tasks/prd-agent-ui-refactor-2026-Q3.md`). Lives in
    /// its own struct so the dozen-or-so fields the refactor introduces
    /// stay namespaced under `"agent_panel": { ... }`.
    pub agent_panel: Option<AgentPanelConfig>,
    /// Per-tool permission patterns (US-111 of
    /// `tasks/prd-agent-ui-refactor-2026-Q3.md`). The key is the
    /// `ToolKind` discriminant (e.g. `"read"`, `"edit"`, `"execute"`)
    /// -- matching Zed §13's `ToolPermissions` shape. An entry's
    /// `always_allow` patterns auto-resolve future
    /// `WaitingForConfirmation` callbacks; `always_deny` patterns
    /// auto-reject them. A bare entry with no patterns matches every
    /// call of that tool kind, which is what the "Allow Always for
    /// this tool" UI writes today.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tool_permissions: HashMap<String, ToolPermissionsEntry>,
}

impl PaneFlowConfig {
    /// EP-004 US-011: default Stalled silence threshold. 300 s tolerates a
    /// long tool-free reasoning stretch while still surfacing a wedged
    /// agent within minutes (CI no-output watchdogs sit at 10 min; an
    /// interactive cockpit warrants half that).
    pub const DEFAULT_AGENT_STALL_THRESHOLD_SECS: u64 = 300;
    /// Lower bound: below the 30 s sweep cadence the threshold cannot be
    /// honored and every long tool call would false-positive.
    pub const MIN_AGENT_STALL_THRESHOLD_SECS: u64 = 30;
    /// Upper bound: a day — past this the feature is effectively off, so
    /// use [`PaneFlowConfig::agent_stall_detection`] instead.
    pub const MAX_AGENT_STALL_THRESHOLD_SECS: u64 = 86_400;

    /// Resolve the Stalled-detection master switch (default ON).
    pub fn agent_stall_detection_enabled(&self) -> bool {
        self.agent_stall_detection.unwrap_or(true)
    }

    /// Resolve `agent_stall_threshold_secs`: default 300, clamped to
    /// `[30, 86400]` with a `warn!` so an out-of-range value is noticed.
    pub fn resolved_agent_stall_threshold_secs(&self) -> u64 {
        let raw = self
            .agent_stall_threshold_secs
            .unwrap_or(Self::DEFAULT_AGENT_STALL_THRESHOLD_SECS);
        let clamped = raw.clamp(
            Self::MIN_AGENT_STALL_THRESHOLD_SECS,
            Self::MAX_AGENT_STALL_THRESHOLD_SECS,
        );
        if clamped != raw {
            tracing::warn!(
                target: "paneflow_config::agent",
                requested = raw,
                clamped,
                "agent_stall_threshold_secs out of range [{min}, {max}], clamped",
                min = Self::MIN_AGENT_STALL_THRESHOLD_SECS,
                max = Self::MAX_AGENT_STALL_THRESHOLD_SECS,
            );
        }
        clamped
    }
}

/// Per-tool permission patterns persisted under `"tool_permissions"`
/// in `paneflow.json` (US-111). Patterns are matched as substrings
/// against the tool call's raw input pretty-printed JSON; an empty
/// `always_allow` list with an existing entry counts as "always
/// allow every call of this tool" (the v1 UI does not yet expose
/// pattern-scoped persistence and uses this shape).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolPermissionsEntry {
    /// Substring patterns whose presence in the tool input auto-
    /// resolves `Allow`. An empty vec means "always allow".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_allow: Vec<String>,
    /// Substring patterns whose presence auto-resolves `Reject`.
    /// Auto-promotion from `always_allow` to `always_deny` happens
    /// at the UI layer when the user explicitly rejects a call that
    /// previously matched -- treated as a correction signal per Zed
    /// §13 / PRD US-111 AC #8.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_deny: Vec<String>,
}

/// US-005: how the terminal surfaces a BEL (`\a`) byte. Default `Visual`
/// preserves the historical flash-only behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalBellMode {
    /// 200 ms background flash only (historical default).
    #[default]
    Visual,
    /// Ring the OS system bell only (no flash).
    Audible,
    /// Both the visual flash and the OS system bell.
    Both,
    /// No bell feedback at all.
    Off,
}

impl TerminalBellMode {
    /// Whether this mode rings the OS system bell.
    pub fn is_audible(self) -> bool {
        matches!(self, Self::Audible | Self::Both)
    }

    /// Whether this mode shows the 200 ms visual flash.
    pub fn is_visual(self) -> bool {
        matches!(self, Self::Visual | Self::Both)
    }
}

/// US-007: configurable default cursor shape, applied as the fallback before
/// any app-driven DECSCUSR escape. Mapped to the renderer's cursor shapes in
/// the app layer (this crate stays free of the terminal backend).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShapeConfig {
    /// Solid block `█` (historical default).
    #[default]
    Block,
    /// Vertical bar `⎸`.
    Beam,
    /// Underline `_`.
    Underline,
    /// Hollow box `▯`.
    Hollow,
}

/// US-008: cursor blink override. `TerminalControlled` (default) defers to the
/// program's DECSCUSR cursor-style setting; `On`/`Off` force the behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorBlinkConfig {
    /// Force the cursor to blink regardless of what the program requests.
    On,
    /// Force the cursor solid regardless of what the program requests.
    Off,
    /// Defer to the program's DECSCUSR setting (historical default).
    #[default]
    TerminalControlled,
}

// Manual `Deserialize` for the terminal enums. A derived `Deserialize` hard-
// errors on an unrecognised variant; that error propagates up to
// `parse_and_validate` (loader.rs), which discards the ENTIRE user config and
// returns defaults. A single typo (`"bell": "loud"`) would silently wipe the
// theme, shell, shortcuts, and agent settings. Instead fall back to the variant
// default with a logged warning, mirroring `ThinkingDisplayMode`. `Serialize`
// stays derived (snake_case), so round-tripping a valid value is unchanged.
impl<'de> Deserialize<'de> for TerminalBellMode {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        Ok(match raw.as_str() {
            "visual" => Self::Visual,
            "audible" => Self::Audible,
            "both" => Self::Both,
            "off" => Self::Off,
            other => {
                tracing::warn!(
                    target: "paneflow_config::terminal",
                    value = other,
                    "terminal.bell value not recognized, defaulting to visual",
                );
                Self::Visual
            }
        })
    }
}

impl<'de> Deserialize<'de> for CursorShapeConfig {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        Ok(match raw.as_str() {
            "block" => Self::Block,
            "beam" => Self::Beam,
            "underline" => Self::Underline,
            "hollow" => Self::Hollow,
            other => {
                tracing::warn!(
                    target: "paneflow_config::terminal",
                    value = other,
                    "terminal.cursor_shape value not recognized, defaulting to block",
                );
                Self::Block
            }
        })
    }
}

impl<'de> Deserialize<'de> for CursorBlinkConfig {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        Ok(match raw.as_str() {
            "on" => Self::On,
            "off" => Self::Off,
            "terminal_controlled" => Self::TerminalControlled,
            other => {
                tracing::warn!(
                    target: "paneflow_config::terminal",
                    value = other,
                    "terminal.cursor_blink value not recognized, defaulting to terminal_controlled",
                );
                Self::TerminalControlled
            }
        })
    }
}

/// Terminal-scoped configuration block (US-008).
///
/// Lives in its own struct so future renderer settings (cursor shape,
/// blink interval, alternate scroll, …) can be added without expanding
/// the top-level `PaneFlowConfig` further.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TerminalConfig {
    /// Render programming-font ligatures (FiraCode `=>`, `!=`, …) when
    /// `Some(true)`. `None` and `Some(false)` both keep the historical
    /// behavior of disabling ligatures via GPUI's `FontFeatures`.
    pub ligatures: Option<bool>,
    /// Maximum scrollback history in lines (Zed parity:
    /// `max_scroll_history_lines`). `None` resolves to
    /// [`TerminalConfig::DEFAULT_SCROLLBACK_LINES`]; values are clamped
    /// to `[100, 100_000]` to keep a runaway log from eating RAM.
    /// Read once at PTY spawn time; changing this value takes effect on
    /// the next new terminal.
    pub scrollback_lines: Option<usize>,
    /// US-005: how a BEL (`\a`) is surfaced — `visual` flash, `audible` system
    /// bell, `both`, or `off`. `None` resolves to `Visual` (historical
    /// default). Read once at terminal construction; takes effect on the next
    /// new terminal.
    pub bell: Option<TerminalBellMode>,
    /// US-007: default cursor shape before any app-driven DECSCUSR escape.
    /// `None` resolves to `Block`. Read once at terminal construction.
    pub cursor_shape: Option<CursorShapeConfig>,
    /// US-008: cursor blink override. `None` resolves to `TerminalControlled`
    /// (defer to DECSCUSR). Read once at terminal construction.
    pub cursor_blink: Option<CursorBlinkConfig>,
    /// US-014: global default extra environment variables injected into every
    /// new terminal PTY. Per-surface `env` ([`SurfaceDefinition::env`]) is
    /// merged on top of these (surface wins on key collision). The identity
    /// keys `TERM` and `COLORTERM` are always protected and cannot be
    /// overridden. On Windows, env names are case-insensitive, so user keys are
    /// normalised to uppercase before merging to avoid a `Path`/`PATH` clash.
    /// `None` (block absent) and `Some({})` both inject nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// US-022: scroll-wheel multiplier for the non-mouse-mode scrollback path.
    /// Multiplies the pixel delta before the line accumulator, so `> 1.0` speeds
    /// up trackpad/wheel scrollback and `< 1.0` slows it. Forced to `1.0` in
    /// mouse-reporting mode (the PTY owns scroll there; altering the delta would
    /// corrupt the report) and in the alt-screen alternate-scroll path. `None`
    /// resolves to `1.0`. Clamped to `[0.1, 10.0]`. Read live on each scroll
    /// event, so a config reload takes effect without a restart.
    pub scroll_multiplier: Option<f32>,
}

impl TerminalConfig {
    /// Default scrollback length matching Zed's `DEFAULT_SCROLL_HISTORY_LINES`.
    pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
    /// Lower bound: below 100 lines the buffer is too small to be useful.
    pub const MIN_SCROLLBACK_LINES: usize = 100;
    /// Upper bound: 100K lines × ~80 cols × cell ≈ 1 GiB ceiling.
    pub const MAX_SCROLLBACK_LINES: usize = 100_000;

    /// Default scroll multiplier: no amplification.
    pub const DEFAULT_SCROLL_MULTIPLIER: f32 = 1.0;
    /// Lower bound: below 0.1× scrollback would be nearly frozen.
    pub const MIN_SCROLL_MULTIPLIER: f32 = 0.1;
    /// Upper bound: beyond 10× a single tick jumps multiple screens.
    pub const MAX_SCROLL_MULTIPLIER: f32 = 10.0;

    /// Resolve `scroll_multiplier` to a usable value: default `1.0`, clamped to
    /// `[MIN_SCROLL_MULTIPLIER, MAX_SCROLL_MULTIPLIER]`. Emits a `warn!` when the
    /// user value is out of range so they notice the clamp.
    pub fn resolved_scroll_multiplier(&self) -> f32 {
        let raw = self
            .scroll_multiplier
            .unwrap_or(Self::DEFAULT_SCROLL_MULTIPLIER);
        // Guard NaN/infinity (serde rejects them from JSON, but an in-memory or
        // future caller could supply one): `f32::NAN.clamp(..)` is NaN and every
        // NaN comparison is false, which would slip a NaN through and freeze the
        // scroll accumulator. Fall back to the default instead.
        if !raw.is_finite() {
            return Self::DEFAULT_SCROLL_MULTIPLIER;
        }
        let clamped = raw.clamp(Self::MIN_SCROLL_MULTIPLIER, Self::MAX_SCROLL_MULTIPLIER);
        if (clamped - raw).abs() > f32::EPSILON {
            tracing::warn!(
                target: "paneflow_config::terminal",
                requested = raw,
                clamped,
                "terminal.scroll_multiplier out of range [{min}, {max}], clamped",
                min = Self::MIN_SCROLL_MULTIPLIER,
                max = Self::MAX_SCROLL_MULTIPLIER,
            );
        }
        clamped
    }

    /// Resolve the configured `scrollback_lines` to a usable value,
    /// applying default + clamp. Out-of-range values are clamped (a
    /// `warn!` is emitted on the first read so the user notices their
    /// config did not take effect verbatim).
    pub fn resolved_scrollback_lines(&self) -> usize {
        let raw = self
            .scrollback_lines
            .unwrap_or(Self::DEFAULT_SCROLLBACK_LINES);
        let clamped = raw.clamp(Self::MIN_SCROLLBACK_LINES, Self::MAX_SCROLLBACK_LINES);
        if clamped != raw {
            tracing::warn!(
                target: "paneflow_config::terminal",
                requested = raw,
                clamped,
                "terminal.scrollback_lines out of range [{min}, {max}], clamped",
                min = Self::MIN_SCROLLBACK_LINES,
                max = Self::MAX_SCROLLBACK_LINES,
            );
        }
        clamped
    }
}

/// Agents-view-scoped configuration block (US-103).
///
/// Lives in its own struct so future Phase B-E stories (thinking
/// display mode, profile selector, OS notification gate, ...) can
/// add fields without bloating the top-level [`PaneFlowConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentPanelConfig {
    /// Max width in pixels of the centered conversation column.
    /// `None` resolves to [`AgentPanelConfig::DEFAULT_MAX_CONTENT_WIDTH`]
    /// at the rendering layer; out-of-range values are clamped to
    /// `[MIN_CONTENT_WIDTH_PX, MAX_CONTENT_WIDTH_PX]` by
    /// [`AgentPanelConfig::resolved_max_content_width`] (US-103 AC #5).
    pub max_content_width: Option<u32>,
    /// How thinking / reasoning blocks render in the message stream.
    /// `None` resolves to [`ThinkingDisplayMode::Auto`] -- the v1
    /// behavior where the live burst is expanded and previous bursts
    /// collapse on their own (US-109 AC #1 / #2). An unknown string
    /// in this slot deserialises as `None` via the custom
    /// [`ThinkingDisplayMode`] deserialiser and a `warn!` is logged
    /// at first read (US-109 AC #7).
    pub thinking_display: Option<ThinkingDisplayMode>,
    /// US-115: user-saved named snapshots of
    /// (agent + model + mode + effort + tools). The composer's profile
    /// pill writes here when the user clicks "Save current as profile";
    /// the three built-in profiles (Write / Ask / Minimal) are NOT
    /// persisted -- they are seeded in-memory by the runtime and only
    /// appear here when the user explicitly customises one. Keys are
    /// the human-readable profile names.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub profiles: HashMap<String, ProfileConfig>,
    /// US-115: name of the profile applied on the next panel open.
    /// `None` falls back to the last-used profile (in-memory), and
    /// ultimately to the `Write` built-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// US-116: gates OS notifications fired when a turn ends, refuses,
    /// or errors out and the panel is not visible+focused. `None`
    /// resolves to [`NotifyWhenAgentWaiting::PrimaryScreen`] -- the
    /// safe default that fires on the user's primary display only.
    /// An unknown string deserialises to that same default via the
    /// custom [`NotifyWhenAgentWaiting`] deserialiser (AC #2 / unhappy
    /// path: invalid value never silently disables notifications).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_when_agent_waiting: Option<NotifyWhenAgentWaiting>,
}

/// US-115: persisted shape of one named profile in `paneflow.json`.
///
/// Every field is optional so a partial profile (e.g. "just lock the
/// effort to Low") round-trips cleanly. The apply path skips `None`
/// fields rather than treating them as a reset -- the user's current
/// state remains untouched for any field the profile does not pin.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProfileConfig {
    /// `AgentKind` discriminant string (`"claude_code"` | `"codex"`).
    /// Stored as `String` so this crate stays free of `paneflow-acp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Model id (e.g. `"claude-sonnet-4-5"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// ACP session mode id (e.g. `"default"`, `"acceptEdits"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// `ThinkingEffort` discriminant string (`"low"` | `"medium"` |
    /// `"high"` | `"xhigh"`). Composer maps the string back to its
    /// internal enum on apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Snake_case tool-kind keys (matches the persistence shape used
    /// by `tool_permissions` -- `read`, `edit`, `execute`, ...).
    /// Treated as the set the profile would prefer to "have on" for
    /// the picker UI; the actual permission resolution still goes
    /// through `tool_permissions`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

/// Per-thread display mode for thinking / reasoning blocks
/// (US-109 of `tasks/prd-agent-ui-refactor-2026-Q3.md`).
///
/// Mirrors Zed's `ThinkingBlockDisplay` enum cited in §12 of
/// `docs/ZED_AGENT_REFERENCE.md`. The default is [`Auto`] -- last
/// burst expanded, previous bursts collapsed to header-only.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ThinkingDisplayMode {
    /// Latest streaming burst expanded; previously-completed bursts
    /// collapse to header-only on next chunk arrival.
    #[default]
    Auto,
    /// Header + a fixed `max_h(256px)` body with a top gradient fade
    /// from `panel_bg.opacity(0.8)` to `transparent`. Lets the user
    /// skim every burst at a glance.
    Preview,
    /// Every thinking block stays expanded regardless of recency.
    AlwaysExpanded,
    /// Every thinking block stays collapsed to header-only; the user
    /// can still expand a single block manually.
    AlwaysCollapsed,
}

/// US-116: where OS notifications are surfaced when an agent turn
/// completes (or refuses / errors) while the panel is not focused.
///
/// Mirrors Zed's `NotifyWhenAgentWaiting` setting cited in §23 of
/// `docs/ZED_AGENT_REFERENCE.md`. Default is
/// [`NotifyWhenAgentWaiting::PrimaryScreen`] -- the user's primary
/// monitor only, which matches the macOS NSUserNotificationCenter +
/// Linux freedesktop daemon defaults.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum NotifyWhenAgentWaiting {
    /// Fire a notification only when Paneflow is not the focused window
    /// AND the user's primary display has no Paneflow window in front.
    /// Falls back to "any screen" in production -- the per-screen filter
    /// is OS-managed for the legacy `NSUserNotification` path on macOS
    /// and the freedesktop spec on Linux; we surface the toggle for
    /// parity but cannot enforce it ourselves.
    #[default]
    PrimaryScreen,
    /// Fire a notification on every display that is not currently
    /// hosting a focused Paneflow window.
    AllScreens,
    /// Never fire a notification. Disables the entire US-116 surface;
    /// no DBus / NSNotification / WinRT toast call is issued.
    Never,
}

impl<'de> Deserialize<'de> for NotifyWhenAgentWaiting {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        match raw.as_str() {
            "PrimaryScreen" => Ok(Self::PrimaryScreen),
            "AllScreens" => Ok(Self::AllScreens),
            "Never" => Ok(Self::Never),
            other => {
                tracing::warn!(
                    target: "paneflow_config::agent_panel",
                    value = other,
                    "agent_panel.notify_when_agent_waiting value not recognized, defaulting to PrimaryScreen",
                );
                Ok(Self::PrimaryScreen)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ThinkingDisplayMode {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        match raw.as_str() {
            "Auto" => Ok(Self::Auto),
            "Preview" => Ok(Self::Preview),
            "AlwaysExpanded" => Ok(Self::AlwaysExpanded),
            "AlwaysCollapsed" => Ok(Self::AlwaysCollapsed),
            other => {
                tracing::warn!(
                    target: "paneflow_config::agent_panel",
                    value = other,
                    "agent_panel.thinking_display value not recognized, defaulting to Auto",
                );
                Ok(Self::Auto)
            }
        }
    }
}

impl AgentPanelConfig {
    /// Default cap matching Zed's empirical sweet spot
    /// (`agent_panel.rs:4831`, cited in PRD §"Best Practices Applied").
    pub const DEFAULT_MAX_CONTENT_WIDTH: u32 = 760;
    /// Smallest cap the renderer accepts. Below this, lines start
    /// wrapping every few words and the column becomes unreadable.
    pub const MIN_CONTENT_WIDTH_PX: u32 = 320;
    /// Largest cap the renderer accepts. Above 4000px the cap is
    /// effectively a no-op on every monitor sold today.
    pub const MAX_CONTENT_WIDTH_PX: u32 = 4000;

    /// Resolve the configured `thinking_display` to a concrete mode,
    /// applying the [`ThinkingDisplayMode::Auto`] default when the
    /// field is missing (US-109 AC #1 / #7). Unknown string values are
    /// already filtered by the custom [`ThinkingDisplayMode`]
    /// deserialiser, so the only mapping needed here is `None` -> Auto.
    pub fn resolved_thinking_display(&self) -> ThinkingDisplayMode {
        self.thinking_display.unwrap_or_default()
    }

    /// Resolve the configured `notify_when_agent_waiting` to a concrete
    /// gate, applying the [`NotifyWhenAgentWaiting::PrimaryScreen`]
    /// default when the field is missing (US-116 AC #2). Unknown
    /// strings are already filtered by the custom
    /// [`NotifyWhenAgentWaiting`] deserialiser so the only mapping
    /// needed here is `None` -> `PrimaryScreen`.
    pub fn resolved_notify_when_agent_waiting(&self) -> NotifyWhenAgentWaiting {
        self.notify_when_agent_waiting.unwrap_or_default()
    }

    /// Resolve the configured `max_content_width` to a usable pixel
    /// value, applying default + clamp + a `warn!` line on out-of-range
    /// input (US-103 AC #1 / #5).
    pub fn resolved_max_content_width(&self) -> u32 {
        let raw = self
            .max_content_width
            .unwrap_or(Self::DEFAULT_MAX_CONTENT_WIDTH);
        let clamped = raw.clamp(Self::MIN_CONTENT_WIDTH_PX, Self::MAX_CONTENT_WIDTH_PX);
        if clamped != raw {
            tracing::warn!(
                target: "paneflow_config::agent_panel",
                requested = raw,
                clamped,
                "agent_panel.max_content_width out of range [{min}, {max}], clamped",
                min = Self::MIN_CONTENT_WIDTH_PX,
                max = Self::MAX_CONTENT_WIDTH_PX,
            );
        }
        clamped
    }
}

/// Desktop telemetry consent state.
///
/// Kept in its own struct (rather than a bare `Option<bool>` on
/// `PaneFlowConfig`) so future telemetry-scoped settings (e.g. a per-user
/// `distinct_id` override, or per-event category toggles) can be added
/// without schema churn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Consent tri-state. `None` = unanswered, `Some(true)` = opted in,
    /// `Some(false)` = opted out. `PANEFLOW_NO_TELEMETRY=1` env var
    /// overrides this unconditionally at the client layer (US-012).
    pub enabled: Option<bool>,
}

/// A single command definition, compatible with the cmux workspace format.
///
/// Each entry is either a workspace definition (with `workspace`) or a simple
/// shell command (with `command`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandDefinition {
    /// Display name (must not be blank).
    pub name: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Search keywords for fuzzy matching.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Workspace layout definition (mutually exclusive with `command`).
    pub workspace: Option<WorkspaceDefinition>,
    /// Shell command string (mutually exclusive with `workspace`).
    pub command: Option<String>,
}

/// Workspace definition containing layout, working directory, and visual config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceDefinition {
    /// Workspace display name.
    pub name: Option<String>,
    /// Default working directory for the workspace.
    pub cwd: Option<String>,
    /// Color as a 6-digit hex string (e.g. "ff6600").
    pub color: Option<String>,
    /// Root layout node describing pane arrangement.
    pub layout: Option<LayoutNode>,
}

/// A node in the layout tree: either a leaf pane or a split container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    /// A leaf pane containing one or more surfaces.
    Pane {
        /// Surfaces within this pane (must have >= 1).
        #[serde(default)]
        surfaces: Vec<SurfaceDefinition>,
    },
    /// A split container dividing space between 2 or more children.
    Split {
        /// Split direction: "horizontal" or "vertical".
        direction: String,
        /// Legacy: single split ratio for binary (2-child) layouts.
        /// Ignored when `ratios` is present. Defaults to 0.5 if omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratio: Option<f64>,
        /// Per-child ratios for N-ary layouts. When present, must have
        /// the same length as `children`. Values should sum to ~1.0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratios: Option<Vec<f64>>,
        /// 2 or more child layout nodes.
        #[serde(default)]
        children: Vec<LayoutNode>,
    },
}

impl LayoutNode {
    /// Count the number of leaf (Pane) nodes in the layout tree.
    pub fn leaf_count(&self) -> usize {
        match self {
            LayoutNode::Pane { .. } => 1,
            LayoutNode::Split { children, .. } => children.iter().map(|c| c.leaf_count()).sum(),
        }
    }

    /// Resolve per-child ratios for a Split node.
    ///
    /// Returns `ratios` if present, else converts legacy `ratio` to binary
    /// `[ratio, 1-ratio]`, else returns equal ratios for the child count.
    ///
    /// US-056: persisted ratios are untrusted input — a hand-edited or corrupt
    /// `session.json` can carry NaN, negative, zero, or wrong-length values. Any
    /// user-supplied set is run through [`sanitize_ratios`] (clamp into
    /// `[MIN_RATIO, 1.0]`, reject non-finite/negative, normalize to sum 1.0)
    /// before it reaches layout construction; the internally generated
    /// equal-share fallback is already valid and returned verbatim.
    pub fn resolved_ratios(&self) -> Vec<f64> {
        match self {
            LayoutNode::Pane { .. } => vec![1.0],
            LayoutNode::Split {
                ratio,
                ratios,
                children,
                ..
            } => {
                let n = children.len().max(1);
                let raw = if let Some(rs) = ratios {
                    rs.clone()
                } else if let Some(r) = ratio {
                    if children.len() == 2 {
                        vec![*r, 1.0 - *r]
                    } else {
                        return vec![1.0 / n as f64; n];
                    }
                } else {
                    return vec![1.0 / n as f64; n];
                };
                sanitize_ratios(raw, n)
            }
        }
    }
}

/// Floor for any single persisted split ratio. Clamping to this keeps every
/// pane visible and prevents a divide-by-zero when the set is normalized.
const MIN_RATIO: f64 = 0.01;

/// Clamp every ratio into `[MIN_RATIO, 1.0]` (mapping NaN/inf/negative to the
/// floor), then normalize so the set sums to 1.0. A length mismatch with the
/// child count is unrecoverable — we cannot know which child a stale ratio was
/// meant for — so it degrades to equal shares.
fn sanitize_ratios(mut ratios: Vec<f64>, n: usize) -> Vec<f64> {
    if ratios.len() != n {
        return vec![1.0 / n as f64; n];
    }
    for r in ratios.iter_mut() {
        *r = if r.is_finite() {
            r.clamp(MIN_RATIO, 1.0)
        } else {
            MIN_RATIO
        };
    }
    let sum: f64 = ratios.iter().sum();
    if sum > 0.0 && (sum - 1.0).abs() > 1e-9 {
        for r in ratios.iter_mut() {
            *r /= sum;
        }
    }
    // US-056 (EP-010 review): re-clamp after normalize. Dividing by a sum > 1
    // can push a just-clamped ratio back below `MIN_RATIO` (e.g. raw
    // `[1.0, 0.005]` → clamp `[1.0, 0.01]` → normalize `[0.990, 0.0099]`),
    // silently violating the floor this fn promises. The config-loader sibling
    // (`loader::validate_layout`) already re-clamps for this exact reason
    // (US-057); the session path must match so both frontiers honour the same
    // 0.01 floor. The renderer re-normalizes proportionally at paint time, so
    // the post-re-clamp sum need not be exactly 1.0 — the floor is the invariant.
    for r in ratios.iter_mut() {
        *r = r.clamp(MIN_RATIO, 1.0);
    }
    ratios
}

/// Top-level UI mode (US-007/US-008 of `prd-agents-view.md`;
/// `Diff` added by US-001 of `prd-git-diff-mode-2026-Q3.md`).
///
/// `Cli` is the traditional terminal-multiplexer view. `Diff` is the
/// dedicated git/worktree diff surface (left git panel + diff area).
/// `Agents` is the Agents view (project + thread sidebar + chat thread).
/// Default is `Cli` so existing users see no behaviour change on first
/// launch after upgrading. Variant order mirrors the on-screen segment
/// order (CLI / Diff / Agents) in `render_mode_toggle`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    #[default]
    Cli,
    Diff,
    Agents,
}

/// Persisted session state written to `~/.cache/paneflow/session.json`.
///
/// Backward-compat note: the three Agents-view fields (`projects`,
/// `active_project`, `mode`) all carry `#[serde(default)]`. Loading a
/// session.json written by a pre-US-007 build deserialises cleanly --
/// the missing keys resolve to an empty project list and `AppMode::Cli`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    /// Schema version for forward-compatible migrations.
    pub version: u32,
    /// Index of the active workspace at save time.
    pub active_workspace: usize,
    /// Ordered list of workspace snapshots.
    pub workspaces: Vec<WorkspaceSession>,
    /// Ordered list of project snapshots for the Agents view.
    /// US-007 of `tasks/prd-agents-view.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectSession>,
    /// Index of the active project at save time. `0` when no projects
    /// exist (the sidebar treats `projects.is_empty()` as the empty
    /// state regardless of this value).
    #[serde(default)]
    pub active_project: usize,
    /// Free chats — terminal threads not attached to any project, anchored
    /// on the user's home dir (US-002 of
    /// `prd-agents-ui-codex-redesign-2026-Q3.md`). A separate list from
    /// `projects` by design (no implicit "~" project). `skip_serializing_if`
    /// mirrors the `projects` field, so a pre-refonte session.json
    /// (without this key) restores as an empty chat list without touching
    /// the project serialization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chats: Vec<ThreadSession>,
    /// Last UI mode the user was in. The bootstrap reads this to
    /// reopen the Agents view if it was active at quit time (US-009).
    #[serde(default)]
    pub mode: AppMode,
    /// US-015 (prd-git-diff-mode-2026-Q3.md): the Git Diff view scope at save
    /// time, snake_case (`"project"` / `"multi_project"` / `"worktree"`),
    /// restored into `AppMode::Diff` on boot when reconstructable. Stored as a
    /// string so this config crate stays independent of the app's `DiffScope`
    /// type. Absent / `None` on sessions written before this field — defaults
    /// to the app's `DiffScope::default()` (Project).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_scope: Option<String>,
}

/// Persisted shape of one [`crate::project::Project`] (the runtime type
/// lives in `src-app/src/project/mod.rs`). The `id` is the in-memory
/// monotonic counter at save time -- it is restored on load so the
/// counter stays monotonic across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectSession {
    /// Monotonic in-memory ID at save time.
    pub id: u64,
    /// Human-readable title (sidebar header).
    pub title: String,
    /// Root cwd for new threads in this project.
    pub cwd: String,
    /// Whether the sidebar header was expanded at save time. `true`
    /// is the default for backward-compat (a missing key restores as
    /// "expanded" so an old session.json doesn't ghost the threads).
    #[serde(default = "default_true")]
    pub is_expanded: bool,
    /// Ordered list of thread snapshots in this project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<ThreadSession>,
}

/// Persisted shape of one thread (the runtime type lives in
/// `src-app/src/project/mod.rs`). Thread *content* (messages, tool
/// calls, attachments) is NOT stored here -- that lives in the
/// `paneflow-threads` SQLite DB (US-006). This struct holds only the
/// sidebar-visible metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadSession {
    /// Monotonic in-memory ID at save time.
    pub id: u64,
    /// Human-readable title (sidebar row).
    pub title: String,
    /// Wire-format tag for the agent. Canonical values: `"claude_code"`,
    /// `"codex"`. Stored as a `String` rather than a typed enum so
    /// `paneflow-config` does not need to depend on `paneflow-acp`
    /// (which would pull tokio + ACP into this lightweight crate).
    pub agent: String,
    /// Per-thread cwd. May differ from the parent project's cwd if the
    /// user explicitly forked into a subdirectory.
    pub cwd: String,
    /// Creation timestamp (unix-epoch milliseconds UTC). Used by the
    /// sidebar for relative-time labels.
    pub created_at: u64,
    /// Last selected model name from the agent's `session/new` response.
    /// `None` means "use the agent's default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Last selected ACP mode (Claude: `default`/`acceptEdits`/`plan`...).
    /// `None` means "use the agent's default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Foreign key into the `paneflow-threads` SQLite DB. `None` for
    /// threads that have never been persisted (the runtime layer sets
    /// this on first `append_message`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
    /// Runtime kind discriminant for the v1.x "Terminal Thread" surface
    /// (mirrors Zed's `AgentPanelEntryKind`). `None` (the default for
    /// every pre-Terminal-Thread session.json) restores as the legacy
    /// `Agent` kind. `Some("terminal")` restores as a Terminal Thread
    /// (PTY surface in the main area instead of a chat). Unknown
    /// strings fall back to `Agent` so a forward-rolled session from a
    /// future build does not ghost the row.
    ///
    /// Stored as a `String` rather than a typed enum so this crate
    /// stays free of the runtime `ThreadKind` enum (which lives in
    /// `src-app` to keep `paneflow-config` a leaf crate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Which CLI coding agent a Terminal Thread launches on first mount
    /// (`"claude_code"`, `"codex"`, `"opencode"`, `"pi"`, `"hermes"`).
    /// Drives the sidebar row icon and the auto-run command. `None`
    /// restores as a bare shell (legacy Terminal Threads + plain
    /// "New terminal thread" rows). Stored as a tag string so this crate
    /// stays free of the runtime `TerminalAgent` enum (which lives in
    /// `src-app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_agent: Option<String>,
    /// Whether the user pinned this thread (US-001 of
    /// `prd-agents-ui-codex-redesign-2026-Q3.md`). Pinned threads are
    /// surfaced in the rail's PINNED section across both projects and
    /// free chats. `#[serde(default)]` so a session.json written before
    /// this field deserialises cleanly as `false` — no migration.
    #[serde(default)]
    pub pinned: bool,
}

fn default_true() -> bool {
    true
}

/// Snapshot of a single workspace for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSession {
    /// Workspace display title.
    pub title: String,
    /// Root working directory of the workspace.
    pub cwd: String,
    /// Layout tree (splits + panes). `None` means a single default pane.
    pub layout: Option<LayoutNode>,
    /// User-defined command buttons rendered in this workspace's tab bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_buttons: Vec<ButtonCommand>,
    /// Workspace-relative directory paths expanded in the Files tree sidebar
    /// (PRD files-tree US-007). Additive + optional: absent in older
    /// `session.json` files, which deserialize to an empty list and never
    /// break restore of the other fields. The sidebar's open/closed state is
    /// deliberately NOT persisted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded_paths: Vec<String>,
    /// Git worktrees Paneflow created for this workspace via `paneflow up`
    /// (EP-002, prd-orchestration-v2). Persisted so a crash/restart keeps the
    /// ownership record (teardown at close, `git worktree prune` at startup).
    /// Additive + optional like `expanded_paths`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_worktrees: Vec<ManagedWorktreeDef>,
}

/// A git worktree created (and therefore owned) by Paneflow for one pane of a
/// `paneflow up` workspace. Paths are stored absolute; `teardown` is `"auto"`
/// (remove at close when clean) or `"keep"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ManagedWorktreeDef {
    /// Worktree checkout directory.
    pub path: String,
    /// Main repository root (where `git worktree` commands run).
    pub repo_root: String,
    /// Branch checked out in the worktree (diagnostics only — never deleted).
    pub branch: String,
    /// Teardown policy: `"auto"` | `"keep"`. Unknown values read as `"auto"`;
    /// the data-loss protection is the clean-check, not this flag.
    pub teardown: String,
}

/// A user-defined command button rendered in a workspace's tab bar.
/// Clicking the button sends `{command}\r` to the active terminal.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ButtonCommand {
    /// Stable identifier (opaque string) — survives reorderings and renames.
    pub id: String,
    /// Display name (also used as hover tooltip).
    pub name: String,
    /// Icon asset path relative to the `assets/` folder (e.g. `"icons/rocket.svg"`).
    pub icon: String,
    /// Shell command string, executed verbatim in the active terminal
    /// with a trailing `\r` appended (no bracketed-paste wrapping).
    pub command: String,
}

/// A surface within a pane (terminal, browser, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SurfaceDefinition {
    /// Surface type identifier: "terminal", "browser", etc.
    pub surface_type: Option<String>,
    /// Display name for this surface.
    pub name: Option<String>,
    /// User-assigned custom name (US-013). When set, it overrides the
    /// auto-derived surface name everywhere (sidebar/IPC `surface.list`/MCP),
    /// and survives restart via this field. Cleared by renaming to empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// Shell command to run in this surface.
    pub command: Option<String>,
    /// Working directory override for this surface.
    pub cwd: Option<String>,
    /// Extra environment variables.
    pub env: Option<HashMap<String, String>>,
    /// Whether this surface should receive initial focus.
    pub focus: Option<bool>,
    /// Saved scrollback text (plain, ANSI stripped). Up to 4000 lines / 400K chars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<String>,
    /// EP-005 US-013: stable tag of the agent CLI last detected in this
    /// surface's PTY subtree (e.g. `"claude_code"`), so the identity pill
    /// survives restart as a dimmed "last known" until the first scan
    /// confirms it. Whitelisted at ingress against the known agent tags;
    /// unknown or malformed values are dropped silently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// EP-006 US-019: per-pane font-size override in pixels. `None` =
    /// follow the global config. Validated at restore ingress (NaN/inf
    /// dropped, finite values clamped to [8.0, 32.0]) — never fed raw to
    /// the cell geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};

    fn object_keys(value: &serde_json::Value) -> BTreeSet<String> {
        value
            .as_object()
            .expect("expected JSON object")
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn public_json_schema_covers_every_config_field() {
        let mut permissions = HashMap::new();
        permissions.insert(
            "read".to_string(),
            ToolPermissionsEntry {
                always_allow: vec!["src/".to_string()],
                always_deny: vec!["secrets/".to_string()],
            },
        );
        let mut profiles = HashMap::new();
        profiles.insert(
            "Write".to_string(),
            ProfileConfig {
                agent: Some("codex".to_string()),
                model: Some("default".to_string()),
                mode: Some("default".to_string()),
                effort: Some("medium".to_string()),
                tools: vec!["read".to_string()],
            },
        );

        // Deliberately exhaustive struct literals: adding a Rust config field
        // fails this test at compile time until the public schema is updated.
        let config = PaneFlowConfig {
            shortcuts: HashMap::new(),
            default_shell: Some("sh".to_string()),
            theme: Some("One Dark".to_string()),
            commands: Vec::new(),
            window_decorations: Some("client".to_string()),
            line_height: Some(1.3),
            font_family: Some("Lilex".to_string()),
            font_size: Some(14.0),
            option_as_meta: Some(true),
            shell_integration: Some(true),
            agent_stall_detection: Some(true),
            agent_stall_threshold_secs: Some(300),
            external_editor: Some("auto".to_string()),
            claude_code_bypass_permissions: Some(false),
            claude_code_button_visible: Some(true),
            codex_button_visible: Some(true),
            opencode_button_visible: Some(true),
            pi_button_visible: Some(true),
            hermes_agent_button_visible: Some(true),
            grok_button_visible: Some(true),
            amp_button_visible: Some(true),
            cursor_button_visible: Some(true),
            gemini_button_visible: Some(true),
            kiro_button_visible: Some(true),
            antigravity_button_visible: Some(true),
            copilot_button_visible: Some(true),
            codebuddy_button_visible: Some(true),
            factory_button_visible: Some(true),
            qoder_button_visible: Some(true),
            openclaw_button_visible: Some(true),
            telemetry: Some(TelemetryConfig {
                enabled: Some(false),
            }),
            terminal: Some(TerminalConfig {
                ligatures: Some(false),
                scrollback_lines: Some(10_000),
                bell: Some(TerminalBellMode::Visual),
                cursor_shape: Some(CursorShapeConfig::Block),
                cursor_blink: Some(CursorBlinkConfig::TerminalControlled),
                env: Some(HashMap::new()),
                scroll_multiplier: Some(1.0),
            }),
            agent_panel: Some(AgentPanelConfig {
                max_content_width: Some(760),
                thinking_display: Some(ThinkingDisplayMode::Auto),
                profiles,
                default_profile: Some("Write".to_string()),
                notify_when_agent_waiting: Some(NotifyWhenAgentWaiting::PrimaryScreen),
            }),
            tool_permissions: permissions,
        };

        let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/paneflow.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
        let serialized = serde_json::to_value(config).unwrap();
        let mut schema_top_level = object_keys(&schema["properties"]);
        schema_top_level.remove("$schema");
        schema_top_level.remove("$schemaVersion");

        assert_eq!(
            object_keys(&serialized),
            schema_top_level,
            "top-level PaneFlowConfig and public JSON Schema drifted"
        );
        assert_eq!(
            object_keys(&serialized["terminal"]),
            object_keys(&schema["properties"]["terminal"]["properties"]),
            "TerminalConfig and public JSON Schema drifted"
        );
        assert_eq!(
            object_keys(&serialized["agent_panel"]),
            object_keys(&schema["properties"]["agent_panel"]["properties"]),
            "AgentPanelConfig and public JSON Schema drifted"
        );
        assert_eq!(
            object_keys(&serialized["agent_panel"]["profiles"]["Write"]),
            object_keys(&schema["definitions"]["profileConfig"]["properties"]),
            "ProfileConfig and public JSON Schema drifted"
        );
        assert_eq!(
            object_keys(&serialized["tool_permissions"]["read"]),
            object_keys(&schema["definitions"]["toolPermissionsEntry"]["properties"]),
            "ToolPermissionsEntry and public JSON Schema drifted"
        );
    }

    #[test]
    fn agent_stall_settings_resolve_with_defaults_and_clamp() {
        // EP-004 US-011: default ON, threshold 300 s.
        let cfg = PaneFlowConfig::default();
        assert!(cfg.agent_stall_detection_enabled());
        assert_eq!(cfg.resolved_agent_stall_threshold_secs(), 300);

        // Kill switch.
        let cfg = PaneFlowConfig {
            agent_stall_detection: Some(false),
            ..Default::default()
        };
        assert!(!cfg.agent_stall_detection_enabled());

        // Clamp both ends.
        let cfg = PaneFlowConfig {
            agent_stall_threshold_secs: Some(1),
            ..Default::default()
        };
        assert_eq!(cfg.resolved_agent_stall_threshold_secs(), 30);
        let cfg = PaneFlowConfig {
            agent_stall_threshold_secs: Some(u64::MAX),
            ..Default::default()
        };
        assert_eq!(cfg.resolved_agent_stall_threshold_secs(), 86_400);
        let cfg = PaneFlowConfig {
            agent_stall_threshold_secs: Some(600),
            ..Default::default()
        };
        assert_eq!(cfg.resolved_agent_stall_threshold_secs(), 600);
    }

    #[test]
    fn agent_panel_thinking_display_pascal_case_roundtrip() {
        // US-109 AC #1: PascalCase tags as documented in the PRD.
        let raw = r#"{"thinking_display": "Preview"}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.thinking_display, Some(ThinkingDisplayMode::Preview));

        let raw = r#"{"thinking_display": "AlwaysExpanded"}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(
            cfg.thinking_display,
            Some(ThinkingDisplayMode::AlwaysExpanded)
        );

        let raw = r#"{"thinking_display": "AlwaysCollapsed"}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(
            cfg.thinking_display,
            Some(ThinkingDisplayMode::AlwaysCollapsed)
        );

        let raw = r#"{"thinking_display": "Auto"}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.thinking_display, Some(ThinkingDisplayMode::Auto));
    }

    #[test]
    fn agent_panel_thinking_display_unknown_falls_back_to_auto() {
        // US-109 AC #7: unknown string deserialises as Auto (the
        // custom deserialiser logs a warn! line; this test asserts
        // only the surface behavior since `warn!` is not captured).
        let raw = r#"{"thinking_display": "Bogus"}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.thinking_display, Some(ThinkingDisplayMode::Auto));
    }

    #[test]
    fn agent_panel_thinking_display_missing_resolves_to_auto() {
        // US-109 AC #1: missing field resolves to Auto via the
        // resolver (the on-disk Option stays `None`).
        let raw = r#"{}"#;
        let cfg: AgentPanelConfig = serde_json::from_str(raw).unwrap();
        assert!(cfg.thinking_display.is_none());
        assert_eq!(cfg.resolved_thinking_display(), ThinkingDisplayMode::Auto);
    }

    #[test]
    fn terminal_bell_mode_semantics_and_serde() {
        // US-005: feedback predicates per mode.
        assert!(TerminalBellMode::Audible.is_audible() && !TerminalBellMode::Audible.is_visual());
        assert!(TerminalBellMode::Both.is_audible() && TerminalBellMode::Both.is_visual());
        assert!(TerminalBellMode::Visual.is_visual() && !TerminalBellMode::Visual.is_audible());
        assert!(!TerminalBellMode::Off.is_audible() && !TerminalBellMode::Off.is_visual());

        // Default preserves the historical flash-only behavior.
        assert_eq!(TerminalBellMode::default(), TerminalBellMode::Visual);

        // snake_case config values round-trip.
        let cfg: TerminalConfig = serde_json::from_str(r#"{"bell": "audible"}"#).unwrap();
        assert_eq!(cfg.bell, Some(TerminalBellMode::Audible));
        let cfg: TerminalConfig = serde_json::from_str(r#"{"bell": "both"}"#).unwrap();
        assert_eq!(cfg.bell, Some(TerminalBellMode::Both));
        let cfg: TerminalConfig = serde_json::from_str(r#"{"bell": "off"}"#).unwrap();
        assert_eq!(cfg.bell, Some(TerminalBellMode::Off));

        // Missing field → None → resolves to Visual (historical default).
        // (A typo'd value errors at parse time and is absorbed by the loader's
        // whole-config default fallback in `loader::load_config`, never a panic.)
        let cfg: TerminalConfig = serde_json::from_str(r#"{}"#).unwrap();
        assert!(cfg.bell.is_none());
        assert_eq!(cfg.bell.unwrap_or_default(), TerminalBellMode::Visual);
    }

    #[test]
    fn cursor_shape_and_blink_config_serde() {
        // US-007 / US-008: snake_case config values + historical defaults.
        assert_eq!(CursorShapeConfig::default(), CursorShapeConfig::Block);
        assert_eq!(
            CursorBlinkConfig::default(),
            CursorBlinkConfig::TerminalControlled
        );

        let cfg: TerminalConfig =
            serde_json::from_str(r#"{"cursor_shape": "beam", "cursor_blink": "off"}"#).unwrap();
        assert_eq!(cfg.cursor_shape, Some(CursorShapeConfig::Beam));
        assert_eq!(cfg.cursor_blink, Some(CursorBlinkConfig::Off));

        let cfg: TerminalConfig = serde_json::from_str(r#"{"cursor_shape": "hollow"}"#).unwrap();
        assert_eq!(cfg.cursor_shape, Some(CursorShapeConfig::Hollow));

        // Missing → None → resolves to historical defaults.
        let cfg: TerminalConfig = serde_json::from_str(r#"{}"#).unwrap();
        assert!(cfg.cursor_shape.is_none() && cfg.cursor_blink.is_none());
        assert_eq!(
            cfg.cursor_shape.unwrap_or_default(),
            CursorShapeConfig::Block
        );
        assert_eq!(
            cfg.cursor_blink.unwrap_or_default(),
            CursorBlinkConfig::TerminalControlled
        );
    }
}
