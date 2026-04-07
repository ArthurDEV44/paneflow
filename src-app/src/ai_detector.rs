//! AI tool session detection — event-driven state machine.
//!
//! Detects when AI coding tools (Claude Code, OpenAI Codex) are running in a
//! terminal by watching for tool name + spinner characters in the **OSC title**
//! (set via OSC 0/2). Both tools set the terminal title to include their name
//! and a rotating spinner while thinking — this IS the structured API.
//!
//! No grid scanning, no `/proc` introspection, no polling. Fed by title change
//! events from `TerminalState::sync()`.

use std::time::Instant;

/// Debug file logger — writes to `/tmp/paneflow-ai.log` bypassing env_logger.
/// Controlled by `PANEFLOW_AI_DEBUG=1` env var. Remove after debugging.
fn ai_debug_log(msg: &str) {
    use std::io::Write;
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| std::env::var("PANEFLOW_AI_DEBUG").is_ok());
    if !enabled {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/paneflow-ai.log")
    {
        let _ = writeln!(
            f,
            "[{:.3}] {msg}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        );
    }
}

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
}

/// AI tool session state visible to the rest of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiToolState {
    /// No AI tool session detected in this terminal.
    Inactive,
    /// AI tool is processing (spinner active in terminal title).
    Thinking(AiTool),
    /// AI tool stopped spinning and likely awaits user input or approval.
    WaitingForInput(AiTool),
    /// AI tool finished its response (process exited or title changed away).
    Finished(AiTool),
}

impl AiToolState {
    /// Whether this state represents an active (non-Inactive) session.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }

    /// Get the active tool, if any.
    pub fn tool(&self) -> Option<AiTool> {
        match self {
            Self::Inactive => None,
            Self::Thinking(t) | Self::WaitingForInput(t) | Self::Finished(t) => Some(*t),
        }
    }
}

// ---------------------------------------------------------------------------
// Title-based detection constants
// ---------------------------------------------------------------------------

/// Claude Code custom spinner glyphs (non-braille frames used in the title).
const CLAUDE_TITLE_SPINNER_CHARS: [char; 6] = ['✻', '✽', '✶', '✳', '✢', '·'];

/// How long after the spinner disappears from the title before we transition
/// to `WaitingForInput`. Tools briefly clear the title between operations.
const SPINNER_GONE_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(2);

/// How long to stay in `Finished` before auto-resetting to `Inactive`.
const FINISHED_RESET: std::time::Duration = std::time::Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Title analysis
// ---------------------------------------------------------------------------

/// Check if the title contains an active spinner character.
///
/// Matches any character in the Unicode Braille Patterns block (U+2801–U+28FF,
/// excluding the empty braille U+2800) plus Claude Code's custom spinner glyphs.
/// This covers all possible braille spinner variants without hardcoding frames.
fn title_has_spinner(title: &str) -> bool {
    title.chars().any(|c| {
        // Any non-empty braille character (covers ⠂ ⠐ ⠋ ⠙ ⠹ etc.)
        ('\u{2801}'..='\u{28FF}').contains(&c)
        // Claude Code custom glyphs (✻ ✽ ✶ ✳ ✢ ·)
        || CLAUDE_TITLE_SPINNER_CHARS.contains(&c)
    })
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

pub struct AiToolDetector {
    state: AiToolState,
    /// Which tool was last detected in the title (persists across title flickers).
    last_tool: Option<AiTool>,
    /// Timestamp when the spinner last disappeared from the title.
    spinner_gone_at: Option<Instant>,
    /// Timestamp when we entered `Finished` state.
    finished_at: Option<Instant>,
}

impl AiToolDetector {
    pub fn new() -> Self {
        ai_debug_log("AiToolDetector::new() — title-based detector initialized");
        Self {
            state: AiToolState::Inactive,
            last_tool: None,
            spinner_gone_at: None,
            finished_at: None,
        }
    }

    pub fn state(&self) -> AiToolState {
        self.state
    }

    /// Fed with the terminal's OSC title on each title change event.
    ///
    /// The title signals **whether** a tool is thinking (spinner present).
    /// It does NOT contain the tool name — Claude Code sets the title to
    /// `"{spinner} {project_name}"`. Tool identification uses a one-shot
    /// `/proc/cmdline` check, cached in `last_tool`.
    pub fn feed_title(&mut self, title: &str, child_pid: u32) -> Option<AiToolState> {
        let has_spinner = title_has_spinner(title);
        let old = self.state;

        ai_debug_log(&format!(
            "feed_title: spinner={has_spinner} cached_tool={:?} state={:?} title={title:?}",
            self.last_tool, self.state,
        ));

        if has_spinner {
            // Spinner in title — an AI tool is thinking.
            // Identify which tool via cached result or one-shot process check.
            let tool = self.last_tool.unwrap_or_else(|| {
                let detected = identify_ai_tool(child_pid);
                ai_debug_log(&format!("  process check (pid={child_pid}) → {detected:?}"));
                detected
            });
            self.last_tool = Some(tool);
            self.spinner_gone_at = None;
            self.finished_at = None;
            self.state = AiToolState::Thinking(tool);
        } else if matches!(self.state, AiToolState::Thinking(_)) {
            // Spinner gone from title — start grace timer.
            if self.spinner_gone_at.is_none() {
                self.spinner_gone_at = Some(Instant::now());
            }
        }

        if old != self.state {
            ai_debug_log(&format!("  → transition: {old:?} → {:?}", self.state));
            Some(self.state)
        } else {
            None
        }
    }

    /// Called periodically (every ~500ms) for timeout-based transitions.
    /// Returns `Some(new_state)` on a state transition.
    pub fn tick(&mut self) -> Option<AiToolState> {
        let old = self.state;

        match self.state {
            AiToolState::Thinking(tool) => {
                if let Some(gone_at) = self.spinner_gone_at
                    && gone_at.elapsed() > SPINNER_GONE_THRESHOLD
                {
                    self.state = AiToolState::WaitingForInput(tool);
                    self.spinner_gone_at = None;
                }
            }
            AiToolState::WaitingForInput(_) => {
                // Stay in WaitingForInput until title changes or child exits.
                // No timeout here — the tool is genuinely waiting for user input.
            }
            AiToolState::Finished(_) => {
                if let Some(at) = self.finished_at
                    && at.elapsed() > FINISHED_RESET
                {
                    *self = Self::new();
                    return Some(AiToolState::Inactive);
                }
            }
            AiToolState::Inactive => {}
        }

        if old != self.state {
            ai_debug_log(&format!("  tick → transition: {old:?} → {:?}", self.state));
            Some(self.state)
        } else {
            None
        }
    }

    /// Force transition to `Finished` (e.g., on ChildExit).
    pub fn force_finished(&mut self) {
        if let Some(tool) = self.state.tool() {
            self.state = AiToolState::Finished(tool);
        }
        self.finished_at = Some(Instant::now());
        self.spinner_gone_at = None;
    }
}

// ---------------------------------------------------------------------------
// Process identification (one-shot, cached by the detector)
// ---------------------------------------------------------------------------

/// Identify which AI tool is the foreground process in this terminal's PTY.
///
/// Uses `TIOCGPGRP` ioctl → `/proc/{pgid}/cmdline`. Called once per session
/// when a spinner first appears in the title. Defaults to `Claude` if the
/// process can't be read (most common case).
fn identify_ai_tool(shell_pid: u32) -> AiTool {
    let fd_path = format!("/proc/{shell_pid}/fd/0");
    let c_path = match std::ffi::CString::new(fd_path.as_str()) {
        Ok(p) => p,
        Err(_) => return AiTool::Claude,
    };
    // Safety: c_path is a valid NUL-terminated string.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return AiTool::Claude;
    }

    let mut fg_pgid: libc::pid_t = 0;
    // Safety: TIOCGPGRP writes a single pid_t. fd is a valid PTY slave.
    let ret = unsafe { libc::ioctl(fd, libc::TIOCGPGRP, &mut fg_pgid) };
    // Safety: we opened this fd, we close it.
    unsafe { libc::close(fd) };

    if ret != 0 || fg_pgid <= 0 {
        return AiTool::Claude;
    }

    let path = format!("/proc/{fg_pgid}/cmdline");
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => return AiTool::Claude,
    };
    for arg in data.split(|&b| b == 0) {
        let lower = String::from_utf8_lossy(arg).to_ascii_lowercase();
        if lower.contains("codex") {
            return AiTool::Codex;
        }
    }
    // Default — Claude Code is the most common case
    AiTool::Claude
}

#[cfg(test)]
mod tests {
    use super::*;

    // PID 99999 won't match any real process → identify_ai_tool defaults to Claude.
    const FAKE_PID: u32 = 99999;

    #[test]
    fn spinner_detection_in_title() {
        assert!(title_has_spinner("⠋ paneflow"));
        assert!(title_has_spinner("⠹ my-project"));
        assert!(title_has_spinner("✻ some-title"));
        assert!(!title_has_spinner("paneflow"));
        assert!(!title_has_spinner("normal title"));
    }

    #[test]
    fn spinner_in_title_triggers_thinking() {
        let mut d = AiToolDetector::new();
        assert_eq!(d.state(), AiToolState::Inactive);

        // Braille spinner in title → Thinking (defaults to Claude with fake PID)
        let result = d.feed_title("⠋ paneflow", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Thinking(AiTool::Claude));
        assert_eq!(result, Some(AiToolState::Thinking(AiTool::Claude)));
    }

    #[test]
    fn spinner_gone_starts_grace_timer() {
        let mut d = AiToolDetector::new();

        // Start thinking
        d.feed_title("⠋ paneflow", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Thinking(AiTool::Claude));

        // Spinner disappears → still Thinking (grace timer started)
        let result = d.feed_title("paneflow", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Thinking(AiTool::Claude));
        assert!(result.is_none()); // no transition yet
        assert!(d.spinner_gone_at.is_some());
    }

    #[test]
    fn no_spinner_stays_inactive() {
        let mut d = AiToolDetector::new();
        let result = d.feed_title("normal shell title", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Inactive);
        assert!(result.is_none());
    }

    #[test]
    fn force_finished_preserves_tool() {
        let mut d = AiToolDetector::new();
        d.feed_title("⠋ paneflow", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Thinking(AiTool::Claude));

        d.force_finished();
        assert_eq!(d.state(), AiToolState::Finished(AiTool::Claude));
        assert!(d.finished_at.is_some());
    }

    #[test]
    fn tool_cached_across_title_changes() {
        let mut d = AiToolDetector::new();

        // First spinner → identifies tool (defaults to Claude)
        d.feed_title("⠋ paneflow", FAKE_PID);
        assert_eq!(d.last_tool, Some(AiTool::Claude));

        // Subsequent spinners reuse cached tool, no re-check
        d.feed_title("⠹ paneflow", FAKE_PID);
        assert_eq!(d.state(), AiToolState::Thinking(AiTool::Claude));
    }
}
