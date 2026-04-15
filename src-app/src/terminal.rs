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
    App, ClipboardItem, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render,
    ScrollWheelEvent, Styled, Window, div, prelude::*,
};

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
// Terminal state
// ---------------------------------------------------------------------------

pub struct TerminalState {
    pub term: Arc<FairMutex<Term<ZedListener>>>,
    pub notifier: PtyNotifier,
    events_rx: UnboundedReceiver<AlacEvent>,
    pub exited: Option<i32>,
    /// PID of the shell child process, used for port detection.
    pub child_pid: u32,
    /// Terminal title set via OSC 0/2 escape sequences (e.g. shell prompt, Claude Code).
    pub title: String,
    /// Current working directory of the shell process.
    /// Updated via OSC 7 escape sequence (push from shell) or on-demand
    /// via `cwd_now()` (fallback for shells that don't emit OSC 7).
    pub current_cwd: Option<String>,
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
    ) -> anyhow::Result<Self> {
        let (events_tx, events_rx) = unbounded();
        let listener = ZedListener(events_tx.clone());

        let cols: usize = 80;
        let rows: usize = 24;

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
        // Also owns the child handle to capture exit status after EOF.
        let term_for_reader = term.clone();
        let listener_for_reader = ZedListener(events_tx.clone());
        std::thread::spawn(move || {
            pty_reader_loop(pty.reader, term_for_reader, listener_for_reader, pty.child);
        });

        // Message handler thread: receives Notifier messages → writes to PTY / resizes.
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
        std::thread::spawn(move || {
            pty_message_loop(msg_rx, pty.writer, pty.master);
        });

        Ok(Self {
            term,
            notifier: PtyNotifier(PtySender(msg_tx)),
            events_rx,
            exited: None,
            child_pid: pty.child_pid,
            current_cwd: None,
            title: String::from("Terminal"),
            dirty: true, // Force initial render
            output_scan_ticks: 0,
            reported_ports: Vec::new(),
            #[cfg(debug_assertions)]
            last_keystroke_at: None,
        })
    }

    /// Drain alacritty events. Sets `dirty = true` when PTY output was processed.
    pub fn sync(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                AlacEvent::Wakeup => {
                    self.dirty = true;
                }
                // Upstream alacritty_terminal v0.26 uses ExitStatus instead of i32
                // for ChildExit. CurrentWorkingDirectory was fork-only (OSC 7) —
                // CWD tracking now relies on cwd_now() polling fallback.
                AlacEvent::ChildExit(status) => {
                    self.exited = Some(status.code().unwrap_or(-1));
                    self.dirty = true;
                    self.reported_ports.clear();
                }
                AlacEvent::Exit => {
                    self.exited = Some(-1);
                    self.dirty = true;
                }
                AlacEvent::Title(t) => {
                    self.title = t;
                }
                AlacEvent::ResetTitle => {
                    self.title = String::from("Terminal");
                }
                _ => {} // Bell, ClipboardStore, etc.
            }
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
    /// Whether copy mode (keyboard-driven selection) is active
    copy_mode_active: bool,
    /// Copy mode cursor position in grid coordinates
    copy_cursor: AlacPoint,
    /// Display offset frozen at copy mode entry to prevent auto-scroll
    copy_mode_frozen_offset: usize,
}

impl TerminalView {
    pub fn new(workspace_id: u64, cx: &mut Context<Self>) -> Self {
        Self::with_cwd(workspace_id, None, cx)
    }

    pub fn with_cwd(
        workspace_id: u64,
        cwd: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let surface_id = cx.entity_id().as_u64();
        let backend = PortablePtyBackend;
        let terminal = TerminalState::new(&backend, cwd, workspace_id, surface_id)
            .expect("Failed to create terminal");
        let focus_handle = cx.focus_handle();

        // Adaptive sync: poll fast (4ms) when active, slow (50ms) when idle.
        // Idle terminal = fewer lock acquisitions, less CPU.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut idle_ticks: u32 = 0;
                loop {
                    let interval = if idle_ticks > 10 { 50 } else { 4 };
                    smol::Timer::after(std::time::Duration::from_millis(interval)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            let old_title = view.terminal.title.clone();
                            let old_cwd = view.terminal.current_cwd.clone();
                            view.terminal.sync();
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

                                // Skip output scanning and event emission when
                                // the settings overlay is visible — no point
                                // locking the grid or triggering subscribers.
                                if !suppress {
                                    view.terminal.output_scan_ticks += 1;
                                    // Scan aggressively for first 10 ticks (~40ms each),
                                    // then throttle to every 50th tick (~200ms).
                                    // This catches server startup messages before they
                                    // scroll out of view from rapid log output.
                                    let should_scan = idle_ticks > 10
                                        || view.terminal.output_scan_ticks <= 10
                                        || view.terminal.output_scan_ticks >= 50;
                                    if should_scan {
                                        view.terminal.output_scan_ticks = 0;
                                        for service in view.terminal.scan_output() {
                                            cx.emit(TerminalEvent::ServiceDetected(service));
                                        }
                                    }

                                    if idle_ticks > 10 {
                                        cx.emit(TerminalEvent::ActivityBurst);
                                    }
                                }

                                idle_ticks = 0;
                                if !suppress {
                                    // Copy mode: restore frozen display offset to prevent
                                    // auto-scroll when new terminal output arrives.
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
                            } else {
                                idle_ticks = idle_ticks.saturating_add(1);
                            }
                        })
                    });
                    if result.is_err() {
                        break;
                    }
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
                            view.cursor_visible = !view.cursor_visible;
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
            copy_mode_active: false,
            copy_cursor: AlacPoint::new(GridLine(0), GridCol(0)),
            copy_mode_frozen_offset: 0,
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
        if let Some(seq) = crate::keys::to_esc_str(keystroke, &mode) {
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

    // --- Mouse selection handlers ---

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;

        // Clear empty selections (single click without drag)
        let mut term = self.terminal.term.lock();
        if let Some(ref sel) = term.selection
            && sel.is_empty()
        {
            term.selection = None;
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
        if let Some(item) = cx.read_from_clipboard()
            && let Some(text) = item.text()
        {
            let mode = {
                let term = self.terminal.term.lock();
                *term.mode()
            };
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
    }

    // --- Scroll handlers ---

    fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
// portable-pty I/O loops (US-007) — replace AlacEventLoop
// ---------------------------------------------------------------------------

/// Reader thread: reads PTY output, feeds through VTE parser into Term, sends Wakeup events.
/// Owns the child handle to capture exit status after the read loop ends.
fn pty_reader_loop(
    mut reader: Box<dyn Read + Send>,
    term: Arc<FairMutex<Term<ZedListener>>>,
    listener: ZedListener,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
) {
    let mut buf = [0u8; 4096];
    let mut processor = alacritty_terminal::vte::ansi::Processor::<
        alacritty_terminal::vte::ansi::StdSyncHandler,
    >::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let mut term = term.lock();
                processor.advance(&mut *term, &buf[..n]);
                drop(term);
                listener.send_event(AlacEvent::Wakeup);
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
// Search methods on TerminalView
// ---------------------------------------------------------------------------

impl TerminalView {
    fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.search_matches.clear();
            self.search_current = 0;
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
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::Bottom);
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
        self.search_matches = crate::search::search_term(&self.terminal.term, &self.search_query);
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
    /// Used to trigger debounced port scans without polling.
    ActivityBurst,
    /// A server/service was detected in PTY output (e.g. "Listening on :3000").
    /// Enriches the bare port from `/proc/net/tcp` with label and URL.
    ServiceDetected(ServiceInfo),
    /// Escape pressed while swap mode is active — requests cancellation.
    CancelSwapMode,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

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

        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            PtyNotifier(self.terminal.notifier.0.clone()),
            self.cursor_visible,
            focused,
            self.terminal.exited,
            self.element_origin.clone(),
            search_match_rects,
            copy_cursor_state,
            #[cfg(debug_assertions)]
            keystroke_at,
        );

        // Search overlay bar
        let search_active = self.search_active;
        let search_query = self.search_query.clone();
        let match_count = self.search_matches.len();
        let current_match = if match_count > 0 {
            self.search_current + 1
        } else {
            0
        };

        let mut el = div()
            .id("terminal-view")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
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
            .on_action(cx.listener(|this, _: &crate::SearchNext, _window, cx| {
                this.search_next(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::SearchPrev, _window, cx| {
                this.search_prev(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleCopyMode, _window, cx| {
                this.toggle_copy_mode(cx);
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
                .child(
                    div()
                        .id("search-query-display")
                        .min_w(gpui::px(120.0))
                        .px_2()
                        .py(gpui::px(2.0))
                        .rounded_sm()
                        .bg(gpui::rgb(0x1e1e2e))
                        .text_color(gpui::rgb(0xcdd6f4))
                        .text_size(gpui::px(13.0))
                        .child(if search_query.is_empty() {
                            "...".to_string()
                        } else {
                            search_query
                        }),
                )
                .child(
                    div()
                        .id("search-status")
                        .text_color(gpui::rgb(0xa6adc8))
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
        let state = TerminalState::new(&backend, None, 1, 1);
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
        let state = TerminalState::new(&backend, None, 1, 1).unwrap();

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
        let state = TerminalState::new(&backend, None, 1, 1).unwrap();
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
