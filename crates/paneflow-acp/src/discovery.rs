//! Agent identity enum.
//!
//! The PATH-based ACP agent discovery (scanner, `which` probing, focus
//! refresh) was removed along with the "Connect" discovery shell - the
//! Agents view is terminal-only and each agent self-authenticates in its
//! own launched terminal. Only the [`AgentKind`] identity enum remains,
//! kept here so existing `paneflow_acp::AgentKind` references (legacy
//! `Thread.agent` data) keep resolving.

/// The ACP agents Paneflow tracks for legacy thread metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    /// Human-readable label.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    /// All agent kinds in display order.
    pub fn all() -> [AgentKind; 2] {
        [Self::ClaudeCode, Self::Codex]
    }
}
