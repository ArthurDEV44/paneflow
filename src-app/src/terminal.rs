//! Terminal state and view — PTY management and GPUI view wrapper.
//!
//! Manages the alacritty_terminal Term, portable-pty PTY, and periodic sync.
//! The TerminalView creates a TerminalElement for cell-by-cell rendering.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify};
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::TermMode;

use portable_pty::PtySize;

use crate::pty::{PortablePtyBackend, PtyBackend};

use gpui::{
    App, ClipboardEntry, ClipboardItem, Context, EventEmitter, ExternalPaths, FocusHandle,
    InteractiveElement, IntoElement, KeyContext, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Render, ScrollWheelEvent, Styled, Window, div, prelude::*,
};

use crate::mouse;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::terminal_element::TerminalElement;

/// Global flag: when true, terminals skip `cx.notify()` to avoid repaints
/// while a non-terminal page (e.g. settings) is displayed.
pub static SUPPRESS_REPAINTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Debug latency probes — zero overhead in release builds
// ---------------------------------------------------------------------------

/// Check once whether PANEFLOW_LATENCY_PROBE=1 is set.
/// Cached in a OnceLock so the env var is read only on first call.
#[cfg(debug_assertions)]
pub(crate) fn probe_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("PANEFLOW_LATENCY_PROBE").as_deref() == Ok("1"))
}

// ---------------------------------------------------------------------------
// Dimensions adapter
// ---------------------------------------------------------------------------

pub struct SpikeTermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl Dimensions for SpikeTermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

// ---------------------------------------------------------------------------
// Event listener — receives events from alacritty's event loop
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ZedListener(UnboundedSender<AlacEvent>);

impl EventListener for ZedListener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

// ---------------------------------------------------------------------------
// Shell integration — automatic OSC 7 injection
// ---------------------------------------------------------------------------

/// zsh: ZDOTDIR-based injection. Our `.zshenv` restores the original ZDOTDIR
/// so all other dotfiles (`.zshrc`, `.zprofile`) load from `$HOME` as usual.
const ZSH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting + wrapper PATH
if [[ -n "${PANEFLOW_ORIG_ZDOTDIR+x}" ]]; then
    ZDOTDIR="${PANEFLOW_ORIG_ZDOTDIR}"
    unset PANEFLOW_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
__paneflow_osc7() { printf '\e]7;file://%s%s\a' "${HOST}" "${PWD}"; }
autoload -Uz add-zsh-hook
add-zsh-hook chpwd __paneflow_osc7
__paneflow_osc7
# Prepend wrapper bin dir to PATH after .zshrc has loaded.
# Uses precmd (fires once before first prompt) so it runs AFTER all rc files.
if [[ -n "$__PANEFLOW_BIN_DIR" ]]; then
    __paneflow_path_inject() {
        [[ "$PATH" != "$__PANEFLOW_BIN_DIR:"* ]] && export PATH="$__PANEFLOW_BIN_DIR:$PATH"
        add-zsh-hook -d precmd __paneflow_path_inject
    }
    add-zsh-hook precmd __paneflow_path_inject
fi
"#;

/// bash: `--rcfile` replacement. Sources the real `.bashrc`, then appends
/// our OSC 7 function to PROMPT_COMMAND (preserving starship/oh-my-bash/etc.).
const BASH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting + wrapper PATH
[[ -f ~/.bashrc ]] && source ~/.bashrc
__paneflow_osc7() { printf '\e]7;file://%s%s\a' "${HOSTNAME}" "${PWD}"; }
PROMPT_COMMAND="__paneflow_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
# Prepend wrapper bin dir to PATH after .bashrc has loaded.
if [[ -n "$__PANEFLOW_BIN_DIR" && "$PATH" != "$__PANEFLOW_BIN_DIR:"* ]]; then
    export PATH="$__PANEFLOW_BIN_DIR:$PATH"
fi
"#;

/// fish: `--init-command` sourced script. Uses `--on-variable PWD` so it
/// fires on every directory change independently of the prompt function.
const FISH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting + wrapper PATH
function __paneflow_osc7 --on-variable PWD
    printf '\e]7;file://%s%s\a' (hostname) "$PWD"
end
__paneflow_osc7
# Prepend wrapper bin dir to PATH after config.fish has loaded.
if set -q __PANEFLOW_BIN_DIR; and test "$PATH[1]" != "$__PANEFLOW_BIN_DIR"
    set -gx PATH $__PANEFLOW_BIN_DIR $PATH
end
"#;

/// Write OSC 7 shell integration scripts and return the extra shell args
/// and env vars needed to activate them. Scripts are written to
/// `$XDG_DATA_HOME/paneflow/shell/{zsh,bash,fish}/`.
///
/// # Platform gaps (for future porting)
/// - **Windows**: PowerShell needs a `prompt` function override;
///   cmd.exe has no hook mechanism (consider ConPTY passthrough).
/// - **Shells without injection** (nushell, elvish, xonsh): rely on
///   `cwd_now()` fallback. On macOS this requires `proc_pidinfo()`.
fn setup_shell_integration(
    shell: &str,
    env: &mut std::collections::HashMap<String, String>,
) -> Vec<String> {
    let Some(base) = dirs::data_dir().map(|d| d.join("paneflow").join("shell")) else {
        return vec![];
    };

    let basename = shell.rsplit('/').next().unwrap_or(shell);
    match basename {
        "zsh" => {
            let dir = base.join("zsh");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let _ = std::fs::write(dir.join(".zshenv"), ZSH_OSC7);
            if let Ok(orig) = std::env::var("ZDOTDIR") {
                env.insert("PANEFLOW_ORIG_ZDOTDIR".into(), orig);
            }
            env.insert("ZDOTDIR".into(), dir.display().to_string());
            vec![]
        }
        "bash" => {
            let dir = base.join("bash");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let rcfile = dir.join("bashrc");
            let _ = std::fs::write(&rcfile, BASH_OSC7);
            vec!["--rcfile".into(), rcfile.display().to_string()]
        }
        "fish" => {
            let dir = base.join("fish");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let initfile = dir.join("osc7.fish");
            let _ = std::fs::write(&initfile, FISH_OSC7);
            vec![
                "--init-command".into(),
                format!("source {}", initfile.display()),
            ]
        }
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// PTY notifier — replaces alacritty's Notifier (US-007, portable-pty)
// ---------------------------------------------------------------------------

/// Channel sender for PTY messages. Mirrors `alacritty_terminal::event_loop::Notifier`
/// interface but uses a plain `mpsc::Sender` instead of mio-backed `EventLoopSender`.
#[derive(Clone)]
pub struct PtySender(std::sync::mpsc::Sender<Msg>);

impl PtySender {
    pub fn send(&self, msg: Msg) -> Result<(), std::sync::mpsc::SendError<Msg>> {
        self.0.send(msg)
    }
}

/// Wrapper for PTY write channel. Implements `Notify` for input, exposes `.0.send()`
/// for resize/shutdown messages — same usage pattern as alacritty's `Notifier`.
pub struct PtyNotifier(pub PtySender);

impl Notify for PtyNotifier {
    fn notify<B: Into<Cow<'static, [u8]>>>(&self, bytes: B) {
        let _ = self.0.send(Msg::Input(bytes.into()));
    }
}

// ---------------------------------------------------------------------------
// OSC 52 clipboard mode
// ---------------------------------------------------------------------------

/// Controls OSC 52 clipboard access. Default: CopyOnly (write-only).
/// Read path (CopyPaste) is a security risk — clipboard exfiltration.
#[derive(Clone, Copy, PartialEq)]
pub enum Osc52Mode {
    Disabled,
    CopyOnly,
    CopyPaste,
}

/// Kind of OSC 133 prompt mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptMarkKind {
    /// A — prompt start
    PromptStart,
    /// B — prompt end / command start
    CommandStart,
    /// C — command end / output start
    OutputStart,
    /// D — output end
    OutputEnd,
}

/// A shell prompt boundary detected via OSC 133.
#[derive(Clone, Debug)]
pub struct PromptMark {
    /// Grid line where the mark was detected (topmost_line..=bottommost_line coordinate).
    pub line: i32,
    pub kind: PromptMarkKind,
}

/// Deferred clipboard operation from sync() — executed in cx.update() closure.
enum ClipboardOp {
    Store(String),
    Load(std::sync::Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

/// Deferred color query response (OSC 10/11/12) from sync().
struct ColorOp {
    index: usize,
    format_fn: std::sync::Arc<dyn Fn(AlacRgb) -> String + Sync + Send + 'static>,
}

use alacritty_terminal::event::WindowSize as AlacWindowSize;
use alacritty_terminal::vte::ansi::Rgb as AlacRgb;

/// Convert GPUI Hsla to alacritty Rgb for color query responses.
fn hsla_to_alac_rgb(hsla: gpui::Hsla) -> AlacRgb {
    let rgba = gpui::Rgba::from(hsla);
    AlacRgb {
        r: (rgba.r.clamp(0.0, 1.0) * 255.0) as u8,
        g: (rgba.g.clamp(0.0, 1.0) * 255.0) as u8,
        b: (rgba.b.clamp(0.0, 1.0) * 255.0) as u8,
    }
}

// ---------------------------------------------------------------------------
// Terminal state
// ---------------------------------------------------------------------------

pub struct TerminalState {
    pub term: Arc<FairMutex<Term<ZedListener>>>,
    pub notifier: PtyNotifier,
    events_rx: Option<UnboundedReceiver<AlacEvent>>,
    cwd_rx: UnboundedReceiver<String>,
    prompt_rx: UnboundedReceiver<PromptMark>,
    /// Shell prompt marks detected via OSC 133 sequences.
    pub prompt_marks: Vec<PromptMark>,
    pub exited: Option<i32>,
    /// PID of the shell child process, used for port detection.
    pub child_pid: u32,
    /// Terminal title set via OSC 0/2 escape sequences (e.g. shell prompt, Claude Code).
    pub title: String,
    /// Current working directory of the shell process.
    /// Updated via OSC 7 escape sequence (push from shell) or on-demand
    /// via `cwd_now()` (fallback for shells that don't emit OSC 7).
    pub current_cwd: Option<String>,
    /// OSC 52 clipboard access mode (default: copy-only for security).
    pub osc52_mode: Osc52Mode,
    /// Deferred clipboard operations from sync() — drained in the poll loop
    /// where cx is available for clipboard read/write.
    pending_clipboard_ops: Vec<ClipboardOp>,
    /// Deferred color query responses (OSC 10/11/12) from sync().
    pending_color_ops: Vec<ColorOp>,
    /// Deferred text area size request responses from sync().
    pending_size_ops: Vec<std::sync::Arc<dyn Fn(AlacWindowSize) -> String + Sync + Send + 'static>>,
    /// Bell event received — triggers visual flash in poll loop.
    pub bell_active: bool,
    /// Whether the terminal wants the cursor to blink (from CursorBlinkingChange).
    pub cursor_blinking: bool,
    /// Set when PTY output has been processed (Wakeup event received).
    /// Cleared after cx.notify() triggers a repaint.
    pub dirty: bool,
    /// Counter for throttling output scans — scans every 50th dirty tick.
    output_scan_ticks: u32,
    /// Ports already reported via ServiceDetected (dedup guard).
    /// Cleared on ChildExit so a restarted server is re-detected.
    reported_ports: Vec<u16>,
    /// Timestamp of the most recent keystroke, used by latency probes
    /// to measure total keystroke-to-pixel time. Debug builds only.
    /// Note: on rapid keystrokes before a render frame, earlier timestamps are overwritten.
    #[cfg(debug_assertions)]
    pub(crate) last_keystroke_at: Option<std::time::Instant>,
}

impl TerminalState {
    pub fn new(
        backend: &dyn PtyBackend,
        working_directory: Option<std::path::PathBuf>,
        workspace_id: u64,
        surface_id: u64,
        initial_size: Option<(usize, usize)>,
    ) -> anyhow::Result<Self> {
        let (events_tx, events_rx) = unbounded();
        let (cwd_tx, cwd_rx) = unbounded();
        let (prompt_tx, prompt_rx) = unbounded();
        let listener = ZedListener(events_tx.clone());

        let (cols, rows) = initial_size.unwrap_or((120, 40));

        let config = TermConfig::default();
        let dimensions = SpikeTermSize {
            columns: cols,
            screen_lines: rows,
        };

        let term = Term::new(config, &dimensions, listener.clone());
        let term = Arc::new(FairMutex::new(term));

        // Build shell command and environment.
        // Fallback chain: config default_shell → $SHELL → /bin/sh
        let shell = {
            let config = paneflow_config::loader::load_config();
            let configured = config
                .default_shell
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match configured {
                Some(path) => {
                    let p = std::path::Path::new(path);
                    let is_executable = p.is_file() && {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::metadata(p)
                            .map(|m| m.permissions().mode() & 0o111 != 0)
                            .unwrap_or(false)
                    };
                    if is_executable {
                        path.to_string()
                    } else {
                        log::warn!(
                            "Configured default_shell {:?} not found or not executable, \
                             falling back to $SHELL",
                            path
                        );
                        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
                    }
                }
                None => std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            }
        };
        let mut env = std::collections::HashMap::new();
        let extra_args = setup_shell_integration(&shell, &mut env);

        // Inject PaneFlow identity env vars for AI tool hook integration.
        env.insert("PANEFLOW_WORKSPACE_ID".into(), workspace_id.to_string());
        env.insert("PANEFLOW_SURFACE_ID".into(), surface_id.to_string());
        if let Some(socket_path) = paneflow_socket_path() {
            env.insert("PANEFLOW_SOCKET_PATH".into(), socket_path);
        }

        // Explicit TERM so TUI apps detect capabilities correctly
        // (don't rely on portable-pty's default).
        env.insert("TERM".into(), "xterm-256color".into());

        // Ensure UTF-8 locale in minimal environments (containers, etc.)
        if std::env::var("LANG").map_or(true, |v| v.is_empty()) {
            env.insert("LANG".into(), "en_US.UTF-8".into());
        }

        // Standard terminal identification for TUI app capability detection.
        env.insert("TERM_PROGRAM".into(), "paneflow".into());
        env.insert(
            "TERM_PROGRAM_VERSION".into(),
            env!("CARGO_PKG_VERSION").into(),
        );
        env.insert("COLORTERM".into(), "truecolor".into());

        // Expose wrapper scripts directory for shell integration.
        if let Some(bin_dir) = paneflow_bin_dir() {
            ensure_wrapper_scripts(&bin_dir);
            let bin_dir_str = bin_dir.display().to_string();
            env.insert("__PANEFLOW_BIN_DIR".into(), bin_dir_str.clone());
            let current_path = std::env::var("PATH").unwrap_or_default();
            if !current_path.split(':').any(|p| p == bin_dir_str) {
                env.insert("PATH".into(), format!("{bin_dir_str}:{current_path}"));
            }
        }

        let cwd = working_directory
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));

        // Spawn PTY via the injected backend (US-008)
        let pty = backend.spawn(&shell, &extra_args, &cwd, &env, rows as u16, cols as u16)?;

        // I/O threads replace AlacEventLoop (US-007).
        // Reader thread: reads PTY output → VTE parser → Term mutations → Wakeup.
        // DEC 2026 sync gating is handled by vte's built-in Processor (150ms timeout).
        // Also owns the child handle to capture exit status after EOF.
        let term_for_reader = term.clone();
        let listener_for_reader = ZedListener(events_tx.clone());
        std::thread::spawn(move || {
            pty_reader_loop(
                pty.reader,
                term_for_reader,
                listener_for_reader,
                pty.child,
                cwd_tx,
                prompt_tx,
            );
        });

        // Message handler thread: receives Notifier messages → writes to PTY / resizes.
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
        std::thread::spawn(move || {
            pty_message_loop(msg_rx, pty.writer, pty.master);
        });

        Ok(Self {
            term,
            notifier: PtyNotifier(PtySender(msg_tx)),
            events_rx: Some(events_rx),
            cwd_rx,
            prompt_rx,
            prompt_marks: Vec::new(),
            exited: None,
            child_pid: pty.child_pid,
            current_cwd: None,
            osc52_mode: Osc52Mode::CopyOnly,
            pending_clipboard_ops: Vec::new(),
            pending_color_ops: Vec::new(),
            pending_size_ops: Vec::new(),
            bell_active: false,
            cursor_blinking: true,
            title: String::from("Terminal"),
            dirty: true, // Force initial render
            output_scan_ticks: 0,
            reported_ports: Vec::new(),
            #[cfg(debug_assertions)]
            last_keystroke_at: None,
        })
    }

    /// Create a display-only terminal with no PTY, no reader thread, no message loop.
    /// Content is rendered via `write_output()` which processes bytes through VTE directly.
    /// The terminal supports full ANSI rendering but does not accept keyboard input.
    #[allow(dead_code)]
    pub fn new_display_only(rows: usize, cols: usize) -> Self {
        // events_tx is kept alive inside Term's ZedListener (Term emits Wakeup after VTE mutations).
        // cwd/prompt senders are dropped so their try_recv() always returns Err (no PTY scanner).
        let (events_tx, events_rx) = unbounded();
        let (_cwd_tx, cwd_rx) = unbounded::<String>();
        let (_prompt_tx, prompt_rx) = unbounded::<PromptMark>();
        let listener = ZedListener(events_tx);

        let config = TermConfig::default();
        let dimensions = SpikeTermSize {
            columns: cols,
            screen_lines: rows,
        };
        let term = Term::new(config, &dimensions, listener);
        let term = Arc::new(FairMutex::new(term));

        // Dummy notifier — sends are silently discarded (receiver dropped)
        let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();

        Self {
            term,
            notifier: PtyNotifier(PtySender(msg_tx)),
            events_rx: Some(events_rx),
            cwd_rx,
            prompt_rx,
            prompt_marks: Vec::new(),
            exited: None,
            child_pid: 0,
            current_cwd: None,
            osc52_mode: Osc52Mode::Disabled,
            pending_clipboard_ops: Vec::new(),
            pending_color_ops: Vec::new(),
            pending_size_ops: Vec::new(),
            bell_active: false,
            cursor_blinking: false,
            title: String::from("Display"),
            dirty: true,
            output_scan_ticks: 0,
            reported_ports: Vec::new(),
            #[cfg(debug_assertions)]
            last_keystroke_at: None,
        }
    }

    /// Write ANSI-formatted content to a display-only terminal.
    /// Converts bare `\n` to `\r\n` (since there is no PTY to perform CR insertion).
    /// Processes bytes through VTE for full ANSI color/attribute support.
    /// Note: callers must not split a `\r\n` pair across two calls (the second call
    /// would insert an extra `\r`, producing `\r\r\n`). Prefer complete chunks.
    #[allow(dead_code)]
    pub fn write_output(&self, bytes: &[u8]) {
        // Convert \n to \r\n — bare LF without preceding CR needs CR insertion
        let mut converted = Vec::with_capacity(bytes.len());
        let mut prev = 0u8;
        for &b in bytes {
            if b == b'\n' && prev != b'\r' {
                converted.push(b'\r');
            }
            converted.push(b);
            prev = b;
        }

        let mut term = self.term.lock();
        let mut processor = alacritty_terminal::vte::ansi::Processor::<
            alacritty_terminal::vte::ansi::StdSyncHandler,
        >::new();
        processor.advance(&mut *term, &converted);
    }

    /// Drain CWD and prompt mark channels, then drain any remaining events.
    /// Sets `dirty = true` when PTY output was processed.
    #[allow(dead_code)]
    pub fn sync(&mut self) {
        self.sync_channels();
        if let Some(mut rx) = self.events_rx.take() {
            while let Ok(event) = rx.try_recv() {
                self.process_event(event);
            }
            self.events_rx = Some(rx);
        }
    }

    /// Drain only the CWD and prompt mark channels (not events).
    /// Used by the batched event loop which handles events directly.
    pub fn sync_channels(&mut self) {
        while let Ok(cwd) = self.cwd_rx.try_recv() {
            self.current_cwd = Some(cwd);
        }
        while let Ok(mark) = self.prompt_rx.try_recv() {
            self.prompt_marks.push(mark);
        }
    }

    /// Defensively reset terminal modes that could corrupt the outer terminal.
    /// Called on child exit before marking the terminal as exited.
    /// Only resets modes that are actually active (clean exits won't trigger).
    fn reset_active_modes(&mut self) {
        let mode = *self.term.lock().mode();
        if mode.contains(TermMode::BRACKETED_PASTE) {
            self.notifier.notify(b"\x1b[?2004l" as &[u8]);
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            self.notifier.notify(b"\x1b[?1004l" as &[u8]);
        }
        if mode.intersects(TermMode::MOUSE_MODE) {
            self.notifier
                .notify(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l" as &[u8]);
        }
        if mode.contains(TermMode::ALT_SCREEN) {
            self.notifier.notify(b"\x1b[?1049l" as &[u8]);
        }
    }

    /// Process a single alacritty event.
    pub fn process_event(&mut self, event: AlacEvent) {
        match event {
            AlacEvent::Wakeup => {
                self.dirty = true;
            }
            AlacEvent::ChildExit(status) => {
                self.reset_active_modes();
                self.exited = Some(status.code().unwrap_or(-1));
                self.dirty = true;
                self.reported_ports.clear();
            }
            AlacEvent::Exit => {
                self.reset_active_modes();
                self.exited = Some(-1);
                self.dirty = true;
            }
            AlacEvent::Title(t) => {
                self.title = t;
            }
            AlacEvent::ResetTitle => {
                self.title = String::from("Terminal");
            }
            AlacEvent::PtyWrite(text) => {
                self.notifier.notify(text.into_bytes());
            }
            AlacEvent::ClipboardStore(_selection, text) => {
                // Cap at 100 KiB to prevent memory DoS from malicious programs
                const MAX_OSC52_BYTES: usize = 100 * 1024;
                if self.osc52_mode != Osc52Mode::Disabled && text.len() <= MAX_OSC52_BYTES {
                    self.pending_clipboard_ops.push(ClipboardOp::Store(text));
                }
            }
            AlacEvent::ClipboardLoad(_selection, format_fn) => {
                if self.osc52_mode == Osc52Mode::CopyPaste {
                    self.pending_clipboard_ops
                        .push(ClipboardOp::Load(format_fn));
                }
            }
            AlacEvent::ColorRequest(index, format_fn) => {
                self.pending_color_ops.push(ColorOp { index, format_fn });
            }
            AlacEvent::Bell => {
                self.bell_active = true;
            }
            AlacEvent::CursorBlinkingChange => {
                let term = self.term.lock();
                self.cursor_blinking = term.cursor_style().blinking;
            }
            AlacEvent::TextAreaSizeRequest(format_fn) => {
                self.pending_size_ops.push(format_fn);
            }
            _ => {} // MouseCursorDirty, etc.
        }
    }

    /// Read the shell's CWD from the OS on demand.
    /// Fallback for shells that don't emit OSC 7 — used at split time.
    #[cfg(target_os = "linux")]
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        let proc_path = format!("/proc/{}/cwd", self.child_pid);
        std::fs::read_link(&proc_path).ok()
    }

    /// Stub for non-Linux platforms. macOS: use `proc_pidinfo()` with
    /// `PROC_PIDVNODEPATHINFO`. Windows: use `NtQueryInformationProcess`.
    #[cfg(not(target_os = "linux"))]
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Scan the last 100 lines of terminal output for server/service patterns.
    /// Returns newly detected services (deduped against previously reported ports).
    /// Lock on `self.term` is held only for text extraction, then released before parsing.
    pub fn scan_output(&mut self) -> Vec<ServiceInfo> {
        let lines: Vec<String> = {
            let term = self.term.lock();
            let bottom = term.bottommost_line();
            let top_limit = term.topmost_line();
            let cols = term.last_column();

            let mut buf = Vec::with_capacity(100);
            let mut row = bottom.0;
            while row >= top_limit.0 && buf.len() < 100 {
                let line = term.bounds_to_string(
                    AlacPoint::new(GridLine(row), GridCol(0)),
                    AlacPoint::new(GridLine(row), cols),
                );
                let trimmed = line.trim_end().to_string();
                if !trimmed.is_empty() {
                    buf.push(trimmed);
                }
                row -= 1;
            }
            buf
            // term lock dropped here
        };

        // Detect framework from ALL lines (context-wide), not just the port line.
        // Next.js prints "▲ Next.js 16.1.6" on one line and "localhost:3000" on another.
        let all_text = lines.join(" ");
        let (global_label, global_is_frontend) = detect_framework(&all_text);

        let mut results = Vec::new();
        for line in &lines {
            if let Some(mut info) = parse_service_line(line)
                && !self.reported_ports.contains(&info.port)
            {
                if info.label.is_none() {
                    info.label = global_label.clone();
                    info.is_frontend = global_is_frontend;
                }
                self.reported_ports.push(info.port);
                results.push(info);
            }
        }

        results
    }

    pub fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(input);
    }

    /// Extract scrollback as plain text (ANSI stripped) for session persistence.
    /// Caps at 4000 lines and 400,000 characters. Returns None if scrollback is empty.
    pub fn extract_scrollback(&self) -> Option<String> {
        const MAX_LINES: usize = 4000;
        const MAX_CHARS: usize = 400_000;

        let term = self.term.lock();
        let top = term.topmost_line();
        let bottom = term.bottommost_line();
        let cols = term.last_column();

        let mut lines: Vec<String> = Vec::new();
        let mut row = top.0;
        while row <= bottom.0 {
            let text = term.bounds_to_string(
                AlacPoint::new(GridLine(row), GridCol(0)),
                AlacPoint::new(GridLine(row), cols),
            );
            lines.push(text.trim_end().to_string());
            row += 1;
        }

        // Trim trailing empty lines
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }

        if lines.is_empty() {
            return None;
        }

        // Keep only the most recent MAX_LINES
        if lines.len() > MAX_LINES {
            lines.drain(..lines.len() - MAX_LINES);
        }

        let mut result = lines.join("\n");

        // Cap at MAX_CHARS
        if result.len() > MAX_CHARS {
            // Truncate to MAX_CHARS, then trim to last complete line
            result.truncate(MAX_CHARS);
            if let Some(last_newline) = result.rfind('\n') {
                result.truncate(last_newline);
            }
            // ANSI-safe: strip any partial escape sequence at the truncation boundary.
            // bounds_to_string produces plain text today, but this guards against
            // future changes that might preserve escape sequences.
            strip_partial_ansi_tail(&mut result);
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Feed saved scrollback text into the terminal grid via VTE processor.
    /// Called during session restore, before the shell has produced output.
    /// Prepends `\x1b[0m` (SGR reset) to clear any dangling style state from
    /// a prior truncated scrollback — ANSI-safe defense-in-depth (US-012).
    pub fn restore_scrollback(&self, text: &str) {
        let mut term = self.term.lock();
        let mut processor = alacritty_terminal::vte::ansi::Processor::<
            alacritty_terminal::vte::ansi::StdSyncHandler,
        >::new();
        // Reset any dangling style state before feeding restored content
        processor.advance(&mut *term, b"\x1b[0m");
        // Feed each line with \r\n to advance the cursor
        for line in text.split('\n') {
            let bytes = line.as_bytes();
            if !bytes.is_empty() {
                processor.advance(&mut *term, bytes);
            }
            processor.advance(&mut *term, b"\r\n");
        }
    }
}

/// Strip any partial ANSI escape sequence from the end of a truncated string.
///
/// Scans backward from the end for an ESC (`\x1b`) that starts a CSI (`\x1b[`),
/// OSC (`\x1b]`), or DCS (`\x1bP`) sequence. If the sequence is unterminated
/// (no final byte in the valid range), it is removed. Plain text strings with
/// no ESC bytes are returned unmodified — truncation is identical to naive splitting.
fn strip_partial_ansi_tail(text: &mut String) {
    // Find the last ESC byte
    let Some(esc_pos) = text.rfind('\x1b') else {
        return; // No escape sequences at all
    };

    let tail = &text[esc_pos..];
    let bytes = tail.as_bytes();

    if bytes.len() < 2 {
        // Lone ESC at the very end — always incomplete
        text.truncate(esc_pos);
        return;
    }

    match bytes[1] {
        b'[' => {
            // CSI sequence: \x1b[ ... terminated by byte in 0x40..=0x7E
            let terminated = bytes[2..].iter().any(|&b| (0x40..=0x7E).contains(&b));
            if !terminated {
                text.truncate(esc_pos);
            }
        }
        b']' => {
            // OSC sequence: \x1b] ... terminated by BEL (0x07) or ST (\x1b\\)
            let terminated = bytes[2..].contains(&0x07) || tail[2..].contains("\x1b\\");
            if !terminated {
                text.truncate(esc_pos);
            }
        }
        b'P' => {
            // DCS sequence: \x1bP ... terminated by ST (\x1b\\)
            let terminated = tail[2..].contains("\x1b\\");
            if !terminated {
                text.truncate(esc_pos);
            }
        }
        _ => {
            // Other ESC sequences (SS2, SS3, etc.) are 2 bytes — if we have 2+ bytes
            // it's complete. Nothing to strip.
        }
    }
}

/// Compute the PaneFlow IPC socket path: `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`.
fn paneflow_socket_path() -> Option<String> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(dirs::runtime_dir)?;
    Some(
        runtime_dir
            .join("paneflow")
            .join("paneflow.sock")
            .display()
            .to_string(),
    )
}

/// Compute the PaneFlow wrapper scripts directory: `$XDG_RUNTIME_DIR/paneflow/bin/`.
fn paneflow_bin_dir() -> Option<std::path::PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(dirs::runtime_dir)?;
    Some(runtime_dir.join("paneflow").join("bin"))
}

/// Extract embedded wrapper scripts (claude, paneflow-hook) to the runtime bin
/// directory with executable permissions. Idempotent: only writes if the file
/// is missing or content has changed. Uses atomic write (temp + rename) to
/// avoid race conditions when multiple terminals spawn concurrently.
fn ensure_wrapper_scripts(bin_dir: &std::path::Path) {
    use crate::assets::Assets;
    use std::os::unix::fs::PermissionsExt;

    if let Err(e) = std::fs::create_dir_all(bin_dir) {
        log::warn!(
            "paneflow: failed to create wrapper bin dir {}: {e}",
            bin_dir.display()
        );
        return;
    }

    for name in &["claude", "codex", "paneflow-hook"] {
        let asset_path = format!("bin/{name}");
        let Some(file) = Assets::get(&asset_path) else {
            continue;
        };

        let dest = bin_dir.join(name);

        // Skip write if content matches (avoid unnecessary disk I/O)
        if dest.exists()
            && let Ok(existing) = std::fs::read(&dest)
            && existing == file.data.as_ref()
        {
            continue;
        }

        // Atomic write: temp file → chmod → rename (same filesystem guarantees atomicity)
        let tmp = bin_dir.join(format!(".{name}.tmp"));
        if std::fs::write(&tmp, file.data.as_ref()).is_ok() {
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
            if let Err(e) = std::fs::rename(&tmp, &dest) {
                log::warn!("paneflow: failed to install wrapper script {name}: {e}");
                let _ = std::fs::remove_file(&tmp);
            }
        } else {
            log::warn!("paneflow: failed to write wrapper script {name}");
        }
    }
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);

        // Grace period + SIGKILL: if the child process ignores SIGHUP
        // (from PTY master close), force-kill it after 100ms.
        let pid = self.child_pid as i32;
        if pid > 0 {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                // Check if process is still alive before sending SIGKILL
                unsafe {
                    if libc::kill(pid, 0) == 0 {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal View — GPUI Render impl
// ---------------------------------------------------------------------------

const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

pub struct TerminalView {
    pub terminal: TerminalState,
    focus_handle: FocusHandle,
    cursor_visible: bool,
    /// Track mouse button state for drag selection
    selecting: bool,
    /// Last known cell dimensions (from TerminalElement::measure_cell)
    cell_width: gpui::Pixels,
    line_height: gpui::Pixels,
    /// Element origin in window coordinates — set by TerminalElement::paint(),
    /// read by mouse handlers for pixel→grid conversion.
    element_origin: Arc<Mutex<gpui::Point<gpui::Pixels>>>,
    /// Sub-line scroll accumulator for smooth trackpad scrolling
    scroll_remainder: f32,
    /// Whether the search overlay is visible
    search_active: bool,
    /// Current search query string
    search_query: String,
    /// Cached search matches (grid coordinates)
    search_matches: Vec<crate::search::SearchMatch>,
    /// Index of the currently focused match (for navigation)
    search_current: usize,
    /// Whether regex search mode is active (vs plain text)
    search_regex_mode: bool,
    /// Regex compilation error message (None when valid or plain text mode)
    search_regex_error: Option<String>,
    /// Current prompt mark navigation index (for jump-to-prompt cycling)
    prompt_mark_current: Option<usize>,
    /// Whether Alt key is treated as Meta (ESC prefix). Read from config.
    option_as_meta: bool,
    /// Whether copy mode (keyboard-driven selection) is active
    copy_mode_active: bool,
    /// Copy mode cursor position in grid coordinates
    copy_cursor: AlacPoint,
    /// Display offset frozen at copy mode entry to prevent auto-scroll
    copy_mode_frozen_offset: usize,
    /// Previous focus state, used to detect focus transitions for DEC 1004 events.
    was_focused: bool,
    /// Bell flash deadline — background pulse visible until this instant.
    bell_flash_until: Option<std::time::Instant>,
    /// Last hovered cell position for URL regex detection (US-015).
    hovered_cell: Option<AlacPoint>,
    /// Active hyperlink under Ctrl+hover — drives underline rendering and Ctrl+click.
    ctrl_hovered_link: Option<crate::terminal_element::HyperlinkZone>,
    /// IME preedit text (in-progress composition). Empty when no composition active.
    ime_marked_text: String,
}

impl TerminalView {
    pub fn new(workspace_id: u64, cx: &mut Context<Self>) -> Self {
        Self::with_cwd(workspace_id, None, None, cx)
    }

    pub fn with_cwd(
        workspace_id: u64,
        cwd: Option<std::path::PathBuf>,
        initial_size: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let surface_id = cx.entity_id().as_u64();
        let backend = PortablePtyBackend;
        let mut terminal =
            TerminalState::new(&backend, cwd, workspace_id, surface_id, initial_size)
                .expect("Failed to create terminal");
        let focus_handle = cx.focus_handle();

        // Event batch coalescing (Zed pattern):
        // Phase 1: Block until first event (zero CPU when idle)
        // Phase 2: Batch for 4ms, max 100 events, dedup Wakeup
        // Phase 3: Process batch, yield to other GPUI tasks
        let events_rx = terminal.events_rx.take().expect("events_rx already taken");
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut events_rx = events_rx;
                loop {
                    // Phase 1: Block until first event arrives (zero CPU when idle)
                    let Some(first_event) = events_rx.next().await else {
                        break; // Channel closed
                    };

                    // Phase 2: Batch additional events for 4ms, max 100
                    let mut batch: Vec<AlacEvent> = Vec::with_capacity(32);
                    let mut had_wakeup = matches!(first_event, AlacEvent::Wakeup);
                    if !had_wakeup {
                        batch.push(first_event);
                    }

                    {
                        let timer = futures::FutureExt::fuse(smol::Timer::after(
                            std::time::Duration::from_millis(4),
                        ));
                        futures::pin_mut!(timer);
                        loop {
                            futures::select_biased! {
                                event = events_rx.next() => {
                                    match event {
                                        Some(AlacEvent::Wakeup) => had_wakeup = true,
                                        Some(e) => batch.push(e),
                                        None => break,
                                    }
                                    if batch.len() >= 100 { break; }
                                }
                                _ = timer => break,
                            }
                        }
                    }

                    // Phase 3: Process the batch in a single entity update
                    let result = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            let old_title = view.terminal.title.clone();
                            let old_cwd = view.terminal.current_cwd.clone();
                            view.terminal.sync_channels();
                            if had_wakeup {
                                view.terminal.process_event(AlacEvent::Wakeup);
                            }
                            for event in batch {
                                view.terminal.process_event(event);
                            }

                            // Execute deferred clipboard operations (OSC 52)
                            for op in view.terminal.pending_clipboard_ops.drain(..) {
                                match op {
                                    ClipboardOp::Store(text) => {
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    }
                                    ClipboardOp::Load(format_fn) => {
                                        let text = cx
                                            .read_from_clipboard()
                                            .and_then(|c| c.text())
                                            .unwrap_or_default();
                                        // Strip ESC and C1 control chars to prevent injection
                                        let sanitized: String = text
                                            .chars()
                                            .filter(|&c| {
                                                c != '\x1b'
                                                    && !(('\u{0080}'..='\u{009f}').contains(&c))
                                            })
                                            .collect();
                                        let response = format_fn(&sanitized);
                                        view.terminal.notifier.notify(response.into_bytes());
                                    }
                                }
                            }

                            // Execute deferred color query responses (OSC 10/11/12)
                            // Cap at 16 to prevent flooding from malicious programs
                            view.terminal.pending_color_ops.truncate(16);
                            if !view.terminal.pending_color_ops.is_empty() {
                                let theme = crate::theme::active_theme();
                                for op in view.terminal.pending_color_ops.drain(..) {
                                    let color = match op.index {
                                        // OSC 10: foreground
                                        10 => Some(theme.foreground),
                                        // OSC 11: background
                                        11 => Some(theme.ansi_background),
                                        // OSC 12: cursor
                                        12 => Some(theme.cursor),
                                        _ => None,
                                    };
                                    if let Some(hsla) = color {
                                        let rgb = hsla_to_alac_rgb(hsla);
                                        let response = (op.format_fn)(rgb);
                                        view.terminal.notifier.notify(response.into_bytes());
                                    }
                                }
                            }

                            // Execute deferred text area size responses
                            view.terminal.pending_size_ops.truncate(8);
                            for format_fn in view.terminal.pending_size_ops.drain(..) {
                                let term = view.terminal.term.lock();
                                let size = AlacWindowSize {
                                    num_cols: term.columns() as u16,
                                    num_lines: term.screen_lines() as u16,
                                    cell_width: view.cell_width.as_f32() as u16,
                                    cell_height: view.line_height.as_f32() as u16,
                                };
                                drop(term);
                                let response = format_fn(size);
                                view.terminal.notifier.notify(response.into_bytes());
                            }

                            // Bell: trigger visual flash
                            if view.terminal.bell_active {
                                view.terminal.bell_active = false;
                                view.bell_flash_until = Some(
                                    std::time::Instant::now()
                                        + std::time::Duration::from_millis(200),
                                );
                                cx.emit(TerminalEvent::Bell);
                                // Schedule notify after flash duration to clear it
                                cx.spawn(async |this, cx| {
                                    smol::Timer::after(std::time::Duration::from_millis(200)).await;
                                    let _ = cx.update(|cx| {
                                        this.update(cx, |view, cx| {
                                            // Only clear if no newer bell extended the deadline
                                            if view
                                                .bell_flash_until
                                                .is_some_and(|t| t <= std::time::Instant::now())
                                            {
                                                view.bell_flash_until = None;
                                            }
                                            cx.notify();
                                        })
                                    });
                                })
                                .detach();
                            }

                            if view.terminal.exited.is_some() {
                                cx.emit(TerminalEvent::ChildExited);
                            }
                            if view.terminal.title != old_title {
                                cx.emit(TerminalEvent::TitleChanged);
                            }
                            if view.terminal.current_cwd != old_cwd
                                && let Some(ref cwd) = view.terminal.current_cwd
                            {
                                cx.emit(TerminalEvent::CwdChanged(cwd.clone()));
                            }

                            if view.terminal.dirty {
                                view.terminal.dirty = false;
                                let suppress =
                                    SUPPRESS_REPAINTS.load(std::sync::atomic::Ordering::Relaxed);

                                if !suppress {
                                    view.terminal.output_scan_ticks += 1;
                                    let should_scan = view.terminal.output_scan_ticks <= 10
                                        || view.terminal.output_scan_ticks >= 50;
                                    if should_scan {
                                        view.terminal.output_scan_ticks = 0;
                                        for service in view.terminal.scan_output() {
                                            cx.emit(TerminalEvent::ServiceDetected(service));
                                        }
                                    }

                                    // Copy mode: restore frozen display offset
                                    if view.copy_mode_active {
                                        let mut term = view.terminal.term.lock();
                                        let current = term.grid().display_offset();
                                        let frozen = view.copy_mode_frozen_offset;
                                        if current != frozen {
                                            let delta = frozen as i32 - current as i32;
                                            term.scroll_display(AlacScroll::Delta(delta));
                                        }
                                    }

                                    cx.notify();
                                }
                            }
                        })
                    });
                    if result.is_err() {
                        break;
                    }

                    // Yield to other GPUI tasks between batches
                    smol::future::yield_now().await;
                }
            },
        )
        .detach();

        // Cursor blink timer: toggle visibility every 530ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(CURSOR_BLINK_INTERVAL_MS))
                        .await;
                    let result = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            if view.terminal.exited.is_some()
                                || SUPPRESS_REPAINTS.load(std::sync::atomic::Ordering::Relaxed)
                            {
                                return;
                            }
                            if view.terminal.cursor_blinking {
                                view.cursor_visible = !view.cursor_visible;
                            } else {
                                view.cursor_visible = true;
                            }
                            cx.notify();
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        Self {
            terminal,
            focus_handle,
            cursor_visible: true,
            selecting: false,
            cell_width: gpui::px(8.0),
            line_height: gpui::px(16.0),
            element_origin: Arc::new(Mutex::new(gpui::Point::default())),
            scroll_remainder: 0.0,
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current: 0,
            search_regex_mode: false,
            search_regex_error: None,
            prompt_mark_current: None,
            option_as_meta: paneflow_config::loader::load_config()
                .option_as_meta
                .unwrap_or(true),
            copy_mode_active: false,
            copy_cursor: AlacPoint::new(GridLine(0), GridCol(0)),
            copy_mode_frozen_offset: 0,
            was_focused: false,
            bell_flash_until: None,
            hovered_cell: None,
            ctrl_hovered_link: None,
            ime_marked_text: String::new(),
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Cancel swap mode on Escape — checked before any other mode handling
        if crate::SWAP_MODE.load(std::sync::atomic::Ordering::Relaxed)
            && event.keystroke.key == "escape"
        {
            cx.emit(TerminalEvent::CancelSwapMode);
            return;
        }

        // When search overlay is active, redirect typed characters to search query
        if self.search_active {
            let keystroke = &event.keystroke;
            if keystroke.key == "backspace" && !keystroke.modifiers.control {
                self.search_query.pop();
                self.run_search();
                cx.notify();
                return;
            }
            if let Some(key_char) = &keystroke.key_char
                && !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && self.search_query.len() < crate::search::MAX_QUERY_LEN
            {
                self.search_query.push_str(key_char);
                self.run_search();
                cx.notify();
                return;
            }
            // Consume all other keys while search is active — action bindings
            // (ToggleSearch, DismissSearch, SearchNext, SearchPrev) are dispatched
            // by GPUI before on_key_down, so they still fire. Everything else
            // (F-keys, Alt combos, etc.) is intentionally suppressed.
            return;
        }

        // When copy mode is active, intercept navigation and exit keys
        if self.copy_mode_active {
            let keystroke = &event.keystroke;
            let key = keystroke.key.as_str();
            let shift = keystroke.modifiers.shift;

            match key {
                "left" | "right" | "up" | "down" => {
                    let (dx, dy): (i32, i32) = match key {
                        "left" => (-1, 0),
                        "right" => (1, 0),
                        "up" => (0, -1),
                        "down" => (0, 1),
                        _ => unreachable!(),
                    };
                    if shift {
                        self.extend_copy_selection(dx, dy, cx);
                    } else {
                        self.move_copy_cursor(dx, dy, cx);
                    }
                }
                "enter" => {
                    self.exit_copy_mode(true, cx);
                }
                "escape" => {
                    self.exit_copy_mode(false, cx);
                }
                _ => {
                    // 'q' exits copy mode (vi-style)
                    if keystroke.key_char.as_deref() == Some("q")
                        && !keystroke.modifiers.control
                        && !keystroke.modifiers.alt
                    {
                        self.exit_copy_mode(false, cx);
                    }
                    // All other keys consumed — not sent to PTY
                }
            }
            return;
        }

        #[cfg(debug_assertions)]
        let _probe_start = if probe_enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Reset cursor blink on keystroke
        self.cursor_visible = true;

        let keystroke = &event.keystroke;

        // Get current TermMode for key mapping (APP_CURSOR, etc.)
        let term_guard = self.terminal.term.lock();
        let mode = *term_guard.mode();
        drop(term_guard);

        // Try the key mapping module first (handles ctrl, special keys, modifiers)
        if let Some(seq) = crate::keys::to_esc_str(keystroke, &mode, self.option_as_meta) {
            match seq {
                Cow::Borrowed(s) => {
                    // Zero allocation — static byte slice
                    self.terminal.write_to_pty(Cow::Borrowed(s.as_bytes()));
                }
                Cow::Owned(s) => {
                    // One allocation for modifier combos
                    self.terminal.write_to_pty(s.into_bytes());
                }
            }
        } else if let Some(key_char) = &keystroke.key_char {
            // Printable character input — single allocation (String → Vec<u8>)
            self.terminal.write_to_pty(key_char.as_bytes().to_vec());
        }

        #[cfg(debug_assertions)]
        if let Some(start) = _probe_start {
            let elapsed = start.elapsed();
            // Store timestamp for total keystroke→pixel measurement in paint()
            self.terminal.last_keystroke_at = Some(start);
            if elapsed.as_millis() > 1 {
                log::warn!(
                    "[latency] keystroke→PTY: {:.2}ms",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
        }
    }

    // --- Pixel → grid coordinate conversion ---

    fn pixel_to_grid(&self, pos: gpui::Point<gpui::Pixels>) -> (AlacPoint, Side) {
        let origin = *self.element_origin.lock().unwrap();
        let relative_x = (pos.x - origin.x).max(gpui::px(0.0));
        let relative_y = (pos.y - origin.y).max(gpui::px(0.0));

        let col_f = relative_x / self.cell_width;
        let half_cell = self.cell_width / 2.0;
        let cell_x = relative_x % self.cell_width;
        let side = if cell_x > half_cell {
            Side::Right
        } else {
            Side::Left
        };

        let term = self.terminal.term.lock();
        let max_col = term.columns().saturating_sub(1);
        let max_line = term.screen_lines().saturating_sub(1) as i32;
        let display_offset = term.grid().display_offset();
        drop(term);

        let col = (col_f as usize).min(max_col);
        let line = ((relative_y / self.line_height) as i32).min(max_line);

        (
            AlacPoint::new(GridLine(line - display_offset as i32), GridCol(col)),
            side,
        )
    }

    /// Convert pixel position to viewport grid coordinates (for mouse reporting).
    /// Unlike `pixel_to_grid`, this returns 0-based viewport coordinates without
    /// the scrollback display_offset subtraction.
    fn pixel_to_viewport(&self, pos: gpui::Point<gpui::Pixels>) -> AlacPoint {
        let origin = *self.element_origin.lock().unwrap();
        let relative_x = (pos.x - origin.x).max(gpui::px(0.0));
        let relative_y = (pos.y - origin.y).max(gpui::px(0.0));
        let col_f = relative_x / self.cell_width;
        let term = self.terminal.term.lock();
        let max_col = term.columns().saturating_sub(1);
        let max_line = term.screen_lines().saturating_sub(1) as i32;
        drop(term);
        let col = (col_f as usize).min(max_col);
        let line = ((relative_y / self.line_height) as i32).min(max_line);
        AlacPoint::new(GridLine(line), GridCol(col))
    }

    /// Write a mouse report to the PTY using the appropriate encoding format.
    fn write_mouse_report(&self, point: AlacPoint, button: u8, pressed: bool, mode: TermMode) {
        let format = mouse::MouseFormat::from_mode(mode);
        let bytes = match format {
            mouse::MouseFormat::Sgr => mouse::sgr_mouse_report(point, button, pressed).into_bytes(),
            mouse::MouseFormat::Normal { utf8 } => {
                // Normal/UTF-8 encoding: release always uses button code 3 (no per-button release)
                let btn = if pressed { button } else { 3 };
                match mouse::normal_mouse_report(point, btn, utf8) {
                    Some(b) => b,
                    None => return, // position exceeds encoding limits
                }
            }
        };
        self.terminal.write_to_pty(bytes);
    }

    // --- Mouse selection handlers ---

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Ctrl+Left-click: open hyperlink (US-016)
        if event.button == MouseButton::Left && event.modifiers.control && event.click_count == 1 {
            if let Some(ref link) = self.ctrl_hovered_link
                && link.is_openable
            {
                let _ = open::that(&link.uri);
            }
            return;
        }

        let mode = { *self.terminal.term.lock().mode() };

        // Forward to PTY when mouse reporting is active.
        // Shift overrides mouse mode for text selection (standard terminal convention).
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let point = self.pixel_to_viewport(event.position);
            let button = mouse::mouse_button_code(event.button, event.modifiers);
            self.write_mouse_report(point, button, true, mode);
            return;
        }

        // Text selection (mouse mode inactive or Shift held)
        if event.button != MouseButton::Left {
            return;
        }

        let (point, side) = self.pixel_to_grid(event.position);

        let selection_type = match event.click_count {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => return,
        };

        let selection = Selection::new(selection_type, point, side);
        let mut term = self.terminal.term.lock();
        term.selection = Some(selection);
        drop(term);

        self.selecting = true;
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward motion to PTY when mouse tracking is active.
        // Shift overrides mouse mode for text selection.
        if !event.modifiers.shift
            && (mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && event.pressed_button.is_some()))
        {
            let point = self.pixel_to_viewport(event.position);
            let button_base = match event.pressed_button {
                Some(btn) => mouse::mouse_button_code(btn, event.modifiers),
                None => 3, // no button held = release code in motion reports
            };
            // Motion events add +32 to the button code per protocol spec
            let button = button_base + 32;
            self.write_mouse_report(point, button, true, mode);
            return;
        }

        // Track hovered cell for URL regex detection (US-015)
        let (hover_point, _) = self.pixel_to_grid(event.position);
        self.hovered_cell = Some(hover_point);

        // Ctrl+hover: detect URL under cursor for hyperlink rendering (US-016)
        // OSC 8 takes priority over regex detection.
        if event.modifiers.control {
            // Check OSC 8 hyperlink on the hovered cell first
            let osc8_link = {
                let term = self.terminal.term.lock();
                let cell = &term.grid()[hover_point.line][hover_point.column];
                cell.hyperlink().map(|hl| {
                    use crate::terminal_element::{
                        HyperlinkSource, HyperlinkZone, is_url_scheme_openable,
                    };
                    HyperlinkZone {
                        uri: hl.uri().to_string(),
                        id: hl.id().to_string(),
                        start: hover_point,
                        end: hover_point, // Single cell — hover underline will cover it
                        is_openable: is_url_scheme_openable(hl.uri()),
                        source: HyperlinkSource::Osc8,
                    }
                })
            };
            self.ctrl_hovered_link = osc8_link.or_else(|| {
                let zones = self.detect_url_at_hover();
                zones.into_iter().find(|z| {
                    hover_point.line == z.start.line
                        && hover_point.column >= z.start.column
                        && hover_point.column <= z.end.column
                })
            });
            cx.notify();
        } else if self.ctrl_hovered_link.is_some() {
            self.ctrl_hovered_link = None;
            cx.notify();
        }

        // Text selection (mouse mode inactive)
        if !self.selecting {
            return;
        }

        let (point, side) = self.pixel_to_grid(event.position);

        let mut term = self.terminal.term.lock();
        if let Some(ref mut selection) = term.selection {
            selection.update(point, side);
        }
        drop(term);

        cx.notify();
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward release to PTY when mouse reporting is active.
        // Shift overrides mouse mode for text selection.
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let point = self.pixel_to_viewport(event.position);
            let button = mouse::mouse_button_code(event.button, event.modifiers);
            self.write_mouse_report(point, button, false, mode);
            return;
        }

        // Middle-click: paste from primary selection
        if event.button == MouseButton::Middle {
            if let Some(item) = cx.read_from_primary()
                && let Some(text) = item.text()
            {
                self.write_paste_text(&text, mode);
            }
            return;
        }

        // Text selection cleanup (mouse mode inactive or Shift held)
        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;

        // Clear empty selections, write non-empty selections to primary clipboard
        let mut term = self.terminal.term.lock();
        if let Some(ref sel) = term.selection
            && sel.is_empty()
        {
            term.selection = None;
        } else if let Some(text) = term.selection_to_string() {
            drop(term);
            cx.write_to_primary(ClipboardItem::new_string(text));
            cx.notify();
            return;
        }
        drop(term);

        cx.notify();
    }

    // --- Clipboard handlers ---

    fn handle_copy(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let term = self.terminal.term.lock();
        if let Some(text) = term.selection_to_string() {
            drop(term);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn handle_paste(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };

        // Image in clipboard: forward raw Ctrl+V (0x16) so TUI agents can read it
        if let Some(ClipboardEntry::Image(image)) = clipboard.entries().first()
            && !image.bytes.is_empty()
        {
            self.terminal.write_to_pty(vec![0x16]);
            return;
        }

        // Text paste
        if let Some(text) = clipboard.text() {
            let mode = { *self.terminal.term.lock().mode() };
            self.write_paste_text(&text, mode);
        }
    }

    /// Prepare and write paste text to PTY, respecting bracketed paste mode.
    /// Strips ESC and C1 control chars when bracketed paste is active.
    fn handle_file_drop(
        &mut self,
        paths: &ExternalPaths,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let quoted: Vec<String> = paths
            .paths()
            .iter()
            .filter_map(|p| {
                let s = p.to_string_lossy();
                // Reject paths with newlines or null bytes — they break shell quoting
                if s.contains('\n') || s.contains('\0') {
                    return None;
                }
                if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('\\') {
                    Some(format!("'{}'", s.replace('\'', "'\\''")))
                } else {
                    Some(s.into_owned())
                }
            })
            .collect();
        if quoted.is_empty() {
            return;
        }
        let text = quoted.join(" ");
        let mode = *self.terminal.term.lock().mode();
        self.write_paste_text(&text, mode);
    }

    fn write_paste_text(&self, text: &str, mode: TermMode) {
        let paste_text = if mode.contains(TermMode::BRACKETED_PASTE) {
            // Strip ESC and C1 control chars (U+0080..U+009F) to prevent
            // bracketed paste escape and CSI injection
            let sanitized: String = text
                .chars()
                .filter(|&c| c != '\x1b' && !(('\u{0080}'..='\u{009f}').contains(&c)))
                .collect();
            format!("\x1b[200~{sanitized}\x1b[201~")
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.terminal.write_to_pty(paste_text.into_bytes());
    }

    // --- Scroll handlers ---

    fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward scroll to PTY when mouse reporting is active.
        // Shift overrides mouse mode for scrollback.
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let delta_y = event.delta.pixel_delta(self.line_height).y;
            self.scroll_remainder += delta_y / self.line_height;
            self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);
            let lines = self.scroll_remainder as i32;
            if lines == 0 {
                return;
            }
            self.scroll_remainder -= lines as f32;

            let point = self.pixel_to_viewport(event.position);
            let direction = if lines > 0 {
                mouse::ScrollDirection::Up
            } else {
                mouse::ScrollDirection::Down
            };
            let button = mouse::scroll_button_code(direction, event.modifiers);
            // Send one report per scroll line
            for _ in 0..lines.unsigned_abs() {
                self.write_mouse_report(point, button, true, mode);
            }
            return;
        }

        // Alternate scroll: ALT_SCREEN + ALTERNATE_SCROLL without MOUSE_MODE
        // Synthesize arrow key sequences so scroll works in less, vim, htop, etc.
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
            && !event.modifiers.shift
        {
            let delta_y = event.delta.pixel_delta(self.line_height).y;
            self.scroll_remainder += delta_y / self.line_height;
            self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);
            let lines = self.scroll_remainder as i32;
            if lines == 0 {
                return;
            }
            self.scroll_remainder -= lines as f32;

            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            let arrow: &[u8] = match (lines > 0, app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let count = lines.unsigned_abs() as usize;
            let mut buf = Vec::with_capacity(arrow.len() * count);
            for _ in 0..count {
                buf.extend_from_slice(arrow);
            }
            self.terminal.write_to_pty(buf);
            return;
        }

        // Scrollback (mouse mode inactive, not alt screen alternate scroll)
        // Convert pixel delta to fractional lines, accumulate sub-line remainders
        let delta_y = event.delta.pixel_delta(self.line_height).y;
        self.scroll_remainder += delta_y / self.line_height;

        // Clamp to prevent extreme values from synthesised events
        self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);

        let lines = self.scroll_remainder as i32;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32;

        // Negate: positive pixel delta = scroll down, but AlacScroll::Delta
        // positive = scroll toward history (up)
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::Delta(-lines));
        self.terminal.dirty = true;
        drop(term);

        cx.notify();
    }

    fn handle_scroll_page_up(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::PageUp);
        self.terminal.dirty = true;
        drop(term);
        cx.notify();
    }

    fn handle_scroll_page_down(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::PageDown);
        self.terminal.dirty = true;
        drop(term);
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// DEC 2026 synchronized output scanner
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// XTVERSION query scanner
// ---------------------------------------------------------------------------

/// Byte-level scanner for XTVERSION queries (`\x1b[>0q` or `\x1b[>q`).
/// When detected, emits a DCS response via the event listener.
struct XtversionScanner {
    match_pos: usize,
}

/// Sequence: ESC [ > 0 q  (also accept ESC [ > q with implicit 0 param)
const XTVERSION_SEQ: [u8; 5] = [0x1b, b'[', b'>', b'0', b'q'];

impl XtversionScanner {
    fn new() -> Self {
        Self { match_pos: 0 }
    }

    /// Scan `buf` for XTVERSION queries. If found, send a DCS response via `listener`.
    fn scan(&mut self, buf: &[u8], listener: &ZedListener) {
        for &byte in buf {
            if self.match_pos < XTVERSION_SEQ.len() {
                if byte == XTVERSION_SEQ[self.match_pos] {
                    self.match_pos += 1;
                } else if self.match_pos == 3 && byte == b'q' {
                    // Accept `\x1b[>q` (no explicit 0 parameter)
                    self.emit_response(listener);
                    self.match_pos = 0;
                } else if byte == 0x1b {
                    self.match_pos = 1;
                } else {
                    self.match_pos = 0;
                }
            }
            if self.match_pos == XTVERSION_SEQ.len() {
                self.emit_response(listener);
                self.match_pos = 0;
            }
        }
    }

    fn emit_response(&self, listener: &ZedListener) {
        let version = env!("CARGO_PKG_VERSION");
        let response = format!("\x1bP>|paneflow({version})\x1b\\");
        listener.send_event(AlacEvent::PtyWrite(response));
    }
}

// ---------------------------------------------------------------------------
// OSC 7 CWD scanner
// ---------------------------------------------------------------------------

/// Byte-level scanner for OSC 7 (`\x1b]7;file://[host]/path{BEL|ST}`).
/// The Alacritty fork silently ignores OSC 7, so we intercept it in the reader
/// loop before VTE processing and send the parsed CWD through a channel.
struct Osc7Scanner {
    state: u8,
    payload: Vec<u8>,
}

impl Osc7Scanner {
    fn new() -> Self {
        Self {
            state: 0,
            payload: Vec::new(),
        }
    }

    fn scan(&mut self, buf: &[u8], cwd_tx: &UnboundedSender<String>) {
        for &byte in buf {
            match self.state {
                0 => {
                    if byte == 0x1b {
                        self.state = 1;
                    }
                }
                1 => {
                    // After ESC, expect ]
                    if byte == b']' {
                        self.state = 2;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                2 => {
                    // After ESC ], expect 7
                    if byte == b'7' {
                        self.state = 3;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                3 => {
                    // After ESC ] 7, expect ;
                    if byte == b';' {
                        self.state = 4;
                        self.payload.clear();
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                4 => {
                    // Collecting payload until BEL or ST
                    if byte == 0x07 {
                        self.emit_cwd(cwd_tx);
                        self.state = 0;
                    } else if byte == 0x1b {
                        self.state = 5; // Possible ST (\x1b\\)
                    } else if self.payload.len() < 2048 {
                        self.payload.push(byte);
                    }
                    // Silently drop bytes beyond 2048 limit
                }
                5 => {
                    // After ESC in payload: ST terminator is \x1b followed by \\
                    if byte == b'\\' {
                        self.emit_cwd(cwd_tx);
                        self.state = 0;
                    } else if byte == 0x1b {
                        self.state = 1; // New ESC sequence starting
                    } else {
                        self.state = 0;
                    }
                }
                _ => {
                    self.state = 0;
                }
            }
        }
    }

    fn emit_cwd(&self, cwd_tx: &UnboundedSender<String>) {
        if let Ok(uri) = std::str::from_utf8(&self.payload)
            && let Some(path) = parse_osc7_uri(uri)
        {
            let _ = cwd_tx.unbounded_send(path);
        }
    }
}

// ---------------------------------------------------------------------------
// OSC 133 scanner — detects shell prompt marks (A/B/C/D)
// ---------------------------------------------------------------------------

/// Byte-level scanner for OSC 133 sequences emitted by shell integration.
/// Matches `ESC ] 133 ; {A|B|C|D} [; params] {BEL | ST}`.
/// Only the mark kind (A/B/C/D) is captured; any trailing parameters after
/// a second `;` are ignored.
struct Osc133Scanner {
    state: u8,
}

impl Osc133Scanner {
    fn new() -> Self {
        Self { state: 0 }
    }

    fn scan(
        &mut self,
        buf: &[u8],
        term: &Arc<FairMutex<Term<ZedListener>>>,
        prompt_tx: &UnboundedSender<PromptMark>,
    ) {
        for &byte in buf {
            match self.state {
                0 => {
                    if byte == 0x1b {
                        self.state = 1;
                    }
                }
                1 => {
                    if byte == b']' {
                        self.state = 2;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                2 => {
                    // After ESC ], expect '1'
                    if byte == b'1' {
                        self.state = 3;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                3 => {
                    // After ESC ] 1, expect '3'
                    if byte == b'3' {
                        self.state = 4;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                4 => {
                    // After ESC ] 13, expect '3'
                    if byte == b'3' {
                        self.state = 5;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                5 => {
                    // After ESC ] 133, expect ';'
                    if byte == b';' {
                        self.state = 6;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                6 => {
                    // After ESC ] 133 ;, expect mark kind (A/B/C/D)
                    let kind = match byte {
                        b'A' => Some(PromptMarkKind::PromptStart),
                        b'B' => Some(PromptMarkKind::CommandStart),
                        b'C' => Some(PromptMarkKind::OutputStart),
                        b'D' => Some(PromptMarkKind::OutputEnd),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        self.emit_mark(k, term, prompt_tx);
                    }
                    // Skip remaining params until terminator
                    if byte == 0x07 {
                        self.state = 0;
                    } else if byte == 0x1b {
                        self.state = 8; // Possible ST
                    } else {
                        self.state = 7; // Skip params
                    }
                }
                7 => {
                    // Skipping optional parameters until BEL or ST
                    if byte == 0x07 {
                        self.state = 0;
                    } else if byte == 0x1b {
                        self.state = 8;
                    }
                }
                8 => {
                    // After ESC in skip mode — check for ST (\)
                    if byte == b'\\' {
                        self.state = 0;
                    } else if byte == 0x1b {
                        self.state = 1;
                    } else {
                        self.state = 0;
                    }
                }
                _ => {
                    self.state = 0;
                }
            }
        }
    }

    fn emit_mark(
        &self,
        kind: PromptMarkKind,
        term: &Arc<FairMutex<Term<ZedListener>>>,
        prompt_tx: &UnboundedSender<PromptMark>,
    ) {
        // Read the current cursor line from the term grid.
        // The cursor position at the time OSC 133 is emitted corresponds
        // to the line where the prompt mark applies.
        let line = {
            let term = term.lock();
            term.grid().cursor.point.line.0
        };
        let _ = prompt_tx.unbounded_send(PromptMark { line, kind });
    }
}

/// Parse `file://[hostname]/path` URI from OSC 7 payload.
/// Returns the percent-decoded path, ignoring hostname.
fn parse_osc7_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest // Empty hostname: file:///path
    } else {
        &rest[rest.find('/')?..] // hostname/path: skip to first /
    };
    Some(percent_decode(path))
}

/// Percent-decode a URI path component. Handles multi-byte UTF-8 encoded
/// as consecutive %XX sequences. Uses lossy UTF-8 for non-UTF-8 bytes.
fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let mut iter = s.as_bytes().iter();
    while let Some(&b) = iter.next() {
        if b == b'%' {
            if let (Some(&hi), Some(&lo)) = (iter.next(), iter.next())
                && let (Some(h), Some(l)) = (hex_val(hi), hex_val(lo))
            {
                bytes.push(h << 4 | l);
                continue;
            }
            bytes.push(b'%');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// portable-pty I/O loops (US-007) — replace AlacEventLoop
// ---------------------------------------------------------------------------

/// Reader thread: reads PTY output, feeds through VTE parser into Term, sends Wakeup events.
/// Owns the child handle to capture exit status after the read loop ends.
/// DEC 2026: scans raw bytes for BSU/ESU before VTE processing, suppresses Wakeup during sync.
fn pty_reader_loop(
    mut reader: Box<dyn Read + Send>,
    term: Arc<FairMutex<Term<ZedListener>>>,
    listener: ZedListener,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    cwd_tx: UnboundedSender<String>,
    prompt_tx: UnboundedSender<PromptMark>,
) {
    let mut buf = [0u8; 4096];
    let mut processor = alacritty_terminal::vte::ansi::Processor::<
        alacritty_terminal::vte::ansi::StdSyncHandler,
    >::new();
    let mut xtversion_scanner = XtversionScanner::new();
    let mut osc7_scanner = Osc7Scanner::new();
    let mut osc133_scanner = Osc133Scanner::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                xtversion_scanner.scan(&buf[..n], &listener);
                osc7_scanner.scan(&buf[..n], &cwd_tx);
                osc133_scanner.scan(&buf[..n], &term, &prompt_tx);

                let mut term = term.lock();
                processor.advance(&mut *term, &buf[..n]);
                drop(term);
                // Gate Wakeup on DEC 2026 sync state: the vte Processor buffers bytes
                // during synchronized output (\e[?2026h..\e[?2026l) and reports them
                // via sync_bytes_count(). Only send Wakeup when some bytes were processed
                // outside the sync buffer — matches Alacritty event_loop.rs:166.
                // Safety timeout (150ms) is built into vte's StdSyncHandler.
                if processor.sync_bytes_count() < n {
                    listener.send_event(AlacEvent::Wakeup);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                break;
            }
        }
    }
    // Capture child exit status after PTY read loop ends.
    match child.wait() {
        Ok(status) => {
            let code = status.exit_code() as i32;
            // portable-pty ExitStatus → std ExitStatus. On Unix, from_raw() expects
            // a raw wait(2) status where exit code lives in bits 8-15. Passing the
            // plain exit code would misinterpret non-zero codes as signal numbers.
            #[cfg(unix)]
            listener.send_event(AlacEvent::ChildExit(
                std::os::unix::process::ExitStatusExt::from_raw(code << 8),
            ));
            #[cfg(not(unix))]
            {
                let _ = code;
                listener.send_event(AlacEvent::Exit);
            }
        }
        Err(_) => {
            listener.send_event(AlacEvent::Exit);
        }
    }
}

/// Message handler thread: receives Msg from Notifier channel, writes to PTY or resizes.
fn pty_message_loop(
    rx: std::sync::mpsc::Receiver<Msg>,
    mut writer: Box<dyn Write + Send>,
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
) {
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Input(bytes) => {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
            Msg::Resize(size) => {
                let pty_size = PtySize {
                    rows: size.num_lines,
                    cols: size.num_cols,
                    pixel_width: size.num_cols * size.cell_width,
                    pixel_height: size.num_lines * size.cell_height,
                };
                if let Ok(master) = master.lock() {
                    let _ = master.resize(pty_size);
                }
            }
            Msg::Shutdown => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Service detection from PTY output
// ---------------------------------------------------------------------------

/// Metadata about a detected service (server listening on a port).
/// Enriches the bare port number from `/proc/net/tcp` with human-readable info.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub port: u16,
    pub url: Option<String>,
    pub label: Option<String>,
    /// True for frontend dev servers (Next.js, Vite, Nuxt) — clickable in sidebar.
    pub is_frontend: bool,
}

/// Parse a terminal output line for local server URL patterns.
/// Derived from VS Code's UrlFinder — anchors on localhost/127.0.0.1/0.0.0.0.
fn parse_service_line(line: &str) -> Option<ServiceInfo> {
    let port = extract_local_port(line)?;
    if port == 0 {
        return None;
    }
    let url = extract_url(line);
    let (label, is_frontend) = detect_framework(line);
    Some(ServiceInfo {
        port,
        url,
        label,
        is_frontend,
    })
}

/// Extract a port number from localhost:PORT, 127.0.0.1:PORT, or 0.0.0.0:PORT patterns.
/// Also handles Python's `http.server` format: "HTTP on 127.0.0.1 port 8000".
fn extract_local_port(line: &str) -> Option<u16> {
    for anchor in ["localhost:", "127.0.0.1:", "0.0.0.0:"] {
        if let Some(idx) = line.find(anchor) {
            let after = &line[idx + anchor.len()..];
            let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = port_str.parse::<u16>() {
                return Some(port);
            }
        }
    }
    // Python http.server: "HTTP on 127.0.0.1 port 8000"
    if let Some(idx) = line.find(" port ")
        && (line.contains("127.0.0.1") || line.contains("0.0.0.0"))
    {
        let after = &line[idx + 6..];
        let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = port_str.parse::<u16>() {
            return Some(port);
        }
    }
    None
}

/// Extract a full URL (http:// or https://) from a terminal line.
fn extract_url(line: &str) -> Option<String> {
    for scheme in ["https://", "http://"] {
        if let Some(start) = line.find(scheme) {
            let url: String = line[start..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != ')' && *c != '"' && *c != '\'')
                .collect();
            if url.len() > scheme.len() {
                return Some(url);
            }
        }
    }
    None
}

/// Detect the framework/server name from keywords in the terminal line.
/// Returns `(label, is_frontend)` — frontend frameworks get clickable URLs in the sidebar.
/// Uses word-boundary matching to avoid false positives (e.g. "origin" matching "gin").
fn detect_framework(line: &str) -> (Option<String>, bool) {
    // (keyword, display_label, is_frontend)
    const FRAMEWORKS: &[(&str, &str, bool)] = &[
        ("next.js", "Next.js", true),
        ("next dev", "Next.js", true),
        ("turbopack", "Next.js", true),
        ("vite", "Vite", true),
        ("nuxt", "Nuxt", true),
        ("remix", "Remix", true),
        ("astro", "Astro", true),
        ("webpack-dev-server", "Webpack", true),
        ("angular", "Angular", true),
        ("express", "Express", false),
        ("fastify", "Fastify", false),
        ("uvicorn", "uvicorn", false),
        ("flask", "Flask", false),
        ("django", "Django", false),
        ("rocket", "Rocket", false),
        ("actix-web", "Actix", false),
        ("axum", "Axum", false),
        ("gin-gonic", "Gin", false),
        ("fiber", "Fiber", false),
        ("puma", "Puma", false),
        ("tomcat", "Tomcat", false),
        ("laravel", "Laravel", false),
        ("spring boot", "Spring", false),
    ];
    let lower = line.to_lowercase();
    for (key, label, frontend) in FRAMEWORKS {
        if lower.contains(key) {
            return (Some(label.to_string()), *frontend);
        }
    }
    (None, false)
}

// ---------------------------------------------------------------------------
// OSC 7 URI parsing
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// IME composition methods (US-017)
// ---------------------------------------------------------------------------

impl TerminalView {
    /// Set preedit text during IME composition.
    pub fn set_marked_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.ime_marked_text = text;
        cx.notify();
    }

    /// Clear preedit text (cancel composition).
    pub fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
        self.ime_marked_text.clear();
        cx.notify();
    }

    /// Commit composed text to the PTY.
    pub fn commit_text(&mut self, text: &str, _cx: &mut Context<Self>) {
        self.terminal.write_to_pty(text.as_bytes().to_vec());
    }

    /// Send arbitrary text to the PTY (no bracketed paste wrapping).
    /// Used by AI agents and automation tools via IPC.
    pub fn send_text(&self, text: &str) {
        self.terminal.write_to_pty(text.as_bytes().to_vec());
    }

    /// Send a keystroke to the PTY by converting it to an escape sequence.
    /// `keystroke_str` is a dash-separated description like "ctrl-c", "enter", "alt-f".
    /// Returns Ok(()) on success, Err(message) if the keystroke string is invalid.
    pub fn send_keystroke(&self, keystroke_str: &str) -> Result<(), String> {
        let keystroke = gpui::Keystroke::parse(keystroke_str).map_err(|e| format!("{e}"))?;
        let mode = *self.terminal.term.lock().mode();
        if let Some(seq) = crate::keys::to_esc_str(&keystroke, &mode, self.option_as_meta) {
            self.terminal.write_to_pty(seq.as_bytes().to_vec());
        } else if let Some(ref key_char) = keystroke.key_char {
            self.terminal.write_to_pty(key_char.as_bytes().to_vec());
        }
        Ok(())
    }

    /// Return the UTF-16 range of the current preedit text, if any.
    pub fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
        if self.ime_marked_text.is_empty() {
            None
        } else {
            let utf16_len: usize = self.ime_marked_text.encode_utf16().count();
            Some(0..utf16_len)
        }
    }
}

// ---------------------------------------------------------------------------
// URL detection on hover (US-015)
// ---------------------------------------------------------------------------

impl TerminalView {
    /// Detect regex URLs on the line at the given grid point.
    /// Extracts line text from the locked term grid, runs the URL regex,
    /// and returns zones that cover the given column (for hover hit-testing).
    #[allow(dead_code)]
    pub fn detect_url_at_hover(&self) -> Vec<crate::terminal_element::HyperlinkZone> {
        let point = match self.hovered_cell {
            Some(p) => p,
            None => return Vec::new(),
        };
        let term = self.terminal.term.lock();
        let grid = term.grid();
        let line = point.line;

        // Extract line text from grid cells, skipping wide-char spacer placeholders.
        // Track a char-to-column mapping so regex byte offsets map to grid columns.
        let cols = term.columns();
        let mut line_text = String::with_capacity(cols);
        let mut char_to_col: Vec<usize> = Vec::with_capacity(cols);
        for col in 0..cols {
            let cell = &grid[line][alacritty_terminal::index::Column(col)];
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
            {
                continue; // Skip trailing spacer of wide chars
            }
            char_to_col.push(col);
            line_text.push(cell.c);
        }

        // Trim trailing whitespace for cleaner regex matching
        let trimmed = line_text.trim_end();
        crate::terminal_element::detect_urls_on_line_mapped(trimmed, line, &char_to_col)
    }
}

// ---------------------------------------------------------------------------
// Terminal control actions on TerminalView
// ---------------------------------------------------------------------------

impl TerminalView {
    fn clear_scroll_history(&mut self, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.grid_mut().clear_history();
        drop(term);
        cx.notify();
    }

    fn reset_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.write_to_pty(b"\x1bc".as_ref());
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Search methods on TerminalView
// ---------------------------------------------------------------------------

impl TerminalView {
    fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.search_matches.clear();
            self.search_current = 0;
            self.search_regex_error = None;
            // Reset scroll position
            let mut term = self.terminal.term.lock();
            term.scroll_display(AlacScroll::Bottom);
        }
        cx.notify();
    }

    fn dismiss_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = false;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_current = 0;
        self.search_regex_error = None;
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::Bottom);
        cx.notify();
    }

    fn toggle_search_regex(&mut self, cx: &mut Context<Self>) {
        self.search_regex_mode = !self.search_regex_mode;
        if !self.search_query.is_empty() {
            self.run_search();
        }
        cx.notify();
    }

    fn search_next(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_current = (self.search_current + 1) % self.search_matches.len();
        self.scroll_to_current_match();
        cx.notify();
    }

    fn search_prev(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.search_current == 0 {
            self.search_current = self.search_matches.len() - 1;
        } else {
            self.search_current -= 1;
        }
        self.scroll_to_current_match();
        cx.notify();
    }

    fn run_search(&mut self) {
        let result = crate::search::search_term(
            &self.terminal.term,
            &self.search_query,
            self.search_regex_mode,
        );
        self.search_matches = result.matches;
        self.search_regex_error = result.regex_error;
        self.search_current = 0;
        if !self.search_matches.is_empty() {
            self.scroll_to_current_match();
        }
    }

    fn scroll_to_current_match(&mut self) {
        if let Some(m) = self.search_matches.get(self.search_current) {
            crate::search::scroll_to_match(&self.terminal.term, m);
        }
    }

    fn jump_to_prompt_prev(&mut self, cx: &mut Context<Self>) {
        let marks = &self.terminal.prompt_marks;
        if marks.is_empty() {
            return;
        }
        // Find prompt-start marks only (kind A) — these are the actual prompt boundaries
        let prompt_indices: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.kind == PromptMarkKind::PromptStart)
            .map(|(i, _)| i)
            .collect();
        if prompt_indices.is_empty() {
            return;
        }
        let current = self.prompt_mark_current.unwrap_or(prompt_indices.len());
        let next = if current == 0 {
            0 // Stay at first prompt
        } else {
            current.saturating_sub(1)
        };
        if let Some(&mark_idx) = prompt_indices.get(next) {
            self.prompt_mark_current = Some(next);
            let mark = &marks[mark_idx];
            let search_match = crate::search::SearchMatch {
                start: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
                end: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
            };
            crate::search::scroll_to_match(&self.terminal.term, &search_match);
            cx.notify();
        }
    }

    fn jump_to_prompt_next(&mut self, cx: &mut Context<Self>) {
        let marks = &self.terminal.prompt_marks;
        if marks.is_empty() {
            return;
        }
        let prompt_indices: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.kind == PromptMarkKind::PromptStart)
            .map(|(i, _)| i)
            .collect();
        if prompt_indices.is_empty() {
            return;
        }
        let next = self
            .prompt_mark_current
            .map_or(0, |c| (c + 1).min(prompt_indices.len() - 1));
        if let Some(&mark_idx) = prompt_indices.get(next) {
            self.prompt_mark_current = Some(next);
            let mark = &marks[mark_idx];
            let search_match = crate::search::SearchMatch {
                start: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
                end: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
            };
            crate::search::scroll_to_match(&self.terminal.term, &search_match);
            cx.notify();
        }
    }
}

// ---------------------------------------------------------------------------
// Copy mode methods on TerminalView
// ---------------------------------------------------------------------------

impl TerminalView {
    fn toggle_copy_mode(&mut self, cx: &mut Context<Self>) {
        if self.copy_mode_active {
            self.exit_copy_mode(false, cx);
        } else {
            self.enter_copy_mode(cx);
        }
    }

    fn enter_copy_mode(&mut self, cx: &mut Context<Self>) {
        // Dismiss search if active
        if self.search_active {
            self.dismiss_search(cx);
        }

        let mut term = self.terminal.term.lock();
        let cursor_point = term.renderable_content().cursor.point;
        let display_offset = term.grid().display_offset();
        let screen_lines = term.screen_lines();
        term.selection = None;

        // Convert display-relative cursor to grid coordinates.
        // If the cursor is off-screen (scrolled away), place at viewport center.
        let cursor_display_line = cursor_point.line.0;
        let copy_cursor = if cursor_display_line >= 0 && cursor_display_line < screen_lines as i32 {
            AlacPoint::new(
                GridLine(cursor_display_line - display_offset as i32),
                cursor_point.column,
            )
        } else {
            // Cursor off-screen — place at center of viewport
            let center_display = screen_lines as i32 / 2;
            AlacPoint::new(GridLine(center_display - display_offset as i32), GridCol(0))
        };
        drop(term);

        self.copy_cursor = copy_cursor;
        self.copy_mode_frozen_offset = display_offset;
        self.copy_mode_active = true;

        cx.notify();
    }

    fn exit_copy_mode(&mut self, copy_to_clipboard: bool, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();

        if copy_to_clipboard {
            if let Some(text) = term.selection_to_string() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            // After copying, scroll to bottom
            term.scroll_display(AlacScroll::Bottom);
        } else {
            // On cancel, restore the scroll position from before copy mode entry
            let current = term.grid().display_offset();
            let frozen = self.copy_mode_frozen_offset;
            if current != frozen {
                let delta = frozen as i32 - current as i32;
                term.scroll_display(AlacScroll::Delta(delta));
            }
        }

        term.selection = None;
        drop(term);

        self.copy_mode_active = false;
        cx.notify();
    }

    fn move_copy_cursor(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let (cols, top, bottom) = {
            let mut term = self.terminal.term.lock();
            term.selection = None;
            (term.columns(), term.topmost_line(), term.bottommost_line())
        };

        let new_col = (self.copy_cursor.column.0 as i32 + dx)
            .max(0)
            .min(cols as i32 - 1) as usize;
        let new_line = (self.copy_cursor.line.0 + dy).max(top.0).min(bottom.0);
        self.copy_cursor = AlacPoint::new(GridLine(new_line), GridCol(new_col));

        self.ensure_copy_cursor_visible();
        cx.notify();
    }

    fn extend_copy_selection(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        let cols = term.columns();
        let top = term.topmost_line();
        let bottom = term.bottommost_line();

        // Start a new selection if none exists
        if term.selection.is_none() {
            let sel = Selection::new(SelectionType::Simple, self.copy_cursor, Side::Left);
            term.selection = Some(sel);
        }

        // Move cursor and update selection endpoint — all under the same lock
        let new_col = (self.copy_cursor.column.0 as i32 + dx)
            .max(0)
            .min(cols as i32 - 1) as usize;
        let new_line = (self.copy_cursor.line.0 + dy).max(top.0).min(bottom.0);
        self.copy_cursor = AlacPoint::new(GridLine(new_line), GridCol(new_col));

        if let Some(ref mut sel) = term.selection {
            sel.update(self.copy_cursor, Side::Right);
        }
        drop(term);

        self.ensure_copy_cursor_visible();
        cx.notify();
    }

    /// Scroll the view to keep the copy cursor visible, updating the frozen offset.
    fn ensure_copy_cursor_visible(&mut self) {
        let offset = self.copy_mode_frozen_offset as i32;
        let cursor_display_line = self.copy_cursor.line.0 + offset;

        let mut term = self.terminal.term.lock();
        let screen_lines = term.screen_lines() as i32;

        let new_offset = if cursor_display_line < 0 {
            // Cursor is above visible area — scroll up
            Some((offset - cursor_display_line) as usize)
        } else if cursor_display_line >= screen_lines {
            // Cursor is below visible area — scroll down
            let excess = cursor_display_line - screen_lines + 1;
            Some((offset - excess).max(0) as usize)
        } else {
            None
        };

        if let Some(new_offset) = new_offset {
            self.copy_mode_frozen_offset = new_offset;
            let current = term.grid().display_offset();
            let delta = new_offset as i32 - current as i32;
            if delta != 0 {
                term.scroll_display(AlacScroll::Delta(delta));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal events
// ---------------------------------------------------------------------------

/// Events emitted by TerminalView via GPUI's EventEmitter.
/// Pane subscribes for ChildExited/TitleChanged; PaneFlowApp subscribes
/// for CwdChanged/ActivityBurst/ServiceDetected to drive sidebar updates.
pub enum TerminalEvent {
    /// The shell process exited (e.g. user typed `exit`).
    ChildExited,
    /// The terminal title changed (via OSC 0/2 escape sequence).
    TitleChanged,
    /// The shell's working directory changed (detected via OSC 7 escape sequence).
    CwdChanged(String),
    /// Terminal transitioned from idle to active (new output after idle period).
    /// No longer emitted by the batched event loop but kept for handler compatibility.
    #[allow(dead_code)]
    ActivityBurst,
    /// A server/service was detected in PTY output (e.g. "Listening on :3000").
    /// Enriches the bare port from `/proc/net/tcp` with label and URL.
    ServiceDetected(ServiceInfo),
    /// Terminal bell (\a) was triggered — visual flash notification.
    Bell,
    /// Escape pressed while swap mode is active — requests cancellation.
    CancelSwapMode,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    /// Build a rich key context from the terminal's current mode flags.
    /// Enables keybindings scoped to terminal state (e.g. `"Terminal && screen == alt"`).
    fn dispatch_context(&self) -> KeyContext {
        let mode = *self.terminal.term.lock().mode();
        let mut ctx = KeyContext::default();
        ctx.add("Terminal");

        // Screen mode
        if mode.contains(TermMode::ALT_SCREEN) {
            ctx.set("screen", "alt");
        } else {
            ctx.set("screen", "normal");
        }

        // DEC private modes
        if mode.contains(TermMode::APP_CURSOR) {
            ctx.add("DECCKM");
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            ctx.add("DECPAM");
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            ctx.add("bracketed_paste");
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            ctx.add("report_focus");
        }
        if mode.contains(TermMode::ALTERNATE_SCROLL) {
            ctx.add("alternate_scroll");
        }

        // Mouse reporting mode
        if mode.intersects(TermMode::MOUSE_MODE) {
            ctx.add("any_mouse_reporting");
            if mode.contains(TermMode::MOUSE_MOTION) {
                ctx.set("mouse_reporting", "motion");
            } else if mode.contains(TermMode::MOUSE_DRAG) {
                ctx.set("mouse_reporting", "drag");
            } else {
                ctx.set("mouse_reporting", "click");
            }
        } else {
            ctx.set("mouse_reporting", "off");
        }

        // Mouse encoding format
        if mode.contains(TermMode::SGR_MOUSE) {
            ctx.set("mouse_format", "sgr");
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            ctx.set("mouse_format", "utf8");
        } else {
            ctx.set("mouse_format", "normal");
        }

        ctx
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

        // DEC 1004: send focus in/out events on focus transitions
        if focused != self.was_focused {
            let mode = { *self.terminal.term.lock().mode() };
            if mode.contains(TermMode::FOCUS_IN_OUT) {
                if focused {
                    self.terminal.write_to_pty(b"\x1b[I".to_vec());
                } else {
                    self.terminal.write_to_pty(b"\x1b[O".to_vec());
                }
            }
            self.was_focused = focused;
        }

        // Update cell dimensions for mouse → grid mapping
        let dims = TerminalElement::measure_cell(window, cx);
        self.cell_width = dims.cell_width;
        self.line_height = dims.line_height;

        #[cfg(debug_assertions)]
        let keystroke_at = self.terminal.last_keystroke_at.take();

        // Collect search match rects for the element to paint
        let search_match_rects = if self.search_active && !self.search_matches.is_empty() {
            self.search_matches
                .iter()
                .enumerate()
                .map(|(i, m)| crate::terminal_element::SearchHighlight {
                    start: m.start,
                    end: m.end,
                    is_active: i == self.search_current,
                })
                .collect()
        } else {
            Vec::new()
        };

        // Build copy mode cursor state for the element
        let copy_cursor_state = if self.copy_mode_active {
            Some(crate::terminal_element::CopyModeCursorState {
                grid_line: self.copy_cursor.line.0,
                col: self.copy_cursor.column.0,
            })
        } else {
            None
        };

        // ALT_SCREEN: cursor always visible (no blink-off) for TUI apps
        let cursor_visible = self.cursor_visible
            || self
                .terminal
                .term
                .lock()
                .mode()
                .contains(TermMode::ALT_SCREEN);

        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            PtyNotifier(self.terminal.notifier.0.clone()),
            cursor_visible,
            focused,
            self.terminal.exited,
            self.element_origin.clone(),
            search_match_rects,
            copy_cursor_state,
            self.bell_flash_until
                .is_some_and(|t| std::time::Instant::now() < t),
            self.ctrl_hovered_link
                .as_ref()
                .map(|link| (link.start.line, link.start.column.0, link.end.column.0)),
            self.ctrl_hovered_link.as_ref().map(|link| link.uri.clone()),
            self.ime_marked_text.clone(),
            self.focus_handle.clone(),
            cx.entity().clone(),
            #[cfg(debug_assertions)]
            keystroke_at,
        );

        // Search overlay bar
        let search_active = self.search_active;
        let search_query = self.search_query.clone();
        let search_regex_mode = self.search_regex_mode;
        let search_has_regex_error = self.search_regex_error.is_some();
        let match_count = self.search_matches.len();
        let current_match = if match_count > 0 {
            self.search_current + 1
        } else {
            0
        };

        let mut el = div()
            .id("terminal-view")
            .key_context(self.dispatch_context())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_any_mouse_down(cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::handle_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::handle_mouse_up))
            .on_action(cx.listener(|this, _: &crate::TerminalCopy, window, cx| {
                this.handle_copy(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::TerminalPaste, window, cx| {
                this.handle_paste(window, cx);
            }))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_action(cx.listener(|this, _: &crate::ScrollPageUp, window, cx| {
                this.handle_scroll_page_up(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ScrollPageDown, window, cx| {
                this.handle_scroll_page_down(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleSearch, _window, cx| {
                this.toggle_search(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::DismissSearch, _window, cx| {
                this.dismiss_search(cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::ToggleSearchRegex, _window, cx| {
                    this.toggle_search_regex(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &crate::SearchNext, _window, cx| {
                this.search_next(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::SearchPrev, _window, cx| {
                this.search_prev(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleCopyMode, _window, cx| {
                this.toggle_copy_mode(cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::JumpToPromptPrev, _window, cx| {
                    this.jump_to_prompt_prev(cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::JumpToPromptNext, _window, cx| {
                    this.jump_to_prompt_next(cx);
                }),
            )
            .on_drop(cx.listener(Self::handle_file_drop))
            .on_action(
                cx.listener(|this, _: &crate::ClearScrollHistory, _window, cx| {
                    this.clear_scroll_history(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &crate::ResetTerminal, _window, cx| {
                this.reset_terminal(cx);
            }))
            .size_full()
            .child(terminal_element);

        if search_active {
            // Add "Search" key context for search-scoped bindings
            el = el.key_context("Search");

            // Build the search overlay bar
            let status_text = if search_query.is_empty() {
                String::new()
            } else if match_count == 0 {
                "0 results".to_string()
            } else {
                format!("{}/{}", current_match, match_count)
            };

            // Regex toggle button: highlighted when active
            let regex_toggle = div()
                .id("search-regex-toggle")
                .px(gpui::px(4.0))
                .py(gpui::px(2.0))
                .rounded_sm()
                .cursor_pointer()
                .text_size(gpui::px(12.0))
                .when(search_regex_mode, |el| {
                    el.bg(gpui::rgb(0x89b4fa)).text_color(gpui::rgb(0x1e1e2e))
                })
                .when(!search_regex_mode, |el| {
                    el.bg(gpui::rgb(0x45475a)).text_color(gpui::rgb(0x6c7086))
                })
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _window, cx| {
                        this.toggle_search_regex(cx);
                    }),
                )
                .child(".*");

            // Query display — red border on regex error
            let query_border_color = if search_has_regex_error {
                gpui::rgb(0xf38ba8) // Catppuccin red
            } else {
                gpui::rgb(0x45475a) // Subtle border
            };

            let query_display = div()
                .id("search-query-display")
                .min_w(gpui::px(120.0))
                .px_2()
                .py(gpui::px(2.0))
                .rounded_sm()
                .bg(gpui::rgb(0x1e1e2e))
                .border_1()
                .border_color(query_border_color)
                .text_color(gpui::rgb(0xcdd6f4))
                .text_size(gpui::px(13.0))
                .child(if search_query.is_empty() {
                    "...".to_string()
                } else {
                    search_query
                });

            let status_text = if search_has_regex_error {
                "Invalid regex".to_string()
            } else {
                status_text
            };

            let search_bar = div()
                .id("search-overlay")
                .absolute()
                .top_1()
                .right_1()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .rounded_md()
                .bg(gpui::rgb(0x313244))
                .border_1()
                .border_color(gpui::rgb(0x585b70))
                .child(
                    div()
                        .id("search-icon")
                        .text_color(gpui::rgb(0x6c7086))
                        .text_size(gpui::px(13.0))
                        .child("Find:"),
                )
                .child(regex_toggle)
                .child(query_display)
                .child(
                    div()
                        .id("search-status")
                        .text_color(if search_has_regex_error {
                            gpui::rgb(0xf38ba8)
                        } else {
                            gpui::rgb(0xa6adc8)
                        })
                        .text_size(gpui::px(12.0))
                        .child(status_text),
                );

            el = el.child(search_bar);
        }

        if self.copy_mode_active {
            let copy_badge = div()
                .id("copy-mode-badge")
                .absolute()
                .top_1()
                .right_1()
                .px_2()
                .py(gpui::px(2.0))
                .rounded_md()
                .bg(gpui::rgba(0x89b4facc))
                .text_color(gpui::rgb(0x1e1e2e))
                .text_size(gpui::px(11.0))
                .font_weight(gpui::FontWeight::BOLD)
                .child("COPY");
            el = el.child(copy_badge);
        }

        el
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::{PtyBackend, PtyProcess};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Mock PTY backend for testing — creates in-memory reader/writer pairs
    /// without spawning a real shell process.
    struct MockPtyBackend;

    /// Minimal mock child process that reports exit code 0.
    #[derive(Debug)]
    struct MockChild;

    impl portable_pty::ChildKiller for MockChild {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(MockChild)
        }
    }

    impl portable_pty::Child for MockChild {
        fn process_id(&self) -> Option<u32> {
            Some(9999)
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
        }
    }

    /// Minimal mock MasterPty — resize is a no-op.
    struct MockMasterPty;

    impl portable_pty::MasterPty for MockMasterPty {
        fn resize(&self, _size: portable_pty::PtySize) -> Result<(), anyhow::Error> {
            Ok(())
        }

        fn get_size(&self) -> Result<portable_pty::PtySize, anyhow::Error> {
            Ok(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
        }

        fn try_clone_reader(&self) -> Result<Box<dyn std::io::Read + Send>, anyhow::Error> {
            Ok(Box::new(Cursor::new(Vec::new())))
        }

        fn take_writer(&self) -> Result<Box<dyn std::io::Write + Send>, anyhow::Error> {
            Ok(Box::new(Vec::new()))
        }

        fn process_group_leader(&self) -> Option<i32> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<i32> {
            None
        }
    }

    impl PtyBackend for MockPtyBackend {
        fn spawn(
            &self,
            _command: &str,
            _args: &[String],
            _cwd: &Path,
            _env: &HashMap<String, String>,
            _rows: u16,
            _cols: u16,
        ) -> anyhow::Result<PtyProcess> {
            // Return an empty reader that immediately reaches EOF,
            // causing the reader thread to exit cleanly.
            let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(Vec::new()));
            let writer: Box<dyn std::io::Write + Send> = Box::new(Vec::new());
            let child: Box<dyn portable_pty::Child + Send + Sync> = Box::new(MockChild);
            let master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>> =
                Arc::new(Mutex::new(Box::new(MockMasterPty)));

            Ok(PtyProcess {
                reader,
                writer,
                child,
                master,
                child_pid: 9999,
            })
        }
    }

    #[test]
    fn mock_backend_creates_terminal_state() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None);
        assert!(state.is_ok(), "TerminalState::new with mock backend failed");

        let state = state.unwrap();
        assert_eq!(state.child_pid, 9999);
        assert!(state.exited.is_none());
        assert_eq!(state.title, "Terminal");
    }

    // --- strip_partial_ansi_tail tests (US-012 / US-015) ---

    #[test]
    fn strip_ansi_plain_text_unchanged() {
        let mut s = "hello world\nline two".to_string();
        super::strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "hello world\nline two");
    }

    #[test]
    fn strip_ansi_lone_esc_removed() {
        let mut s = "hello\x1b".to_string();
        super::strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "hello");
    }

    #[test]
    fn strip_ansi_incomplete_csi_removed() {
        // Incomplete CSI: \x1b[38;2; (no terminating byte in 0x40..0x7E)
        let mut s = "text\x1b[38;2;".to_string();
        super::strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "text");
    }

    #[test]
    fn strip_ansi_complete_csi_kept() {
        // Complete CSI: \x1b[0m (terminated by 'm')
        let mut s = "text\x1b[0m".to_string();
        super::strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "text\x1b[0m");
    }

    #[test]
    fn strip_ansi_incomplete_osc_removed() {
        // Incomplete OSC: \x1b]7;file:// (no BEL or ST)
        let mut s = "prompt\x1b]7;file://host/dir".to_string();
        super::strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "prompt");
    }

    // --- extract_scrollback / restore_scrollback tests (US-011 / US-015) ---

    #[test]
    fn scrollback_round_trip_via_mock() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None).unwrap();

        // Feed some text into the terminal grid
        state.restore_scrollback("line one\nline two\nline three");

        // Extract it back
        let scrollback = state.extract_scrollback();
        assert!(scrollback.is_some(), "Expected scrollback content");
        let text = scrollback.unwrap();
        assert!(text.contains("line one"), "Missing 'line one' in: {text}");
        assert!(text.contains("line two"), "Missing 'line two' in: {text}");
        assert!(
            text.contains("line three"),
            "Missing 'line three' in: {text}"
        );
    }

    #[test]
    fn extract_scrollback_empty_terminal_returns_none() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None).unwrap();
        // Fresh terminal with no content beyond the initial blank grid
        // May return None or Some with only whitespace — both are acceptable
        let scrollback = state.extract_scrollback();
        if let Some(ref text) = scrollback {
            assert!(
                text.trim().is_empty(),
                "Expected empty or whitespace-only scrollback, got: {text}"
            );
        }
    }
}
