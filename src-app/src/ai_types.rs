//! AI tool type definitions shared across the app.
//!
//! `AiTool` identifies which AI coding tool is active (Claude Code, Codex).
//! `AiToolState` tracks the lifecycle state visible in the sidebar.
//! State transitions are driven by IPC hooks from the claude wrapper script.

/// Which AI coding tool is active in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiTool {
    /// Claude Code CLI (Anthropic)
    Claude,
    /// Codex CLI (OpenAI)
    Codex,
}

impl AiTool {
    pub fn label(&self) -> &'static str {
        match self {
            AiTool::Claude => "Claude",
            AiTool::Codex => "Codex",
        }
    }

    /// Wire-format string used by the `ai.*` IPC `tool` parameter.
    #[allow(dead_code)]
    pub fn wire_id(&self) -> &'static str {
        match self {
            AiTool::Claude => "claude",
            AiTool::Codex => "codex",
        }
    }

    /// Parse a tool name string into an `AiTool`. Case-insensitive.
    /// Defaults to `Claude` for unknown names.
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("codex") {
            AiTool::Codex
        } else {
            AiTool::Claude
        }
    }
}

/// AI tool session state visible to the rest of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiToolState {
    /// No AI tool session detected in this terminal.
    Inactive,
    /// AI tool is processing a prompt or using tools.
    Thinking(AiTool),
    /// AI tool needs user input or approval.
    WaitingForInput(AiTool),
    /// AI tool finished its response.
    Finished(AiTool),
}
