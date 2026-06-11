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
/// so the enum carries only the "visible" states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// Agent is processing a prompt or using tools.
    Thinking,
    /// Agent needs user input or approval (permission prompt, elicitation).
    WaitingForInput,
    /// Agent finished its response. Auto-cleared after 5 s by the IPC
    /// `ai.stop` handler unless overridden by a new state transition.
    Finished,
    /// EP-004 US-010 (cli-cockpit): the agent BINARY exited non-zero —
    /// reported by the shim's `ai.exit` frame (the shell's `ChildExit`
    /// only carries the shell's exit, never the agent's). Sticky until a
    /// new lifecycle event replaces it or its pane closes; never produced
    /// by a human interrupt (see [`state_for_exit`]).
    Errored,
    /// EP-004 US-011 (cli-cockpit): a `Thinking` session with no hook
    /// activity past the configured silence threshold. Flipped by the
    /// periodic sweep; any subsequent hook event replaces it immediately
    /// (never sticky).
    Stalled,
}

/// EP-004 US-010: classify the agent binary's raw exit code into the
/// session state it produces. Exit codes are reported by the shim with the
/// shell convention `128 + signum` for signal terminations (see
/// `paneflow-shim::exec::raw_exit_code_from_status`).
///
/// A termination *initiated from outside the agent* is not an agent
/// failure (FR-06: "une interruption humaine n'est PAS une erreur"):
/// - 130 (`128+SIGINT`) — Ctrl+C, the PRD-mandated case.
/// - 129 (`128+SIGHUP`) — pane/PTY closed under a running agent. Without
///   this exclusion every pane close with a live agent would flash a
///   false `Errored`.
/// - 143 (`128+SIGTERM`) / 137 (`128+SIGKILL`) — external kill.
/// - `STATUS_CONTROL_C_EXIT` (0xC000013A) — the Windows Ctrl+C exit code
///   (`code()` is always `Some` on Windows; there are no signals).
///
/// Genuine crash signals (SIGSEGV → 139, SIGABRT → 134, …) and every
/// other non-zero code classify as `Errored`.
pub fn state_for_exit(exit_code: i32) -> AgentState {
    /// `{Application Exit by CTRL+C}` — 0xC000013A as i32.
    const STATUS_CONTROL_C_EXIT: i32 = 0xC000_013Au32 as i32;
    match exit_code {
        0 => AgentState::Finished,
        129 | 130 | 137 | 143 => AgentState::Finished,
        STATUS_CONTROL_C_EXIT => AgentState::Finished,
        _ => AgentState::Errored,
    }
}

/// One row in the per-workspace `agent_sessions` map.
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub tool: AiTool,
    pub state: AgentState,
    /// Name of the active sub-tool (Edit, Bash, Read, …) reported by
    /// `ai.tool_use` hooks. Cleared on every non-Thinking transition.
    pub active_tool_name: Option<String>,
    /// The agent's question, from the `ai.notification` hook payload (≤512
    /// chars, UNTRUSTED terminal-adjacent text — display only, never
    /// interpreted). Set on `WaitingForInput`, cleared on `prompt_submit` /
    /// `stop` so a stale question never haunts the next turn (US-016).
    pub message: Option<String>,
    /// The surface (terminal entity id) this session runs in, resolved from
    /// the hook PID by walking the process ancestor chain to a known pane
    /// `child_pid` (US-017). `None` when unresolved — the session then only
    /// exists at workspace level (no per-pane glow), never a wrong pane.
    pub surface_id: Option<u64>,
    /// EP-004 US-011 (cli-cockpit): when the last `ai.*` lifecycle event
    /// for this session arrived. Stamped by `upsert_session_state` on every
    /// hook frame (prompt_submit / tool_use / notification / stop / exit);
    /// the periodic sweep flips a `Thinking` session to `Stalled` once this
    /// exceeds the configured silence threshold. Monotonic for the same
    /// reason as `waiting_since`.
    pub last_activity: std::time::Instant,
}

impl AgentSession {
    pub fn new(tool: AiTool, state: AgentState) -> Self {
        Self {
            tool,
            state,
            active_tool_name: None,
            message: None,
            surface_id: None,
            last_activity: std::time::Instant::now(),
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
/// multiple sessions in different states. `Errored` outranks everything
/// (a crash must never hide behind a sibling's spinner); `Stalled` sits
/// between `WaitingForInput` (actionable now) and `Thinking` (nominal).
fn state_rank(s: &AgentState) -> u8 {
    match s {
        AgentState::Errored => 5,
        AgentState::WaitingForInput => 4,
        AgentState::Stalled => 3,
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
    fn dominant_picks_errored_over_everything() {
        let sessions = [
            s(AiTool::Claude, AgentState::Thinking),
            s(AiTool::Claude, AgentState::WaitingForInput),
            s(AiTool::Claude, AgentState::Errored),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows[0].dominant, AgentState::Errored);
    }

    #[test]
    fn dominant_picks_waiting_over_stalled() {
        // A waiting agent is actionable NOW; a stalled one is a suspicion.
        let sessions = [
            s(AiTool::Claude, AgentState::Stalled),
            s(AiTool::Claude, AgentState::WaitingForInput),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows[0].dominant, AgentState::WaitingForInput);
    }

    #[test]
    fn dominant_picks_stalled_over_thinking() {
        let sessions = [
            s(AiTool::Claude, AgentState::Thinking),
            s(AiTool::Claude, AgentState::Stalled),
        ];
        let rows = aggregate_by_tool(sessions.iter());
        assert_eq!(rows[0].dominant, AgentState::Stalled);
    }

    #[test]
    fn exit_zero_and_interrupts_finish_everything_else_errors() {
        use AgentState::*;
        // FR-06: clean exit and human/external terminations are not errors.
        assert_eq!(state_for_exit(0), Finished);
        assert_eq!(state_for_exit(130), Finished, "128+SIGINT (Ctrl+C)");
        assert_eq!(state_for_exit(129), Finished, "128+SIGHUP (pane closed)");
        assert_eq!(state_for_exit(143), Finished, "128+SIGTERM");
        assert_eq!(state_for_exit(137), Finished, "128+SIGKILL");
        assert_eq!(
            state_for_exit(0xC000_013Au32 as i32),
            Finished,
            "Windows STATUS_CONTROL_C_EXIT"
        );
        // Genuine failures.
        assert_eq!(state_for_exit(1), Errored);
        assert_eq!(state_for_exit(2), Errored);
        assert_eq!(state_for_exit(127), Errored, "command not found");
        assert_eq!(state_for_exit(139), Errored, "128+SIGSEGV is a crash");
        assert_eq!(state_for_exit(134), Errored, "128+SIGABRT is a crash");
        assert_eq!(state_for_exit(-1), Errored, "negative non-Ctrl+C code");
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
