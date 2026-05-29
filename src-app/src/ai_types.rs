//! AI tool type definitions shared across the app.
//!
//! `AiTool` identifies which AI coding tool is active (Claude Code, Codex).
//! `AgentState` tracks the lifecycle state of a single agent session.
//! `AgentSession` bundles tool + state + the currently-active sub-tool name
//! (`Edit`, `Bash`, …) for one PID. A workspace can hold many sessions
//! concurrently — keyed by PID in `Workspace::agent_sessions`.
//!
//! State transitions are driven by IPC hooks from the `paneflow-ai-hook`
//! binary. Each lifecycle frame carries the emitting process's PID so the
//! server can route updates to the exact session rather than collapsing
//! everything per tool name (which broke when two Claude Codes ran in the
//! same workspace — the second `ai.session_start` used to overwrite the
//! first PID in a `HashMap<String, u32>`).

/// Which AI coding tool is active in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    /// Stable display order — Claude first, Codex second. Used by the
    /// sidebar to render multi-tool status rows deterministically rather
    /// than letting `HashMap` iteration order leak into the UI.
    pub fn display_order(&self) -> u8 {
        match self {
            AiTool::Claude => 0,
            AiTool::Codex => 1,
        }
    }
}

/// Lifecycle state for one agent session (one PID).
///
/// `Inactive` is implicit (a session that's not in the map is inactive),
/// so the enum carries only the three "visible" states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// Agent is processing a prompt or using tools.
    Thinking,
    /// Agent needs user input or approval (permission prompt, elicitation).
    WaitingForInput,
    /// Agent finished its response. Auto-cleared after 5 s by the IPC
    /// `ai.stop` handler unless overridden by a new state transition.
    Finished,
}

/// One row in the per-workspace `agent_sessions` map.
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub tool: AiTool,
    pub state: AgentState,
    /// Name of the active sub-tool (Edit, Bash, Read, …) reported by
    /// `ai.tool_use` hooks. Cleared on every non-Thinking transition.
    pub active_tool_name: Option<String>,
}

impl AgentSession {
    pub fn new(tool: AiTool, state: AgentState) -> Self {
        Self {
            tool,
            state,
            active_tool_name: None,
        }
    }
}

/// Aggregate of a workspace's sessions for a single tool, used by the
/// sidebar render. Computed on-the-fly from `agent_sessions` — never
/// stored. The "dominant" state is the most user-salient one across all
/// sessions of the same tool: `WaitingForInput > Thinking > Finished`.
/// `count` is the total number of sessions for this tool in any visible
/// state (i.e., everything in the map for that tool); `extra` is
/// `count - 1`, the "+N" suffix shown after the lead label.
#[derive(Debug, Clone)]
pub struct ToolAggregate {
    pub tool: AiTool,
    pub dominant: AgentState,
    pub count: usize,
    pub active_tool_name: Option<String>,
}

impl ToolAggregate {
    /// Render the `+N` suffix when more than one session of the same tool
    /// is active. Returns an empty string for a single session so the
    /// sidebar reads `Claude thinking…` (not `Claude thinking… +0`).
    pub fn extra_suffix(&self) -> String {
        if self.count > 1 {
            format!(" +{}", self.count - 1)
        } else {
            String::new()
        }
    }
}

/// Salience ranking used to pick the dominant state when a tool has
/// multiple sessions in different states.
fn state_rank(s: &AgentState) -> u8 {
    match s {
        AgentState::WaitingForInput => 3,
        AgentState::Thinking => 2,
        AgentState::Finished => 1,
    }
}

/// Aggregate the per-PID sessions of a workspace into one row per
/// `AiTool`, sorted by `AiTool::display_order`.
pub fn aggregate_by_tool<'a, I>(sessions: I) -> Vec<ToolAggregate>
where
    I: IntoIterator<Item = &'a AgentSession>,
{
    let mut by_tool: std::collections::HashMap<AiTool, ToolAggregate> =
        std::collections::HashMap::new();

    for s in sessions {
        by_tool
            .entry(s.tool)
            .and_modify(|agg| {
                agg.count += 1;
                if state_rank(&s.state) > state_rank(&agg.dominant) {
                    agg.dominant = s.state.clone();
                    agg.active_tool_name = s.active_tool_name.clone();
                }
            })
            .or_insert_with(|| ToolAggregate {
                tool: s.tool,
                dominant: s.state.clone(),
                count: 1,
                active_tool_name: s.active_tool_name.clone(),
            });
    }

    let mut rows: Vec<ToolAggregate> = by_tool.into_values().collect();
    rows.sort_by_key(|a| a.tool.display_order());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tool: AiTool, state: AgentState) -> AgentSession {
        AgentSession::new(tool, state)
    }

    #[test]
    fn aggregate_empty_yields_no_rows() {
        let rows = aggregate_by_tool(std::iter::empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn single_session_no_suffix() {
        let sessions = [s(AiTool::Claude, AgentState::Thinking)];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[0].extra_suffix(), "");
    }

    #[test]
    fn multi_same_tool_yields_plus_n_suffix() {
        let sessions = [
            s(AiTool::Claude, AgentState::Thinking),
            s(AiTool::Claude, AgentState::Thinking),
            s(AiTool::Claude, AgentState::Thinking),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 3);
        assert_eq!(rows[0].extra_suffix(), " +2");
    }

    #[test]
    fn dominant_picks_waiting_over_thinking() {
        let sessions = [
            s(AiTool::Claude, AgentState::Thinking),
            s(AiTool::Claude, AgentState::WaitingForInput),
            s(AiTool::Claude, AgentState::Finished),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows[0].dominant, AgentState::WaitingForInput);
    }

    #[test]
    fn dominant_picks_thinking_over_finished() {
        let sessions = [
            s(AiTool::Claude, AgentState::Finished),
            s(AiTool::Claude, AgentState::Thinking),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows[0].dominant, AgentState::Thinking);
    }

    #[test]
    fn claude_renders_before_codex() {
        let sessions = [
            s(AiTool::Codex, AgentState::Thinking),
            s(AiTool::Claude, AgentState::Thinking),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool, AiTool::Claude);
        assert_eq!(rows[1].tool, AiTool::Codex);
    }
}
