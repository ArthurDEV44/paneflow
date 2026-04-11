//! Terminal state and view — PTY management and GPUI view wrapper.
//!
//! Manages the alacritty_terminal Term, PTY EventLoop, and periodic sync.
//! The TerminalView creates a TerminalElement for cell-by-cell rendering.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop as AlacEventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty;

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
// Terminal state
// ---------------------------------------------------------------------------

pub struct TerminalState {
    pub term: Arc<FairMutex<Term<ZedListener>>>,
    pub notifier: Notifier,
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

        // Create PTY with OSC 7 shell integration
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut env = std::collections::HashMap::new();
        let extra_args = setup_shell_integration(&shell, &mut env);

        // Inject PaneFlow identity env vars for AI tool hook integration.
        // Child process env only — never inherited by PaneFlow's own process.
        env.insert("PANEFLOW_WORKSPACE_ID".into(), workspace_id.to_string());
        env.insert("PANEFLOW_SURFACE_ID".into(), surface_id.to_string());
        if let Some(socket_path) = paneflow_socket_path() {
            env.insert("PANEFLOW_SOCKET_PATH".into(), socket_path);
        }

        // Extract wrapper scripts and expose their directory for shell integration.
        // We set __PANEFLOW_BIN_DIR as an env var; the shell integration scripts
        // (zsh precmd / bash rcfile) prepend it to PATH AFTER .zshrc/.bashrc load.
        // This ensures our wrappers take priority over user-installed binaries
        // even when .zshrc does `export PATH="$HOME/.local/bin:$PATH"`.
        if let Some(bin_dir) = paneflow_bin_dir() {
            ensure_wrapper_scripts(&bin_dir);
            let bin_dir_str = bin_dir.display().to_string();
            env.insert("__PANEFLOW_BIN_DIR".into(), bin_dir_str.clone());
            // Also set PATH directly as a fallback for shells without integration
            let current_path = std::env::var("PATH").unwrap_or_default();
            if !current_path.split(':').any(|p| p == bin_dir_str) {
                env.insert("PATH".into(), format!("{bin_dir_str}:{current_path}"));
            }
        }

        let cwd = working_directory
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(shell, extra_args)),
            working_directory: Some(cwd),
            drain_on_exit: false,
            env,
        };

        let window_size = WindowSize {
            num_cols: cols as u16,
            num_lines: rows as u16,
            cell_width: 8,
            cell_height: 16,
        };

        let pty = tty::new(&pty_config, window_size, 0)?;
        let child_pid = pty.child().id();

        let event_loop =
            AlacEventLoop::new(term.clone(), ZedListener(events_tx), pty, false, false)?;

        let pty_tx = event_loop.channel();
        let _io_thread = event_loop.spawn();

        Ok(Self {
            term,
            notifier: Notifier(pty_tx),
            events_rx,
            exited: None,
            child_pid,
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
                AlacEvent::CurrentWorkingDirectory(uri) => {
                    if let Some(path) = parse_osc7_uri(&uri) {
                        self.current_cwd = Some(path);
                    }
                }
                AlacEvent::ChildExit(status) => {
                    self.exited = Some(status);
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

    /// Read the shell's CWD from `/proc/<pid>/cwd` on demand.
    /// Fallback for shells that don't emit OSC 7 — used at split time.
    ///
    /// # Platform gaps (for future porting)
    /// - **macOS**: `/proc` doesn't exist. Use `proc_pidinfo()` with
    ///   `PROC_PIDVNODEPATHINFO` instead.
    /// - **Windows**: Use `NtQueryInformationProcess` with
    ///   `ProcessCommandLineInformation`, or `GetFinalPathNameByHandle`.
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        let proc_path = format!("/proc/{}/cwd", self.child_pid);
        std::fs::read_link(&proc_path).ok()
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
        let terminal =
            TerminalState::new(cwd, workspace_id, surface_id).expect("Failed to create terminal");
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
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
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

/// Parse an OSC 7 `file://` URI into a filesystem path.
///
/// Format: `file://hostname/path/to/dir` — hostname is ignored (local only),
/// path is percent-decoded (e.g. `%20` → space). Returns `None` for malformed URIs.
fn parse_osc7_uri(uri: &str) -> Option<String> {
    let path_part = uri.strip_prefix("file://")?;
    // Skip hostname: everything before the first `/` after `file://`
    let path = match path_part.find('/') {
        Some(idx) => &path_part[idx..],
        None => return None,
    };
    if path.is_empty() {
        return None;
    }
    Some(percent_decode(path))
}

/// Decode percent-encoded bytes in a URI path component.
/// Handles `%XX` sequences where XX is a two-digit hex value.
fn percent_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().and_then(|c| (c as char).to_digit(16));
            let lo = bytes.next().and_then(|c| (c as char).to_digit(16));
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8 as char);
            } else {
                out.push('%');
            }
        } else {
            out.push(b as char);
        }
    }
    out
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

        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            Notifier(self.terminal.notifier.0.clone()),
            self.cursor_visible,
            focused,
            self.terminal.exited,
            self.element_origin.clone(),
            #[cfg(debug_assertions)]
            keystroke_at,
        );

        div()
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
            .size_full()
            .child(terminal_element)
    }
}
