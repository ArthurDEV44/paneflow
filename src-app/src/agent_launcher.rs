//! Terminal-agent launcher: the CLI coding agents Paneflow starts in a
//! terminal pane (Claude Code, Codex, OpenCode, Pi, Hermes, plus the
//! cmux-derived set: Grok, Amp, Cursor, Gemini, Kiro, Antigravity,
//! Copilot, CodeBuddy, Factory, Qoder, plus Openclaw). Both the tab-bar
//! launcher buttons
//! (`pane.rs`) and the Agents-view "New thread" picker iterate this single
//! source of truth so the per-agent visibility gate and the "respect
//! bypass" contract can never drift between them.
//!
//! Each variant maps to a display name, an icon, an accent tint, a
//! Settings → AI Agent visibility flag (`*_button_visible`), a stable
//! persistence tag, and a launch command. The launch command honors
//! `claude_code_bypass_permissions` exactly as the tab bar does.

use std::collections::HashSet;
use std::sync::OnceLock;

use paneflow_config::schema::PaneFlowConfig;

/// One of the CLI coding agents Paneflow can launch in a terminal.
///
/// Distinct from [`paneflow_acp::AgentKind`] (Claude/Codex only, the ACP
/// wire agents): this is the broader set surfaced as terminal launchers
/// and bound to Agents-view Terminal Threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAgent {
    ClaudeCode,
    Codex,
    OpenCode,
    Pi,
    Hermes,
    Grok,
    Amp,
    Cursor,
    Gemini,
    Kiro,
    Antigravity,
    Copilot,
    CodeBuddy,
    Factory,
    Qoder,
    Openclaw,
}

impl TerminalAgent {
    /// Every variant, in display order (matches the tab-bar button row).
    /// The original five lead; the cmux-derived launchers follow so the
    /// button order is stable for users who upgraded from a 5-agent build.
    pub const ALL: [TerminalAgent; 16] = [
        TerminalAgent::ClaudeCode,
        TerminalAgent::Codex,
        TerminalAgent::OpenCode,
        TerminalAgent::Pi,
        TerminalAgent::Hermes,
        TerminalAgent::Grok,
        TerminalAgent::Amp,
        TerminalAgent::Cursor,
        TerminalAgent::Gemini,
        TerminalAgent::Kiro,
        TerminalAgent::Antigravity,
        TerminalAgent::Copilot,
        TerminalAgent::CodeBuddy,
        TerminalAgent::Factory,
        TerminalAgent::Qoder,
        TerminalAgent::Openclaw,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            TerminalAgent::ClaudeCode => "Claude Code",
            TerminalAgent::Codex => "Codex",
            TerminalAgent::OpenCode => "OpenCode",
            TerminalAgent::Pi => "Pi",
            TerminalAgent::Hermes => "Hermes Agent",
            TerminalAgent::Grok => "Grok",
            TerminalAgent::Amp => "Amp",
            TerminalAgent::Cursor => "Cursor",
            TerminalAgent::Gemini => "Gemini",
            TerminalAgent::Kiro => "Kiro",
            TerminalAgent::Antigravity => "Antigravity",
            TerminalAgent::Copilot => "Copilot",
            TerminalAgent::CodeBuddy => "CodeBuddy",
            TerminalAgent::Factory => "Factory",
            TerminalAgent::Qoder => "Qoder",
            TerminalAgent::Openclaw => "Openclaw",
        }
    }

    pub fn icon_path(self) -> &'static str {
        match self {
            TerminalAgent::ClaudeCode => "icons/claude-color.svg",
            TerminalAgent::Codex => "icons/codex-color.svg",
            TerminalAgent::OpenCode => "icons/opencode-color.svg",
            TerminalAgent::Pi => "icons/pi-coding-agent.svg",
            TerminalAgent::Hermes => "icons/hermesagent.svg",
            TerminalAgent::Grok => "agents/grok.svg",
            TerminalAgent::Amp => "agents/amp-color.svg",
            TerminalAgent::Cursor => "agents/cursor.svg",
            TerminalAgent::Gemini => "agents/gemini-color.svg",
            TerminalAgent::Kiro => "agents/kiro-color.svg",
            TerminalAgent::Antigravity => "agents/antigravity-color.svg",
            TerminalAgent::Copilot => "agents/githubcopilot.svg",
            TerminalAgent::CodeBuddy => "agents/codebuddy-color.svg",
            TerminalAgent::Factory => "agents/factory.svg",
            TerminalAgent::Qoder => "agents/qoder-color.svg",
            TerminalAgent::Openclaw => "agents/openclaw-color.svg",
        }
    }

    /// Brand accent for the icon tint, as a packed `0xRRGGBB`. `None`
    /// means "use the theme's primary text color" -- the OpenCode / Pi /
    /// Hermes logos are monochrome `currentColor` SVGs.
    pub fn accent(self) -> Option<u32> {
        match self {
            TerminalAgent::ClaudeCode => Some(0xd97757),
            TerminalAgent::Codex => Some(0x7a9dff),
            // Single-color brand logos: `svg()` renders a monochrome alpha
            // mask, so the silhouette is painted in this brand color.
            TerminalAgent::Amp => Some(0xF34E3F),
            TerminalAgent::Qoder => Some(0x2ADB5C),
            // The rest are either monochrome `currentColor` logos (tinted
            // with the theme's primary text color so they stay readable on
            // every theme) or multi-color logos rendered in their native
            // palette via `img()` (see `icon_multicolor`), where `accent`
            // is unused.
            TerminalAgent::OpenCode
            | TerminalAgent::Pi
            | TerminalAgent::Hermes
            | TerminalAgent::Grok
            | TerminalAgent::Cursor
            | TerminalAgent::Gemini
            | TerminalAgent::Kiro
            | TerminalAgent::Antigravity
            | TerminalAgent::Copilot
            | TerminalAgent::CodeBuddy
            | TerminalAgent::Factory
            | TerminalAgent::Openclaw => None,
        }
    }

    /// Whether the icon must be rendered in its native colors via `img()`
    /// (multi-color logos: gradients or several distinct fills) instead of
    /// a `text_color`-tinted monochrome `svg()` mask. GPUI's `svg()`
    /// flattens every path to one tint, which would destroy these palettes;
    /// `img()` rasterizes the SVG (resvg) and preserves every fill. A
    /// single-color brand logo stays monochrome and uses `accent()`.
    pub fn icon_multicolor(self) -> bool {
        matches!(
            self,
            TerminalAgent::Antigravity
                | TerminalAgent::CodeBuddy
                | TerminalAgent::Gemini
                | TerminalAgent::Kiro
                | TerminalAgent::Openclaw
        )
    }

    /// Stable persistence tag for the session.json `terminal_agent`
    /// field. Kept distinct from the binary name so a future rename of
    /// the CLI does not invalidate persisted threads.
    pub fn tag(self) -> &'static str {
        match self {
            TerminalAgent::ClaudeCode => "claude_code",
            TerminalAgent::Codex => "codex",
            TerminalAgent::OpenCode => "opencode",
            TerminalAgent::Pi => "pi",
            TerminalAgent::Hermes => "hermes",
            TerminalAgent::Grok => "grok",
            TerminalAgent::Amp => "amp",
            TerminalAgent::Cursor => "cursor",
            TerminalAgent::Gemini => "gemini",
            TerminalAgent::Kiro => "kiro",
            TerminalAgent::Antigravity => "antigravity",
            TerminalAgent::Copilot => "copilot",
            TerminalAgent::CodeBuddy => "codebuddy",
            TerminalAgent::Factory => "factory",
            TerminalAgent::Qoder => "qoder",
            TerminalAgent::Openclaw => "openclaw",
        }
    }

    /// Map an ACP [`paneflow_acp::AgentKind`] (Claude/Codex only) to its
    /// terminal launcher. Used to relaunch legacy chat threads (which
    /// stored an `AgentKind`) as terminal sessions of the same agent.
    pub fn from_agent_kind(kind: paneflow_acp::AgentKind) -> TerminalAgent {
        match kind {
            paneflow_acp::AgentKind::ClaudeCode => TerminalAgent::ClaudeCode,
            paneflow_acp::AgentKind::Codex => TerminalAgent::Codex,
        }
    }

    pub fn from_tag(tag: &str) -> Option<TerminalAgent> {
        match tag {
            "claude_code" => Some(TerminalAgent::ClaudeCode),
            "codex" => Some(TerminalAgent::Codex),
            "opencode" => Some(TerminalAgent::OpenCode),
            "pi" => Some(TerminalAgent::Pi),
            "hermes" => Some(TerminalAgent::Hermes),
            "grok" => Some(TerminalAgent::Grok),
            "amp" => Some(TerminalAgent::Amp),
            "cursor" => Some(TerminalAgent::Cursor),
            "gemini" => Some(TerminalAgent::Gemini),
            "kiro" => Some(TerminalAgent::Kiro),
            "antigravity" => Some(TerminalAgent::Antigravity),
            "copilot" => Some(TerminalAgent::Copilot),
            "codebuddy" => Some(TerminalAgent::CodeBuddy),
            "factory" => Some(TerminalAgent::Factory),
            "qoder" => Some(TerminalAgent::Qoder),
            "openclaw" => Some(TerminalAgent::Openclaw),
            _ => None,
        }
    }

    /// Whether this launcher is shown in the tab bar / Agents-view picker.
    ///
    /// Tri-state on the `*_button_visible` config key:
    /// - `Some(true)`  — user explicitly enabled it: always shown.
    /// - `Some(false)` — user explicitly disabled it: always hidden.
    /// - `None` (key absent, the default) — shown only if the agent's CLI
    ///   binary is installed ([`Self::is_installed`]), so a fresh config
    ///   surfaces exactly the agents present on the machine. The user can
    ///   still force-show an uninstalled agent by toggling it on.
    pub fn is_visible(self, config: &PaneFlowConfig) -> bool {
        let explicit: Option<bool> = match self {
            TerminalAgent::ClaudeCode => config.claude_code_button_visible,
            TerminalAgent::Codex => config.codex_button_visible,
            TerminalAgent::OpenCode => config.opencode_button_visible,
            TerminalAgent::Pi => config.pi_button_visible,
            TerminalAgent::Hermes => config.hermes_agent_button_visible,
            TerminalAgent::Grok => config.grok_button_visible,
            TerminalAgent::Amp => config.amp_button_visible,
            TerminalAgent::Cursor => config.cursor_button_visible,
            TerminalAgent::Gemini => config.gemini_button_visible,
            TerminalAgent::Kiro => config.kiro_button_visible,
            TerminalAgent::Antigravity => config.antigravity_button_visible,
            TerminalAgent::Copilot => config.copilot_button_visible,
            TerminalAgent::CodeBuddy => config.codebuddy_button_visible,
            TerminalAgent::Factory => config.factory_button_visible,
            TerminalAgent::Qoder => config.qoder_button_visible,
            TerminalAgent::Openclaw => config.openclaw_button_visible,
        };
        explicit.unwrap_or_else(|| self.is_installed())
    }

    /// The CLI executable looked up on `PATH` to decide default visibility;
    /// also the leading token of [`Self::launch_command`]. Cross-platform:
    /// `which` resolves Windows `.exe`/`PATHEXT` extensions.
    pub fn binary(self) -> &'static str {
        match self {
            TerminalAgent::ClaudeCode => "claude",
            TerminalAgent::Codex => "codex",
            TerminalAgent::OpenCode => "opencode",
            TerminalAgent::Pi => "pi",
            TerminalAgent::Hermes => "hermes",
            TerminalAgent::Grok => "grok",
            TerminalAgent::Amp => "amp",
            TerminalAgent::Cursor => "cursor-agent",
            TerminalAgent::Gemini => "gemini",
            TerminalAgent::Kiro => "kiro-cli",
            TerminalAgent::Antigravity => "agy",
            TerminalAgent::Copilot => "copilot",
            TerminalAgent::CodeBuddy => "codebuddy",
            TerminalAgent::Factory => "droid",
            TerminalAgent::Qoder => "qodercli",
            TerminalAgent::Openclaw => "openclaw",
        }
    }

    /// Whether this agent's CLI binary is found on `PATH`. Drives the
    /// default visibility in [`Self::is_visible`].
    ///
    /// Probed once per process and cached: `which` walks `PATH` for every
    /// agent, too costly to repeat on the render thread each frame. The
    /// cache is filled lazily on first call (after `main.rs` patches `PATH`
    /// for GUI launches), so installing an agent while Paneflow runs needs a
    /// restart to flip the default — the user can toggle it on immediately
    /// in Settings meanwhile.
    pub fn is_installed(self) -> bool {
        installed_binaries().contains(self.binary())
    }

    /// The shell command run to start the agent. Honors
    /// `claude_code_bypass_permissions` for Claude Code (the only agent
    /// with a wired bypass flag today); the others launch bare, matching
    /// the tab-bar buttons verbatim. `clear &&` wipes the shell startup
    /// banner so the agent TUI owns the viewport from the first frame.
    pub fn launch_command(self, config: &PaneFlowConfig) -> &'static str {
        match self {
            TerminalAgent::ClaudeCode => {
                if config.claude_code_bypass_permissions.unwrap_or(false) {
                    "clear && claude --permission-mode bypassPermissions"
                } else {
                    "clear && claude"
                }
            }
            TerminalAgent::Codex => "clear && codex",
            TerminalAgent::OpenCode => "clear && opencode",
            TerminalAgent::Pi => "clear && pi",
            TerminalAgent::Hermes => "clear && hermes",
            TerminalAgent::Grok => "clear && grok",
            TerminalAgent::Amp => "clear && amp",
            // Cursor's CLI binary is `cursor-agent`, not `cursor`.
            TerminalAgent::Cursor => "clear && cursor-agent",
            TerminalAgent::Gemini => "clear && gemini",
            // Kiro's interactive entry point is the `chat` subcommand.
            TerminalAgent::Kiro => "clear && kiro-cli chat",
            // Antigravity ships as the `agy` binary.
            TerminalAgent::Antigravity => "clear && agy",
            TerminalAgent::Copilot => "clear && copilot",
            TerminalAgent::CodeBuddy => "clear && codebuddy",
            // Factory's CLI binary is `droid`.
            TerminalAgent::Factory => "clear && droid",
            // Qoder's CLI binary is `qodercli`.
            TerminalAgent::Qoder => "clear && qodercli",
            // Openclaw's interactive entry point is the `tui` subcommand.
            TerminalAgent::Openclaw => "clear && openclaw tui",
        }
    }

    /// Visible variants for the given config, in display order. Drives
    /// both the Agents-view picker and (via the same gates) the tab bar.
    pub fn visible(config: &PaneFlowConfig) -> Vec<TerminalAgent> {
        TerminalAgent::ALL
            .into_iter()
            .filter(|a| a.is_visible(config))
            .collect()
    }
}

/// Agent binaries found on `PATH`, probed once and cached for the process
/// lifetime. Backing store for [`TerminalAgent::is_installed`].
fn installed_binaries() -> &'static HashSet<&'static str> {
    static CACHE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CACHE.get_or_init(|| {
        TerminalAgent::ALL
            .into_iter()
            .map(TerminalAgent::binary)
            .filter(|bin| which::which(bin).is_ok())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_roundtrip() {
        for agent in TerminalAgent::ALL {
            assert_eq!(TerminalAgent::from_tag(agent.tag()), Some(agent));
        }
        assert_eq!(TerminalAgent::from_tag("unknown"), None);
    }

    #[test]
    fn binary_is_launch_command_leading_token() {
        // The PATH probe (`binary`) must match the actual executable the
        // launcher runs, or default visibility detects the wrong binary.
        let cfg = PaneFlowConfig::default();
        for agent in TerminalAgent::ALL {
            let after_clear = agent
                .launch_command(&cfg)
                .strip_prefix("clear && ")
                .expect("launch command must start with `clear && `");
            let leading = after_clear.split_whitespace().next().unwrap_or_default();
            assert_eq!(
                leading,
                agent.binary(),
                "{} binary must match its launch command's leading token",
                agent.display_name()
            );
        }
    }

    #[test]
    fn explicit_visibility_overrides_install_detection() {
        // `Some(true)`/`Some(false)` win over PATH detection, so the result
        // is deterministic on any machine (and never touches the filesystem
        // here — the `unwrap_or_else` install probe is short-circuited).
        let shown = PaneFlowConfig {
            gemini_button_visible: Some(true),
            ..Default::default()
        };
        assert!(TerminalAgent::Gemini.is_visible(&shown));

        let hidden = PaneFlowConfig {
            gemini_button_visible: Some(false),
            ..Default::default()
        };
        assert!(!TerminalAgent::Gemini.is_visible(&hidden));
    }

    #[test]
    fn every_launch_command_clears_first() {
        // The `clear &&` prefix is a hard contract: it wipes the shell
        // banner so each agent TUI owns the viewport from the first frame.
        let default = PaneFlowConfig::default();
        let bypass = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };
        for agent in TerminalAgent::ALL {
            assert!(
                agent.launch_command(&default).starts_with("clear && "),
                "{} launch command must start with `clear && `",
                agent.display_name()
            );
            assert!(
                agent.launch_command(&bypass).starts_with("clear && "),
                "{} launch command must start with `clear && ` under bypass",
                agent.display_name()
            );
        }
    }

    #[test]
    fn icon_paths_are_embedded_assets() {
        // Every icon must live under an embedded asset root (`icons/` or
        // `agents/`) or the tab-bar `svg()` silently renders nothing.
        for agent in TerminalAgent::ALL {
            let p = agent.icon_path();
            assert!(
                p.starts_with("icons/") || p.starts_with("agents/"),
                "{} icon path `{p}` is not under an embedded asset root",
                agent.display_name()
            );
        }
    }

    #[test]
    fn claude_bypass_flag_toggles_command() {
        let off = PaneFlowConfig {
            claude_code_bypass_permissions: Some(false),
            ..Default::default()
        };
        assert_eq!(
            TerminalAgent::ClaudeCode.launch_command(&off),
            "clear && claude"
        );
        let on = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };
        assert_eq!(
            TerminalAgent::ClaudeCode.launch_command(&on),
            "clear && claude --permission-mode bypassPermissions"
        );
    }

    #[test]
    fn non_claude_agents_ignore_bypass() {
        let config = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };
        assert_eq!(
            TerminalAgent::Codex.launch_command(&config),
            "clear && codex"
        );
        assert_eq!(TerminalAgent::Pi.launch_command(&config), "clear && pi");
        assert_eq!(
            TerminalAgent::Hermes.launch_command(&config),
            "clear && hermes"
        );
    }
}
