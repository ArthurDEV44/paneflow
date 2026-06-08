//! `TerminalState` and its PTY lifecycle — spawn, notifier wiring, event
//! processing, OSC channel drains, CWD resolution, scrollback I/O, and the
//! drop-time force-kill path.
//!
//! Cross-platform: POSIX syscalls (`libc::kill`, `proc_pidinfo`) are behind
//! `#[cfg(unix)]` / `#[cfg(target_os = "macos")]`; Windows paths use
//! `windows-sys` (`TerminateProcess`, `WaitForSingleObject`).
//!
//! Extracted from `terminal.rs` per US-012 of the src-app refactor PRD.

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, Notify, WindowSize as AlacWindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Rgb as AlacRgb;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use super::element::color::palette_color_at;
use super::listener::{SpikeTermSize, ZedListener};
use super::service_detector::{ServiceInfo, detect_framework, parse_service_line};
use super::shell::{resolve_default_shell, setup_shell_integration};
use super::types::SharedTerm;
use crate::limits::{MAX_CHARS, MAX_OSC52_BYTES};

/// Default scrollback history length, in lines. Matches Zed's
/// `DEFAULT_SCROLL_HISTORY_LINES`. `TermConfig::default()` is `0`, which
/// disables scrollback entirely. Overridable via
/// `terminal.scrollback_lines` in `paneflow.json` — see
/// [`paneflow_config::TerminalConfig::resolved_scrollback_lines`].
const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// Read the user's configured scrollback length, clamped to the
/// [`paneflow_config::TerminalConfig`] allowed range. Falls back to
/// [`DEFAULT_SCROLLBACK_LINES`] when no `terminal` block exists.
fn resolved_scrollback_lines() -> usize {
    paneflow_config::loader::load_config()
        .terminal
        .as_ref()
        .map(|t| t.resolved_scrollback_lines())
        .unwrap_or(DEFAULT_SCROLLBACK_LINES)
}

/// US-007: map the pure config cursor shape to the renderer's (vte) shape.
/// Separated from the config read so it is unit-testable. `Hollow` maps to the
/// renderer's `HollowBlock`.
fn map_cursor_shape(
    c: paneflow_config::schema::CursorShapeConfig,
) -> alacritty_terminal::vte::ansi::CursorShape {
    use alacritty_terminal::vte::ansi::CursorShape;
    use paneflow_config::schema::CursorShapeConfig as C;
    match c {
        C::Block => CursorShape::Block,
        C::Beam => CursorShape::Beam,
        C::Underline => CursorShape::Underline,
        C::Hollow => CursorShape::HollowBlock,
    }
}

/// US-007: resolve the configured default cursor shape into an alacritty
/// `CursorStyle`, applied as `TermConfig.default_cursor_style` so it is the
/// fallback before any app-driven DECSCUSR escape. Blinking stays at the
/// alacritty default; cursor blink is overridden at the view layer (US-008).
fn resolved_cursor_style() -> alacritty_terminal::vte::ansi::CursorStyle {
    use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
    let shape = paneflow_config::loader::load_config()
        .terminal
        .as_ref()
        .and_then(|t| t.cursor_shape)
        .map(map_cursor_shape)
        .unwrap_or(CursorShape::Block);
    CursorStyle {
        shape,
        ..CursorStyle::default()
    }
}

// ---------------------------------------------------------------------------
// PTY notifier — replaces alacritty's Notifier (US-007, portable-pty)
// ---------------------------------------------------------------------------

/// Whether a terminal is backed by a real PTY (driven by an alacritty
/// `EventLoop`) or is display-only (VTE-rendered content, no PTY, no input).
/// Mirrors Zed's `TerminalType` (`crates/terminal/src/terminal.rs:1281-1287`):
/// the `Pty` variant owns the `EventLoop` write channel; `DisplayOnly` drops
/// every write. Held inside [`PtySender`] so the Pty-vs-display-only state is
/// one named enum instead of an anonymous `Option`, and so US-012 can *promote*
/// a `DisplayOnly` terminal to `Pty` once a background spawn resolves.
#[derive(Clone)]
pub enum TerminalType {
    /// A live PTY: writes go to the alacritty `EventLoop` channel.
    Pty(EventLoopSender),
    /// No PTY: input / resize / shutdown are dropped.
    DisplayOnly,
}

/// The write side of a terminal — routes input / resize / shutdown to the PTY
/// `EventLoop` (or drops them for a display-only terminal). Mirrors Zed's
/// `PtySender` (`crates/terminal/src/alacritty.rs:84-108`), which exposes only
/// notify / resize / shutdown — never the raw `Msg` channel.
#[derive(Clone)]
pub struct PtySender(TerminalType);

impl PtySender {
    /// Real sender wired to a live `EventLoop` channel.
    pub(super) fn pty(sender: EventLoopSender) -> Self {
        Self(TerminalType::Pty(sender))
    }

    /// Display-only sender: every write is dropped (no PTY, no `EventLoop`).
    pub(super) fn display_only() -> Self {
        Self(TerminalType::DisplayOnly)
    }

    /// Whether this is a live PTY (vs display-only / not-yet-promoted). A
    /// display-only sender already drops every write, so this is an explicit
    /// readiness query for callers/tests rather than a guard the write path
    /// needs.
    #[allow(dead_code)]
    pub fn is_pty(&self) -> bool {
        matches!(self.0, TerminalType::Pty(_))
    }

    /// Internal: drop the message for a display-only terminal, otherwise hand it
    /// to the `EventLoop`. The send error is ignored — a closed channel means
    /// the child already exited, which the exit path handles.
    fn send(&self, msg: Msg) {
        if let TerminalType::Pty(sender) = &self.0 {
            let _ = sender.send(msg);
        }
    }

    /// Forward input bytes to the child (the [`Notify`] path).
    pub fn write(&self, bytes: Cow<'static, [u8]>) {
        // alacritty: the terminal hangs if 0 bytes are sent through.
        if bytes.is_empty() {
            return;
        }
        self.send(Msg::Input(bytes));
    }

    /// Resize the PTY grid (drives SIGWINCH to the child).
    pub fn resize(&self, size: AlacWindowSize) {
        self.send(Msg::Resize(size));
    }

    /// Ask the `EventLoop` to shut down (sent from `Drop` before the teardown
    /// ladder).
    pub fn shutdown(&self) {
        self.send(Msg::Shutdown);
    }
}

/// Wrapper for the PTY write channel. Implements `Notify` for input and exposes
/// the resize convenience — same usage pattern as alacritty's `Notifier` (which
/// [`PtySender`] now wraps).
pub struct PtyNotifier(pub PtySender);

impl Notify for PtyNotifier {
    fn notify<B: Into<Cow<'static, [u8]>>>(&self, bytes: B) {
        self.0.write(bytes.into());
    }
}

impl PtyNotifier {
    /// Resize the PTY (drives SIGWINCH to the child) without the caller naming
    /// alacritty's `Msg`/`WindowSize`. Keeps the renderer's resize path off
    /// `alacritty_terminal` so EP-003 confinement holds in `element/`.
    pub fn notify_resize(&self, num_cols: u16, num_lines: u16, cell_width: u16, cell_height: u16) {
        self.0.resize(AlacWindowSize {
            num_cols,
            num_lines,
            cell_width,
            cell_height,
        });
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

/// Deferred clipboard operation from sync() — executed in cx.update() closure.
pub(super) enum ClipboardOp {
    Store(String),
    Load(std::sync::Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

/// Convert GPUI Hsla to alacritty Rgb for color query responses.
pub(super) fn hsla_to_alac_rgb(hsla: gpui::Hsla) -> AlacRgb {
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
    pub(super) events_rx: Option<UnboundedReceiver<AlacEvent>>,
    pub exited: Option<i32>,
    /// US-002: set true once any user input (keystroke, paste, mouse report,
    /// IME commit, user scroll) has been written via `write_to_pty`.
    /// Distinguishes a user-initiated exit (always close the pane) from a
    /// spawn/launch failure (keep the pane open so the exit overlay is
    /// visible). Atomic because `write_to_pty` takes `&self`. Mirrors Zed's
    /// keyboard_input_sent (crates/terminal/src/terminal.rs:2572-2576).
    keyboard_input_sent: std::sync::atomic::AtomicBool,
    /// EP-002 US-005: numeric signal + name if the child was terminated by a
    /// signal (crash), formatted "N (Name)" e.g. "11 (Segmentation fault)".
    /// `None` for a normal code exit. The numeric signal comes directly from
    /// alacritty's native `ChildExit(ExitStatus)` via `ExitStatusExt::signal()`
    /// (no strsignal reversal); the name is `strsignal(n)`. Set in
    /// `process_event`. Rendered by the exit overlay to flag a crash.
    pub exit_signal: Option<String>,
    /// PID of the shell child process, used for port detection.
    pub child_pid: u32,
    /// US-019: raw fd of the PTY master, captured at spawn before the master
    /// moves into the message-loop thread. macOS uses it to call
    /// `tcgetpgrp(fd)` for live foreground-process naming. `None` on the
    /// display-only / mock paths (no real PTY). macOS-only — Linux resolves the
    /// foreground process from `/proc`, Windows from `child_pid`.
    #[cfg(target_os = "macos")]
    pty_master_fd: Option<i32>,
    /// Terminal title set via OSC 0/2 escape sequences (e.g. shell prompt, Claude Code).
    pub title: String,
    /// Current working directory of the shell process. EP-002 US-007: derived
    /// from the process table via `cwd_now()` (proc/libproc), polled in
    /// `sync_channels`. The pre-VTE OSC 7 byte-scanner was removed with the
    /// 2-thread reader (the EventLoop owns the read path with no pre-parse hook).
    pub current_cwd: Option<String>,
    /// User-assigned custom name (US-013). When `Some`, it overrides the
    /// auto-derived surface name in `surface.list` / MCP / the sidebar, and is
    /// persisted to `session.json`. `None` falls back to derivation.
    pub custom_name: Option<String>,
    /// OSC 52 clipboard access mode (default: copy-only for security).
    pub osc52_mode: Osc52Mode,
    /// Deferred clipboard operations from sync() — drained in the poll loop
    /// where cx is available for clipboard read/write.
    pub(super) pending_clipboard_ops: Vec<ClipboardOp>,
    /// Deferred text area size request responses from sync().
    pub(super) pending_size_ops:
        Vec<std::sync::Arc<dyn Fn(AlacWindowSize) -> String + Sync + Send + 'static>>,
    /// Bell event received — triggers visual flash in poll loop.
    pub bell_active: bool,
    /// Whether the terminal wants the cursor to blink (from CursorBlinkingChange).
    pub cursor_blinking: bool,
    /// Set when PTY output has been processed (Wakeup event received).
    /// Cleared after cx.notify() triggers a repaint.
    pub dirty: bool,
    /// Counter for throttling output scans — scans every 50th dirty tick.
    pub(super) output_scan_ticks: u32,
    /// EP-002 US-007: throttle counter for the proc-based CWD refresh in
    /// `sync_channels` (the OSC 7 byte-scanner was removed with the 2-thread
    /// reader; the EventLoop owns the read path with no pre-parse hook).
    cwd_poll_ticks: u32,
    /// Ports already reported via ServiceDetected (dedup guard).
    /// Cleared on ChildExit so a restarted server is re-detected.
    /// U-052: a `HashSet` bounds membership to O(1) and the structure to a
    /// flat per-distinct-port cost, vs. the old `Vec` whose linear `.contains`
    /// and unbounded growth scaled with every detected service.
    reported_ports: std::collections::HashSet<u16>,
    /// Timestamp of the most recent keystroke, used by latency probes
    /// to measure total keystroke-to-pixel time. Debug builds only.
    /// Note: on rapid keystrokes before a render frame, earlier timestamps are overwritten.
    #[cfg(debug_assertions)]
    pub(crate) last_keystroke_at: Option<std::time::Instant>,
    /// GPUI background executor used by `Drop` to schedule the
    /// grace-period force-kill task. Wired by `TerminalView::with_cwd`
    /// immediately after construction. `None` only on display-only /
    /// test paths, where `Drop` falls back to a detached OS thread.
    /// Mirrors Zed `crates/terminal/src/terminal.rs:2451-2457` which
    /// uses `background_executor.spawn(...).detach()` to keep the
    /// kill timer under the GPUI scheduler instead of leaking an
    /// orphan OS thread per closed pane.
    background_executor: Option<gpui::BackgroundExecutor>,
    /// US-012: input written through `write_to_pty` while the terminal is
    /// still display-only (the PTY opens on a background thread and is
    /// installed later by [`promote`](Self::promote)). The display-only
    /// notifier silently drops every write, so without this queue an
    /// auto-launch command issued the instant a thread mounts (the
    /// Agents-view "New thread" picker) — or a keystroke typed in the brief
    /// pre-promotion window — would be lost. [`promote`](Self::promote)
    /// flushes it in order. `Mutex` (not `RefCell`) keeps `TerminalState`
    /// `Send` and matches the crate's interior-mutability idiom; the lock is
    /// uncontended (main thread only).
    pending_input: std::sync::Mutex<Vec<Cow<'static, [u8]>>>,
}

/// Cap on input buffered during the pre-promotion window. Generous for a
/// launch command plus a burst of typing, tight enough that a terminal that
/// never promotes (spawn failure — `promote` is never called) cannot
/// accumulate input without bound.
const MAX_PENDING_INPUT_BYTES: usize = 64 * 1024;

/// The cheap, render-thread-safe half of a spawn: resolved shell, assembled
/// child env, cwd, and grid size. Produced by
/// [`TerminalState::resolve_spawn_params`] and consumed by
/// [`TerminalState::open_pty_and_eventloop`] (which may run on a background
/// thread). All fields are `Send`.
pub(super) struct SpawnParams {
    shell: String,
    extra_args: Vec<String>,
    env: std::collections::HashMap<String, String>,
    cwd: std::path::PathBuf,
    pub(super) cols: usize,
    pub(super) rows: usize,
    surface_id: u64,
}

/// The live PTY handles produced by [`TerminalState::open_pty_and_eventloop`]:
/// the `EventLoop` write channel, the child PID, and (macOS) the master fd.
/// Crosses the background→main boundary to [`TerminalState::promote`]; all
/// fields are `Send`.
pub(super) struct SpawnedPty {
    channel: EventLoopSender,
    child_pid: u32,
    #[cfg(target_os = "macos")]
    pty_master_fd: Option<i32>,
}

/// Foreground (main-thread) signal mask, captured so an off-thread PTY spawn
/// (US-012) doesn't hand the child the background executor's mask (which blocks
/// SIGINT/SIGTSTP and would break Ctrl-C / Ctrl-Z). Unix-only — a ZST on
/// Windows.
#[cfg(unix)]
pub type ForegroundSignalMask = libc::sigset_t;
#[cfg(not(unix))]
pub type ForegroundSignalMask = ();

/// Capture the calling thread's signal mask. Call on the main thread before
/// scheduling an off-thread spawn; thread the result through to
/// [`TerminalState::open_pty_and_eventloop`].
pub(super) fn capture_foreground_signal_mask() -> Option<ForegroundSignalMask> {
    #[cfg(unix)]
    {
        // SAFETY: `pthread_sigmask` with a null `set` only reads the current
        // mask into `oldset`; nothing is changed.
        unsafe {
            let mut oldset: libc::sigset_t = std::mem::zeroed();
            if libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut oldset) == 0 {
                Some(oldset)
            } else {
                None
            }
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Install `mask` on the current thread, returning the previous mask to restore.
/// Brackets the `tty::new` fork so the child inherits the foreground signal
/// disposition even when the spawn runs on a background thread (US-012).
#[cfg(unix)]
fn apply_thread_signal_mask(mask: Option<ForegroundSignalMask>) -> Option<libc::sigset_t> {
    let fg = mask?;
    // SAFETY: set this thread's mask to the captured foreground mask, saving the
    // previous one into `saved` for `restore_thread_signal_mask`.
    unsafe {
        let mut saved: libc::sigset_t = std::mem::zeroed();
        if libc::pthread_sigmask(libc::SIG_SETMASK, &fg, &mut saved) == 0 {
            Some(saved)
        } else {
            None
        }
    }
}

/// Restore a thread mask saved by [`apply_thread_signal_mask`].
#[cfg(unix)]
fn restore_thread_signal_mask(saved: Option<libc::sigset_t>) {
    if let Some(saved) = saved {
        // SAFETY: restore the previously-saved mask on this thread.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &saved, std::ptr::null_mut());
        }
    }
}

impl TerminalState {
    /// Spawn a real PTY-backed terminal synchronously. Resolves the shell + env
    /// ([`resolve_spawn_params`]), builds a display-only `Term`
    /// ([`new_pending`]), opens the PTY ([`open_pty_and_eventloop`]), and
    /// promotes it to a live `Pty` ([`promote`]). The off-thread path
    /// (`TerminalView::with_cwd_and_env`, US-012) runs the same four steps but
    /// spreads the blocking one across the background executor with a
    /// `signal_mask` so the render thread never blocks on the spawn.
    ///
    /// `signal_mask` is `None` on the synchronous main-thread path (the
    /// foreground mask is already active); the off-thread path passes the
    /// captured foreground mask so the child still gets correct Ctrl-C.
    ///
    /// The production GUI path spawns off-thread (`with_cwd_and_env` →
    /// `new_pending` + `open_pty_and_eventloop` + `promote`); this synchronous
    /// composition is the reference path, exercised end-to-end by the live
    /// `eventloop_pty_echoes_input_into_grid` smoke and available to any future
    /// non-GUI (headless) caller.
    #[allow(dead_code)]
    pub fn new(
        working_directory: Option<std::path::PathBuf>,
        workspace_id: u64,
        surface_id: u64,
        initial_size: Option<(usize, usize)>,
        user_env: Option<std::collections::HashMap<String, String>>,
        signal_mask: Option<ForegroundSignalMask>,
    ) -> anyhow::Result<Self> {
        let params = Self::resolve_spawn_params(
            working_directory,
            workspace_id,
            surface_id,
            initial_size,
            user_env,
        );
        let (mut state, events_tx) = Self::new_pending(params.cols, params.rows);
        let term = state.term.clone();
        let spawned = Self::open_pty_and_eventloop(params, term, events_tx, signal_mask)?;
        state.promote(spawned);
        Ok(state)
    }

    /// Resolve the shell, the merged + assembled child env, the cwd, and the
    /// grid size — the cheap, render-thread-safe half of a spawn. Factored out
    /// of `new` so the off-thread path (US-012) runs the *blocking* half
    /// ([`open_pty_and_eventloop`]) on the background executor.
    pub(super) fn resolve_spawn_params(
        working_directory: Option<std::path::PathBuf>,
        workspace_id: u64,
        surface_id: u64,
        initial_size: Option<(usize, usize)>,
        user_env: Option<std::collections::HashMap<String, String>>,
    ) -> SpawnParams {
        // Fallback chain handled by `resolve_default_shell` (US-006):
        // Unix:    config → $SHELL → /bin/sh
        // Windows: config → %ComSpec% → C:\Windows\System32\cmd.exe →
        //          powershell.exe on PATH → bare "cmd.exe"
        let config = paneflow_config::loader::load_config();
        let shell = {
            let configured = config
                .default_shell
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            resolve_default_shell(configured)
        };
        // US-014: layer the per-surface `user_env` on top of the global
        // `terminal.env` default (surface wins on key collision).
        let global_env = config.terminal.as_ref().and_then(|t| t.env.clone());
        let merged_env = match (global_env, user_env) {
            (None, None) => None,
            (Some(g), None) => Some(g),
            (None, Some(s)) => Some(s),
            (Some(mut g), Some(s)) => {
                g.extend(s);
                Some(g)
            }
        };
        let mut env = std::collections::HashMap::new();
        let extra_args = setup_shell_integration(&shell, &mut env);
        // Assemble the child environment (identity vars, TERM, AI-hook PATH
        // prepend, user-env merge with protected keys). Pure function so the env
        // contract stays unit-testable (the mockable `PtyBackend::spawn` seam is
        // gone — EP-002 US-004).
        let env = assemble_pty_env(env, workspace_id, surface_id, merged_env);
        // U-026: on `current_dir()` failure (deleted cwd, permission loss),
        // fall back to the user's home dir rather than `/` — spawning a shell
        // at the filesystem root is surprising and strands the user. `/` stays
        // only as the absolute last resort if even the home dir is unknown.
        let cwd = working_directory.unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|e| {
                log::warn!("pty: current_dir() failed ({e}); falling back to home dir");
                dirs::home_dir().unwrap_or_else(|| "/".into())
            })
        });
        let (cols, rows) = initial_size.unwrap_or((120, 40));
        SpawnParams {
            shell,
            extra_args,
            env,
            cwd,
            cols,
            rows,
            surface_id,
        }
    }

    /// Build a display-only terminal that retains its event-channel *sender* so
    /// a background spawn can later attach a real `EventLoop` to the same
    /// channel and [`promote`](Self::promote) it (US-012). The returned
    /// `UnboundedSender` is the clone handed to [`open_pty_and_eventloop`].
    pub(super) fn new_pending(cols: usize, rows: usize) -> (Self, UnboundedSender<AlacEvent>) {
        Self::build_display_only(cols, rows)
    }

    /// Open the PTY and start its `EventLoop` on the given (shared) `term` and
    /// event channel — the *blocking* half of a spawn (`tty::new` forks). Safe
    /// to call on a background thread: when `signal_mask` is `Some`, it is
    /// installed on this thread around the `tty::new` fork so the child inherits
    /// the foreground (main-thread) signal disposition and Ctrl-C / Ctrl-Z keep
    /// working, then the thread's mask is restored. Upstream `alacritty_terminal`
    /// exposes no `child_signal_mask` pty option (Zed's #58004 is a fork
    /// addition), so bracketing the fork with the thread mask is the
    /// upstream-only equivalent.
    pub(super) fn open_pty_and_eventloop(
        params: SpawnParams,
        term: SharedTerm,
        events_tx: UnboundedSender<AlacEvent>,
        signal_mask: Option<ForegroundSignalMask>,
    ) -> anyhow::Result<SpawnedPty> {
        let listener = ZedListener(events_tx);
        // Pixel size unknown at spawn (apps use the char grid); the live size is
        // pushed via `Msg::Resize` on the first frame.
        let window_size = AlacWindowSize {
            num_cols: params.cols as u16,
            num_lines: params.rows as u16,
            cell_width: 0,
            cell_height: 0,
        };
        let options = tty::Options {
            shell: Some(tty::Shell::new(params.shell, params.extra_args)),
            working_directory: Some(params.cwd),
            // We close the master and signal the child ourselves in `Drop`;
            // matches Zed (`drain_on_exit: false`).
            drain_on_exit: false,
            env: params.env,
            #[cfg(windows)]
            escape_args: true,
        };

        // EP-002 US-004: open the PTY via alacritty's own cross-platform `tty`
        // (Unix openpty + setsid, Windows ConPTY) and drive it with alacritty's
        // `EventLoop`. Mirrors Zed `crates/terminal/src/alacritty.rs`.
        //
        // US-012: bracket the fork with the captured foreground signal mask so
        // an off-thread spawn doesn't hand the child the background executor's
        // signal-blocking mask. No-op on the synchronous path (`signal_mask` is
        // `None`) and on Windows.
        #[cfg(unix)]
        let restore_mask = apply_thread_signal_mask(signal_mask);
        #[cfg(not(unix))]
        let _ = signal_mask;

        let pty = tty::new(&options, window_size, params.surface_id);

        #[cfg(unix)]
        restore_thread_signal_mask(restore_mask);

        let pty = pty.map_err(|e| anyhow::anyhow!("failed to open pty: {e}"))?;

        // Capture the child PID (teardown ladder + port detection) and, on
        // macOS, the PTY master fd (`tcgetpgrp` foreground naming) BEFORE the
        // EventLoop consumes the `Pty`. alacritty `pre_exec`s `setsid()`, so the
        // child is its own session/group leader and `child_pid` is also the PGID
        // (the `kill(-pid, …)` group teardown in `Drop` stays valid). Mirrors Zed
        // `ProcessIdGetter::from(&AlacrittyPty)`.
        #[cfg(unix)]
        let child_pid = pty.child().id();
        #[cfg(windows)]
        let child_pid = pty.child_watcher().pid().map(u32::from).unwrap_or(0);
        #[cfg(target_os = "macos")]
        let pty_master_fd = {
            use std::os::unix::io::AsRawFd;
            // US-034: `dup()` the master fd so we own a copy whose lifetime we
            // control (closed in `Drop`). The borrowed `pty.file().as_raw_fd()`
            // is closed when the EventLoop (which takes ownership of `pty`
            // below) tears the PTY down on child exit, and the OS may reuse
            // that fd number — `tcgetpgrp(stale_fd)` would then report an
            // unrelated process group, defeating the `p > 0` filter.
            let raw = pty.file().as_raw_fd();
            // SAFETY: `raw` is a valid open fd for the PTY master; `dup`
            // returns a fresh owned fd or -1 on error (filtered out).
            let dup = unsafe { libc::dup(raw) };
            (dup >= 0).then_some(dup)
        };

        let event_loop = EventLoop::new(
            term, listener, pty, false, // drain_on_exit
            false, // ref_test
        )
        .map_err(|e| anyhow::anyhow!("failed to start pty event loop: {e}"))?;
        let channel = event_loop.channel();
        // The IO thread runs detached; shutdown is driven by `Msg::Shutdown` in
        // `Drop`. The handle is dropped (the thread joins itself on shutdown).
        let _io_thread = event_loop.spawn();

        Ok(SpawnedPty {
            channel,
            child_pid,
            #[cfg(target_os = "macos")]
            pty_master_fd,
        })
    }

    /// Promote a display-only / pending terminal to a live PTY by installing the
    /// `EventLoop` write channel, child PID, and interactive defaults produced
    /// by [`open_pty_and_eventloop`]. The grid `Term` is unchanged — the
    /// background `EventLoop` was attached to the same shared `term`, so output
    /// already flows; this just opens the write side and lets `Drop` reach the
    /// child.
    pub(super) fn promote(&mut self, spawned: SpawnedPty) {
        self.notifier = PtyNotifier(PtySender::pty(spawned.channel));
        self.child_pid = spawned.child_pid;
        #[cfg(target_os = "macos")]
        {
            self.pty_master_fd = spawned.pty_master_fd;
        }
        // Interactive defaults (a display-only terminal had these off).
        self.osc52_mode = Osc52Mode::CopyOnly;
        self.cursor_blinking = true;
        self.dirty = true;
        // Flush input queued while display-only (US-012): the launch command
        // an Agents-view thread issues the instant it mounts, plus any
        // keystrokes typed before the off-thread fork resolved. Order is
        // preserved; the now-live `Pty` notifier delivers each to the child.
        if let Ok(mut pending) = self.pending_input.lock() {
            for input in pending.drain(..) {
                self.notifier.notify(input);
            }
        }
    }

    /// Wire a GPUI background executor for the grace-period force-kill
    /// task spawned in `Drop`. Without this, the kill timer runs on a
    /// detached OS thread (works, but leaks one thread per closed pane
    /// on intensive use). Called by `TerminalView::with_cwd` so the
    /// production path always goes through GPUI's scheduler.
    pub fn set_background_executor(&mut self, executor: gpui::BackgroundExecutor) {
        self.background_executor = Some(executor);
    }

    /// Create a display-only terminal with no PTY, no reader thread, no message loop.
    /// Content is rendered via `write_output()` which processes bytes through VTE directly.
    /// The terminal supports full ANSI rendering but does not accept keyboard input.
    /// Used by tests (the production spawn-failure fallback keeps the
    /// already-built pending placeholder and writes the error into it).
    #[allow(dead_code)]
    pub fn new_display_only(rows: usize, cols: usize) -> Self {
        Self::build_display_only(cols, rows).0
    }

    /// Shared constructor for the display-only / pending state. Returns the
    /// terminal plus a clone of its event-channel *sender*, so the off-thread
    /// spawn path ([`new_pending`]) can wire a real `EventLoop` to the same
    /// channel and [`promote`](Self::promote) it (US-012). `new_display_only`
    /// discards the sender (its `Term` only emits Wakeups on its own VTE writes).
    fn build_display_only(cols: usize, rows: usize) -> (Self, UnboundedSender<AlacEvent>) {
        let (events_tx, events_rx) = unbounded();
        // The Term keeps one clone (emits Wakeup after VTE mutations); the
        // returned clone is for a later `EventLoop` on promotion.
        let listener = ZedListener(events_tx.clone());

        let config = TermConfig {
            scrolling_history: resolved_scrollback_lines(),
            default_cursor_style: resolved_cursor_style(),
            ..TermConfig::default()
        };
        let dimensions = SpikeTermSize {
            columns: cols,
            screen_lines: rows,
        };
        let term = Term::new(config, &dimensions, listener);
        let term = Arc::new(FairMutex::new(term));

        let state = Self {
            term,
            // No PTY / EventLoop yet — notifier sends are silently dropped until
            // `promote()` installs a `Pty` sender.
            notifier: PtyNotifier(PtySender::display_only()),
            events_rx: Some(events_rx),
            exited: None,
            keyboard_input_sent: std::sync::atomic::AtomicBool::new(false),
            exit_signal: None,
            child_pid: 0,
            #[cfg(target_os = "macos")]
            pty_master_fd: None,
            current_cwd: None,
            custom_name: None,
            osc52_mode: Osc52Mode::Disabled,
            pending_clipboard_ops: Vec::new(),
            pending_size_ops: Vec::new(),
            bell_active: false,
            cursor_blinking: false,
            title: String::from("Terminal"),
            dirty: true,
            output_scan_ticks: 0,
            cwd_poll_ticks: 0,
            reported_ports: std::collections::HashSet::new(),
            #[cfg(debug_assertions)]
            last_keystroke_at: None,
            background_executor: None,
            pending_input: std::sync::Mutex::new(Vec::new()),
        };
        (state, events_tx)
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

    /// Refresh the shell CWD from the process table (EP-002 US-007).
    ///
    /// The pre-VTE OSC 7 byte-scanner was removed with the 2-thread reader (the
    /// EventLoop owns the read path with no pre-parse hook), so CWD is now
    /// derived from `/proc` (Linux) / `libproc` (macOS) via `cwd_now()`.
    /// Throttled so we don't `readlink` on every poll tick. Called by the
    /// batched event loop, which handles alacritty events directly.
    pub fn sync_channels(&mut self) {
        self.cwd_poll_ticks = self.cwd_poll_ticks.wrapping_add(1);
        if self.cwd_poll_ticks.is_multiple_of(25)
            && let Some(cwd) = self.cwd_now()
        {
            self.current_cwd = Some(cwd.to_string_lossy().into_owned());
        }
    }

    /// Defensively reset terminal modes that could corrupt the outer terminal.
    /// Called on child exit before marking the terminal as exited.
    /// Only resets modes that are actually active (clean exits won't trigger).
    fn reset_active_modes(&mut self) {
        // Guard against double-reset: if we've already recorded the exit
        // status, the PTY writer is already closed and the next notify()
        // would log a swallowed EPIPE.
        if self.exited.is_some() {
            return;
        }
        let mode = *self.term.lock_unfair().mode();
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
                // EP-002 US-005: exit status now comes natively from alacritty's
                // `ChildExit(ExitStatus)`. On Unix a signal-kill has no `code()`
                // but carries the numeric signal via `ExitStatusExt::signal()`;
                // pair it with `strsignal` for the overlay ("11 (Segmentation
                // fault)"). No `from_raw(code<<8)` reconstruction and no
                // in-process strsignal reversal — alacritty hands us the number.
                #[cfg(unix)]
                if status.code().is_none()
                    && let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&status)
                {
                    self.exit_signal = Some(format_signal(sig));
                }
                self.exited = Some(status.code().unwrap_or(-1));
                self.dirty = true;
                self.reported_ports.clear();
            }
            AlacEvent::Exit => {
                self.reset_active_modes();
                // First-write-wins (US-003 AC): `Exit` is the EOF fallback with no
                // status. A real `ChildExit` code must never be clobbered by the -1
                // sentinel if both events fire. Mirrors Zed's register_task_finished
                // (crates/terminal/src/terminal.rs:2561-2563), where only ChildExit
                // stores a status and Exit is a status no-op.
                if self.exited.is_none() {
                    self.exited = Some(-1);
                }
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
                // Cap to prevent memory DoS from malicious programs (crate::limits).
                let within_cap =
                    self.osc52_mode != Osc52Mode::Disabled && text.len() <= MAX_OSC52_BYTES;
                if within_cap {
                    self.pending_clipboard_ops.push(ClipboardOp::Store(text));
                }
            }
            AlacEvent::ClipboardLoad(_selection, format_fn)
                if self.osc52_mode == Osc52Mode::CopyPaste =>
            {
                self.pending_clipboard_ops
                    .push(ClipboardOp::Load(format_fn));
            }
            AlacEvent::ClipboardLoad(..) => {}

            AlacEvent::ColorRequest(index, format_fn) => {
                // Respond synchronously to preserve PTY-write order — match
                // Zed (`crates/terminal/src/terminal.rs:997-1009`). Crossterm's
                // `query_foreground_color` / `query_background_color` (used by
                // the OpenAI Codex CLI to detect terminal colors and decide
                // whether to paint its input-bar tint) has a short timeout;
                // a deferred reply both misses it and scrambles ordering with
                // a following `\e[c` (DA1) query, after which Codex falls back
                // to "unknown bg" and silently drops the tint.
                //
                // The `index` here is alacritty's internal `NamedColor`
                // discriminant, NOT the OSC code itself: the VTE parser at
                // `vte-0.15/src/ansi.rs:1431` translates OSC 10/11/12 to
                // `NamedColor::Foreground (256) + (osc_code - 10)`. So the
                // 256/257/258 arms below match OSC 10/11/12; indices 0..=255
                // cover OSC 4 (`OSC 4 ; n ; ?` color-palette queries) which
                // some apps (vim, neovim, python-rich) use to detect themes.
                let theme = crate::theme::active_theme();
                use alacritty_terminal::vte::ansi::NamedColor;
                let color = if index == NamedColor::Foreground as usize {
                    Some(theme.foreground)
                } else if index == NamedColor::Background as usize {
                    Some(theme.ansi_background)
                } else if index == NamedColor::Cursor as usize {
                    Some(theme.cursor)
                } else if index < 256 {
                    Some(palette_color_at(index as u8, &theme))
                } else {
                    None
                };
                if let Some(hsla) = color {
                    let rgb = hsla_to_alac_rgb(hsla);
                    let response = format_fn(rgb);
                    self.notifier.notify(response.into_bytes());
                }
            }
            AlacEvent::Bell => {
                self.bell_active = true;
            }
            AlacEvent::CursorBlinkingChange => {
                let term = self.term.lock_unfair();
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
        // US-034: once the child has exited, `child_pid` is stale and the OS
        // may have reused it for an unrelated process — reading
        // `/proc/<pid>/cwd` would silently return a third party's CWD. Bail.
        if self.exited.is_some() {
            return None;
        }
        let proc_path = format!("/proc/{}/cwd", self.child_pid);
        std::fs::read_link(&proc_path).ok()
    }

    /// macOS implementation of `cwd_now`: read the PTY child shell's current
    /// working directory from the kernel via
    /// `proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &buf, size)`.
    #[cfg(target_os = "macos")]
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        use std::ffi::CStr;
        use std::mem::MaybeUninit;
        use std::os::raw::c_void;

        // US-034: after exit, `child_pid` may have been reused — `proc_pidinfo`
        // would return an unrelated process's CWD. Bail.
        if self.exited.is_some() {
            return None;
        }

        let pid = self.child_pid as libc::c_int;
        let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;

        // SAFETY: `info` is a stack-allocated MaybeUninit zeroed above; we
        // only read from it if the syscall reports the full struct size
        // was written. Zeroing first leaves it in a defined state on any
        // partial-write error path.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                info.as_mut_ptr() as *mut c_void,
                size,
            )
        };

        if written <= 0 {
            let err = std::io::Error::last_os_error();
            log::warn!(
                "cwd_now: proc_pidinfo(pid={pid}) returned {written} ({err}) — shell may have exited or SIP / sandbox is denying the read"
            );
            return None;
        }

        if written < size {
            log::warn!(
                "cwd_now: proc_pidinfo(pid={pid}) wrote {written} of {size} bytes — truncated result discarded"
            );
            return None;
        }

        // SAFETY: `written == size` implies the kernel fully populated the
        // buffer with a valid `proc_vnodepathinfo`.
        let info = unsafe { info.assume_init() };

        let ptr = info.pvi_cdir.vip_path.as_ptr() as *const libc::c_char;
        // SAFETY: the kernel guarantees `vip_path` holds a NUL-terminated
        // C string not exceeding `MAXPATHLEN` bytes when the syscall
        // succeeds with full size.
        let cstr = unsafe { CStr::from_ptr(ptr) };
        match cstr.to_str() {
            Ok(s) if !s.is_empty() => Some(std::path::PathBuf::from(s)),
            _ => None,
        }
    }

    /// Stub for other non-Linux platforms (Windows, BSDs).
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Scan the last 100 lines of terminal output for server/service patterns.
    /// Returns newly detected services (deduped against previously reported ports).
    /// Lock on `self.term` is held only for text extraction, then released before parsing.
    pub fn scan_output(&mut self) -> Vec<ServiceInfo> {
        let lines: Vec<String> = {
            // Read-only grid scan; unfair lock avoids queueing behind the
            // PTY reader thread on the periodic service-detection sweep.
            let term = self.term.lock_unfair();
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
                self.reported_ports.insert(info.port);
                results.push(info);
            }
        }

        results
    }

    pub fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        // US-002: any path through write_to_pty is genuine user input
        // (keystroke, paste, mouse report, IME commit, user scroll). Mark the
        // session user-initiated so a later exit closes the pane. Automated
        // protocol writes (focus reports, search RIS reset, OSC responses)
        // deliberately bypass this by calling `self.notifier.notify` directly.
        self.keyboard_input_sent
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.notify_or_buffer(input.into());
    }

    /// Send input to the live PTY, or queue it when the terminal is still
    /// display-only (US-012 pre-promotion window). The display-only notifier
    /// drops every write, so an auto-launch command (Agents view) or a
    /// keystroke typed before the off-thread fork resolved would otherwise be
    /// lost; [`promote`](Self::promote) flushes the queue in order. Bounded by
    /// [`MAX_PENDING_INPUT_BYTES`] so a never-promoted terminal can't grow it
    /// without bound.
    fn notify_or_buffer(&self, input: Cow<'static, [u8]>) {
        if self.notifier.0.is_pty() {
            self.notifier.notify(input);
            return;
        }
        if input.is_empty() {
            return;
        }
        if let Ok(mut pending) = self.pending_input.lock() {
            let queued: usize = pending.iter().map(|c| c.len()).sum();
            if queued + input.len() <= MAX_PENDING_INPUT_BYTES {
                pending.push(input);
            }
        }
    }

    /// US-002: write to the PTY WITHOUT marking the session user-initiated.
    /// For automated protocol writes (DEC 1004 focus in/out reports, search
    /// RIS reset) that must not flip `keyboard_input_sent` — otherwise a
    /// failed-spawn pane that merely gains focus would wrongly close on exit.
    pub fn write_to_pty_silent(&self, input: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(input);
    }

    /// US-002: whether a child exit should close the pane. A user-initiated
    /// session (any input was sent) always closes; otherwise only a clean exit
    /// (code 0) closes — a non-zero exit with no input is a spawn/launch
    /// failure and stays open so the exit overlay shows the code. Mirrors Zed's
    /// discriminator (crates/terminal/src/terminal.rs:2572-2576).
    pub fn should_close_on_exit(&self) -> bool {
        self.keyboard_input_sent
            .load(std::sync::atomic::Ordering::Relaxed)
            || self.exited == Some(0)
    }

    /// Extract scrollback as plain text (ANSI stripped) for session persistence.
    /// Caps at 4000 lines and 400,000 characters. Returns None if scrollback is empty.
    pub fn extract_scrollback(&self) -> Option<String> {
        Self::extract_scrollback_from(&self.term)
    }

    /// US-011: scrollback drain decoupled from `&self` so `save_session` can
    /// run it on a background thread against a cloned [`SharedTerm`] handle
    /// (the term mutex is `Send + Sync` — it is the only cross-thread state in
    /// the app) instead of holding the GPUI main thread. US-012's windowing
    /// keeps the lock bounded to the most-recent `MAX_LINES` rows.
    pub fn extract_scrollback_from(term: &SharedTerm) -> Option<String> {
        const MAX_LINES: usize = 4000;

        // Read-only scrollback drain for session persistence.
        let term = term.lock_unfair();
        let top = term.topmost_line();
        let bottom = term.bottommost_line();
        let cols = term.last_column();

        // US-012: window to the most-recent MAX_LINES *before* the loop so the
        // lock is never held while materializing the full history (scrollback
        // can be 10k lines — see DEFAULT_SCROLLBACK_LINES). Walk oldest→newest
        // from `bottom - MAX_LINES`, clamped to the topmost line. The drain
        // below stays as a defensive trim (trailing-empty removal can leave at
        // most MAX_LINES + 1 rows).
        let start = bottom.0.saturating_sub(MAX_LINES as i32).max(top.0);
        let mut lines: Vec<String> = Vec::with_capacity((bottom.0 - start + 1).max(0) as usize);
        let mut row = start;
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

        // Cap at MAX_CHARS, then trim to last complete line and strip any
        // partial ANSI escape at the boundary. Shared by both the background
        // save path and the synchronous quit path (`save_session_blocking`).
        cap_scrollback_at_char_boundary(&mut result, MAX_CHARS);

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Best-effort foreground command of this surface, for naming (US-001,
    /// prd-pane-context-bridge). Returns the command line (argv joined by
    /// spaces) of the process currently in the shell's foreground, or the
    /// shell itself when idle (which the namer maps to `shell`). `None` on
    /// platforms without a cheap lookup — callers fall back to the OSC title.
    #[cfg(target_os = "linux")]
    pub fn foreground_command(&self) -> Option<String> {
        // US-034: stale `child_pid` after exit may name an unrelated process.
        if self.exited.is_some() {
            return None;
        }
        // The most-recently-spawned child of the shell approximates the
        // foreground job for the common interactive case (one job at a time).
        // With no children the shell is idle → report its own comm so naming
        // resolves to `shell`.
        let children_path = format!("/proc/{pid}/task/{pid}/children", pid = self.child_pid);
        let target = std::fs::read_to_string(&children_path)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .last()
                    .and_then(|p| p.parse::<u32>().ok())
            })
            .unwrap_or(self.child_pid);
        read_proc_command(target)
    }

    /// macOS: the PTY's foreground process group (the job currently reading
    /// input) via `tcgetpgrp(master_fd)`, resolved to its executable basename
    /// with `proc_pidpath`. Falls back to the shell PID when idle so naming
    /// resolves to the shell. Returns the basename only (no full argv on macOS).
    /// `None` on any failure → caller falls back to the OSC title (US-019).
    #[cfg(target_os = "macos")]
    pub fn foreground_command(&self) -> Option<String> {
        // US-034: stale `child_pid`/master fd after exit may name an unrelated
        // process (or a reused fd reports an arbitrary pgid). Bail.
        if self.exited.is_some() {
            return None;
        }
        let pgid = self
            .pty_master_fd
            // SAFETY: `tcgetpgrp` is a pure query on a valid fd; returns -1 on
            // error (no controlling terminal yet), which we filter out.
            .map(|fd| unsafe { libc::tcgetpgrp(fd) })
            .filter(|&p| p > 0)
            .unwrap_or(self.child_pid as libc::c_int);
        // Reachable on the display-only path (`pty_master_fd` is `None` and
        // `child_pid` is 0): skip the FFI rather than call `proc_pidpath(0)`
        // (kernel pid 0 / swapper). On the live-PTY path `pgid` is always > 0.
        if pgid <= 0 {
            return None;
        }
        macos_exe_basename(pgid)
    }

    /// Windows: walk the process tree from `child_pid` to the deepest descendant
    /// (the most-recently-spawned leaf ≈ the foreground job under ConPTY) via a
    /// Toolhelp32 snapshot, then resolve its executable basename. Best-effort;
    /// `None` on any failure → caller falls back to the OSC title (US-019).
    #[cfg(windows)]
    pub fn foreground_command(&self) -> Option<String> {
        if self.child_pid == 0 {
            return None;
        }
        windows_foreground_command(self.child_pid)
    }

    /// Other platforms (BSDs, etc.): no cheap foreground lookup; naming
    /// degrades to the OSC title (cross-platform rule — graceful fallback).
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub fn foreground_command(&self) -> Option<String> {
        None
    }

    /// Search the scrollback for `pattern` (plain-text, case-insensitive) and
    /// return matching lines as `(grid_line, text)` pairs, deduped by line and
    /// capped at `max_matches`. The bool is `true` when the cap truncated the
    /// result. Backs the `surface.search` IPC method (US-004). The grid lock
    /// is held only for text extraction, never across an await.
    pub fn search_scrollback(
        &self,
        pattern: &str,
        max_matches: usize,
    ) -> (Vec<(i32, String)>, bool) {
        if pattern.is_empty() || max_matches == 0 {
            return (Vec::new(), false);
        }
        let result = crate::search::search_term(&self.term, pattern, false);

        // Collect unique line numbers in order of first appearance.
        let mut seen = std::collections::HashSet::new();
        let mut rows: Vec<i32> = Vec::new();
        let mut hit_cap = false;
        for m in &result.matches {
            let row = m.start.line.0;
            if seen.insert(row) {
                rows.push(row);
                if rows.len() >= max_matches {
                    hit_cap = true;
                    break;
                }
            }
        }

        // Extract each matched line's text under a single read lock.
        let term = self.term.lock_unfair();
        let cols = term.last_column();
        let out: Vec<(i32, String)> = rows
            .into_iter()
            .map(|row| {
                let text = term.bounds_to_string(
                    AlacPoint::new(GridLine(row), GridCol(0)),
                    AlacPoint::new(GridLine(row), cols),
                );
                (row, text.trim_end().to_string())
            })
            .collect();
        (out, hit_cap)
    }

    /// Strip every byte that could re-introduce a live escape/CSI/OSC/DCS
    /// sequence (or C1 control) from a single restored-scrollback line, so the
    /// documented "plain, ANSI stripped" invariant (schema.rs `scrollback`
    /// field) is *enforced* on the restore path — not merely assumed.
    ///
    /// A tampered/imported `session.json` can carry raw VT bytes in
    /// `surface.scrollback`; feeding them verbatim into the VTE processor
    /// allows single-line title-spoof / OSC8 clickable-link injection into the
    /// restored grid (phishing primitive). We drop the ESC introducer
    /// (`0x1b`), all other C0 control code points (keeping only `\t` — `\n`
    /// has already been consumed by the line split and `\r\n` is re-added by
    /// the caller), and the C1 control range (U+0080..=U+009F, which alacritty
    /// also treats as escape introducers). Pure string op: cross-platform, no
    /// OS/`libc` calls, no fallible step.
    fn sanitize_scrollback_line(line: &str) -> String {
        line.chars()
            .filter(|&c| {
                c == '\t'
                    || (!c.is_control()
                        // Reject C1 controls (0x80..=0x9f); `is_control`
                        // already covers them, but spell it out for intent.
                        && !('\u{80}'..='\u{9f}').contains(&c))
            })
            .collect()
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
            // Enforce the "plain, ANSI stripped" invariant: untrusted bytes
            // from a deserialized session must never reach the VTE parser as
            // live escape/CSI/OSC sequences (title-spoof / OSC8 link
            // injection). Sanitize before advancing.
            let sanitized = Self::sanitize_scrollback_line(line);
            let bytes = sanitized.as_bytes();
            if !bytes.is_empty() {
                processor.advance(&mut *term, bytes);
            }
            processor.advance(&mut *term, b"\r\n");
        }
    }
}

/// Cap `result` at `max_chars` bytes, cutting on a UTF-8 char boundary, then
/// trim to the last complete line and strip any partial ANSI escape at the cut.
///
/// U-001: `String::truncate` panics if the byte index is not on a char
/// boundary. Scrollback is built from real grid cells (CJK, emoji,
/// box-drawing are routine coding-agent output), so a raw `truncate(max_chars)`
/// panics whenever `max_chars` lands mid-codepoint. `floor_char_boundary`
/// rounds the index down to the nearest boundary first (no-op when already
/// aligned), so the result is always a valid `&str` of length ≤ `max_chars`.
pub(super) fn cap_scrollback_at_char_boundary(result: &mut String, max_chars: usize) {
    if result.len() > max_chars {
        let boundary = result.floor_char_boundary(max_chars);
        result.truncate(boundary);
        // `rfind('\n')` always returns a char boundary, so this second
        // truncate is already safe.
        if let Some(last_newline) = result.rfind('\n') {
            result.truncate(last_newline);
        }
        strip_partial_ansi_tail(result);
    }
}

/// Strip any partial ANSI escape sequence from the end of a truncated string.
///
/// Scans backward from the end for an ESC (`\x1b`) that starts a CSI (`\x1b[`),
/// OSC (`\x1b]`), or DCS (`\x1bP`) sequence. If the sequence is unterminated
/// (no final byte in the valid range), it is removed. Plain text strings with
/// no ESC bytes are returned unmodified — truncation is identical to naive splitting.
pub(super) fn strip_partial_ansi_tail(text: &mut String) {
    let Some(esc_pos) = text.rfind('\x1b') else {
        return; // No escape sequences at all
    };

    let tail = &text[esc_pos..];
    let bytes = tail.as_bytes();

    if bytes.len() < 2 {
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
            // Other ESC sequences (SS2, SS3, etc.) are 2 bytes — complete as-is.
        }
    }
}

/// Compute the PaneFlow IPC socket path, delegating to `runtime_paths` so
/// the fallback chain stays in sync with `ipc::socket_path`.
fn paneflow_socket_path() -> Option<String> {
    crate::runtime_paths::socket_path().map(|p| p.display().to_string())
}

/// US-009 — extract the embedded AI-hook binaries into the user's cache
/// dir, then expose that dir via `PANEFLOW_BIN_DIR` and prepend it to
/// the child shell's `PATH`.
///
/// Silent-fail: any error (extraction IO failure, unresolvable
/// `cache_dir`) is logged at `warn` and then swallowed so the terminal
/// opens normally without the AI-hook loader for this session. PRD
/// constraint C4 mandates the terminal must never fail to open because
/// of AI-hook wiring.
///
/// Factored out of `TerminalState::new` so the helper is independently
/// testable — the extraction side-effect lives in `ai_hooks::extract`
/// (already unit-tested in US-008); this glue only layers the env
/// mutations on top of a returned `PathBuf`.
fn inject_ai_hook_env(env: &mut std::collections::HashMap<String, String>) {
    let bin_dir = match crate::ai_hooks::extract::ensure_binaries_extracted() {
        Ok(p) => p,
        Err(e) => {
            // `{e:#}` emits the full anyhow context chain (each
            // `.with_context()` frame) rather than just the outermost
            // message — crucial for diagnosing cache-dir permission
            // errors that arrive with a useful inner IO error.
            log::warn!(
                "paneflow: AI-hook binary extraction failed ({e:#}); sidebar loader will not activate for this terminal session"
            );
            return;
        }
    };

    // `PANEFLOW_BIN_DIR` is the source-of-truth the shim uses for its
    // self-exclusion PATH walk (US-004). Set it even in the unlikely
    // event the PATH-prepend below fails, so the shim can still
    // identify its own dir if a later code path routes into it.
    env.insert("PANEFLOW_BIN_DIR".into(), bin_dir.display().to_string());

    prepend_bin_dir_to_path(env, &bin_dir);
}

/// Prepend `bin_dir` to `env["PATH"]` (or to the process `PATH` if the
/// env map does not yet carry one). Cross-platform: uses
/// `std::env::join_paths`, which emits `:` on Unix and `;` on Windows.
///
/// If join-paths fails (e.g. a `PATH` entry contains a platform
/// separator byte — invalid but physically possible), logs a warning
/// and leaves the env map unchanged. Better "no prepend" than "broken
/// PATH".
fn prepend_bin_dir_to_path(
    env: &mut std::collections::HashMap<String, String>,
    bin_dir: &std::path::Path,
) {
    // Order of precedence: explicit map entry first, then process env.
    // `setup_shell_integration` (shell.rs) does not set PATH, so in
    // practice this always falls through to the process PATH — but the
    // explicit-map branch makes the helper reusable and keeps tests
    // decoupled from the process environment.
    let existing: Option<std::ffi::OsString> = env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));

    let mut components: Vec<std::path::PathBuf> = vec![bin_dir.to_path_buf()];
    // Guard against an empty `PATH` string: on Unix, `split_paths("")`
    // yields a single `PathBuf::from("")` which `execvp` resolves as the
    // current working directory — that would silently put `.` on the
    // child's PATH (a classic shell-injection surface). Treat empty and
    // absent identically.
    if let Some(existing) = existing.as_deref()
        && !existing.is_empty()
    {
        components.extend(std::env::split_paths(existing));
    }

    match std::env::join_paths(components) {
        Ok(joined) => {
            // `join_paths` always produces valid UTF-8 when all inputs
            // were UTF-8 PathBufs + an OsString PATH — on all three
            // supported OSes, PATH is conventionally UTF-8 so the
            // `to_string_lossy` round-trip is safe. If a real-world PATH
            // entry contains non-UTF-8 bytes, we lose those in the
            // lossy conversion — but the env map is keyed on
            // `HashMap<String, String>` to begin with, so this is a
            // pre-existing constraint inherited from
            // `PtyBackend::spawn`, not introduced here.
            env.insert("PATH".into(), joined.to_string_lossy().into_owned());
        }
        Err(e) => {
            log::warn!(
                "paneflow: could not prepend AI-hook bin dir {} to PATH: {e}",
                bin_dir.display()
            );
        }
    }
}

/// True if `key` names a dynamic-loader-influencing environment variable that
/// an untrusted source (an imported `session.json` surface env, or the global
/// `terminal.env` config) must NOT be allowed to inject into a spawned child:
/// `LD_PRELOAD` / `LD_LIBRARY_PATH` / `LD_AUDIT` and any `LD_*` on Linux, plus
/// any `DYLD_*` on macOS. Letting these through is an RCE vector — the operator
/// treats imported sessions as untrusted, and the child is always the
/// configured shell. The match is case-sensitive on purpose: the unix loaders
/// only honour the exact upper-case spelling, so a lower-case `ld_preload` is
/// inert and need not be dropped. On Windows these names are meaningless, so the
/// check is a harmless no-op there (the caller has already upper-cased `key`).
fn is_loader_influencing_env_key(key: &str) -> bool {
    key.starts_with("LD_") || key.starts_with("DYLD_")
}

/// True if `key` is a well-formed environment variable name safe to insert into
/// a child env block: non-empty and free of `=` and NUL, which would otherwise
/// corrupt the name/value framing. Charset is intentionally NOT restricted to a
/// strict POSIX set — legitimate user vars (e.g. `ANTHROPIC_API_KEY`) are
/// already all-caps `[A-Z0-9_]`, and over-restricting would silently drop valid
/// keys.
fn is_valid_env_name(key: &str) -> bool {
    !key.is_empty() && !key.contains('=') && !key.contains('\0')
}

/// Assemble the child PTY environment: PaneFlow identity vars, explicit TERM /
/// locale / terminal-program identification, the AI-hook PATH prepend, and the
/// user-env merge (a user var wins on collision EXCEPT the protected keys
/// PaneFlow owns). Pure except for `inject_ai_hook_env` staging the shim
/// binaries, so the env contract stays unit-testable now that the mockable
/// `PtyBackend::spawn` seam is gone (EP-002 US-004). Mirrors Zed's
/// `insert_zed_terminal_env`.
fn assemble_pty_env(
    mut env: std::collections::HashMap<String, String>,
    workspace_id: u64,
    surface_id: u64,
    user_env: Option<std::collections::HashMap<String, String>>,
) -> std::collections::HashMap<String, String> {
    // PaneFlow identity vars (AI-hook + MCP bridge integration).
    env.insert("PANEFLOW_WORKSPACE_ID".into(), workspace_id.to_string());
    env.insert("PANEFLOW_SURFACE_ID".into(), surface_id.to_string());
    if let Some(socket_path) = paneflow_socket_path() {
        env.insert("PANEFLOW_SOCKET_PATH".into(), socket_path);
    }

    // Explicit TERM so TUI apps detect capabilities correctly.
    env.insert("TERM".into(), "xterm-256color".into());

    // Ensure a UTF-8 locale in minimal environments (containers, etc.).
    if std::env::var("LANG").map_or(true, |v| v.is_empty()) {
        env.insert("LANG".into(), "en_US.UTF-8".into());
    }

    // Standard terminal identification for capability detection.
    env.insert("TERM_PROGRAM".into(), "paneflow".into());
    env.insert(
        "TERM_PROGRAM_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    env.insert("COLORTERM".into(), "truecolor".into());

    // Reset SHLVL so the child shell starts fresh at 1. alacritty's `tty`
    // inherits the parent environment (no `env_clear`), so unlike the old
    // portable-pty `env_remove("SHLVL")` we must actively override the value
    // PaneFlow itself inherited (typically >= 2 when launched from a terminal),
    // which otherwise breaks nested-shell prompt detection (oh-my-zsh subshell
    // banner, fish $SHLVL gating). "0" makes the shell initialize it to 1.
    env.insert("SHLVL".into(), "0".into());

    // Cross-platform AI-hook PATH-prepend: stage the embedded shim binaries and
    // prepend their dir to `$PATH` so `claude`/`codex` route through the shim.
    // Silent-fail (the terminal still opens). Sets `PANEFLOW_BIN_DIR`.
    inject_ai_hook_env(&mut env);

    // Merge user-supplied env on top, EXCEPT the protected keys PaneFlow owns:
    // TERM/COLORTERM drive capability detection; the PANEFLOW_* identity vars
    // are how the MCP bridge and the AI-hook shim find PaneFlow — letting a user
    // clobber them would silently break those features.
    if let Some(user_vars) = user_env {
        const PROTECTED: &[&str] = &[
            "TERM",
            "COLORTERM",
            "PANEFLOW_WORKSPACE_ID",
            "PANEFLOW_SURFACE_ID",
            "PANEFLOW_SOCKET_PATH",
            "PANEFLOW_BIN_DIR",
        ];
        for (k, v) in user_vars {
            // Windows env names are case-insensitive; normalise so a user
            // `Path` cannot shadow inherited `PATH` and the protected-key check
            // is not bypassed by casing.
            #[cfg(windows)]
            let k = k.to_uppercase();
            // Reject malformed env names (empty / `=` / NUL) and drop
            // dynamic-loader-influencing keys (LD_* / DYLD_*) outright: an
            // imported `session.json` surface env or the global `terminal.env`
            // is untrusted, and these inject a bundled `.so` into the spawned
            // shell (RCE). `PATH` is deliberately still overridable here (a
            // documented US-014 use case) — it shadows the AI-hook prepend but
            // is not a loader-preload vector; revisit if untrusted import flows
            // widen.
            if !is_valid_env_name(&k) || is_loader_influencing_env_key(&k) {
                continue;
            }
            if PROTECTED.contains(&k.as_str()) {
                continue;
            }
            env.insert(k, v);
        }
    }

    env
}

/// Send SIGTERM to the child's process group, guarded by a no-op signal
/// probe so a dead or empty group is a harmless no-op. Returns true if
/// SIGTERM was delivered. Factored out of `Drop` so the graceful-shutdown
/// step is unit-testable (US-001).
#[cfg(unix)]
fn terminate_process_group(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill(-pid, 0) probes the group without delivering a signal;
    // kill(-pid, SIGTERM) signals every member. Both are FFI-safe with a
    // validated pid > 0.
    unsafe {
        if libc::kill(-pid, 0) == 0 {
            libc::kill(-pid, libc::SIGTERM) == 0
        } else {
            false
        }
    }
}

/// EP-002 US-005: format a numeric signal (from alacritty's native
/// `ExitStatus::signal()`) as "N (Name)" for the exit overlay, e.g.
/// "11 (Segmentation fault)". The name comes from `strsignal`; the number is
/// authoritative (no reversal). Falls back to "signal N" when `strsignal` is
/// null for the signal.
#[cfg(unix)]
fn format_signal(sig: i32) -> String {
    // SAFETY: strsignal is a pure query; the returned C string is copied
    // immediately via CStr before any further libc call.
    let name = unsafe {
        let p = libc::strsignal(sig);
        if p.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    };
    match name {
        Some(n) => format!("{sig} ({n})"),
        None => format!("signal {sig}"),
    }
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        self.notifier.0.shutdown();

        // US-034: close the dup'd PTY master fd we own (macOS). Done exactly
        // once here — the fd was duplicated at spawn so `tcgetpgrp` stayed
        // valid for this session's lifetime independent of the EventLoop's
        // own copy.
        #[cfg(target_os = "macos")]
        if let Some(fd) = self.pty_master_fd.take() {
            // SAFETY: `fd` is our owned dup; close it once.
            unsafe {
                libc::close(fd);
            }
        }

        // Grace period + force-kill: if the child process ignores the PTY
        // master close signal (SIGHUP on Unix, ClosePseudoConsole on Windows),
        // force-kill it after 100ms.
        //
        // Scheduling: prefer the GPUI `background_executor` (Zed parity:
        // `crates/terminal/src/terminal.rs:2451-2457`) so the kill timer
        // lives under the GPUI runtime and gets cleanly torn down with
        // the app. Tests / display-only paths have no executor wired and
        // fall back to a detached OS thread (safe but un-trackable).
        let executor = self.background_executor.clone();

        #[cfg(unix)]
        {
            let pid = self.child_pid as i32;
            // US-034: skip the kill ladder entirely once the child has exited.
            // `child_pid` may have been reused by the OS by now, and signaling
            // a reused PGID would terminate an unrelated process group — the
            // synchronous SIGTERM below has the same PID-reuse window as the
            // delayed SIGKILL. An already-exited child has nothing to kill.
            if pid > 0 && self.exited.is_none() {
                // US-001: graceful shutdown ladder — send SIGTERM to the group
                // synchronously FIRST so agents/shells run their TERM handlers
                // (state checkpoint, HISTFILE flush) before the 100ms-grace
                // SIGKILL escalation below. Mirrors Zed's
                // terminate_child_process() -> 100ms -> kill_child_process()
                // (crates/terminal/src/terminal.rs:2697-2704, pty_info.rs:142-151).
                terminate_process_group(pid);

                let kill = move || {
                    // Target the entire process group (`-pid`) so any
                    // sub-process the shell forked (cargo build, npm dev,
                    // long-running scripts) dies with the shell instead of
                    // becoming an orphan reparented to PID 1. portable-pty
                    // calls `setsid()` on the child (unix.rs:220), so
                    // `child_pid` is both the PID and the PGID of the
                    // session leader — `kill(-pgid, sig)` is the canonical
                    // POSIX idiom to signal every process in that group.
                    //
                    // SAFETY: libc::kill(-pid, 0) is a no-op signal probe;
                    // libc::kill(-pid, SIGKILL) signals every member of the
                    // process group. Both calls are FFI-safe with a validated
                    // pid > 0 captured by value into this closure.
                    unsafe {
                        if libc::kill(-pid, 0) == 0 {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                };
                match executor {
                    Some(bg) => {
                        bg.spawn(async move {
                            smol::Timer::after(std::time::Duration::from_millis(100)).await;
                            kill();
                        })
                        .detach();
                    }
                    None => {
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            kill();
                        });
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
            };
            // SYNCHRONIZE access right is required for WaitForSingleObject on the
            // returned handle; PROCESS_TERMINATE alone makes WaitForSingleObject
            // return WAIT_FAILED. Value mirrors winnt.h (0x0010_0000). Declared
            // locally to avoid pulling the Win32_Storage_FileSystem feature flag.
            const SYNCHRONIZE: u32 = 0x0010_0000;

            let pid = self.child_pid;
            if pid != 0 {
                // US-001 asymmetry: Windows console processes have no
                // SIGTERM-equivalent graceful signal. TerminateProcess is a
                // hard kill and serves as the escalation; there is no Windows
                // mirror of the Unix synchronous-SIGTERM grace step above.
                let terminate = move || {
                    // SAFETY: Win32 handles are owned within this closure;
                    // we always CloseHandle before returning each branch.
                    unsafe {
                        let handle = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, 0, pid);
                        if handle.is_null() {
                            log::debug!(
                                "paneflow: OpenProcess({pid}) returned NULL (child likely already exited)"
                            );
                            return;
                        }
                        if TerminateProcess(handle, 1) == 0 {
                            log::warn!("paneflow: TerminateProcess({pid}) failed");
                            let _ = CloseHandle(handle);
                            return;
                        }
                        let wait = WaitForSingleObject(handle, 5000);
                        if wait != WAIT_OBJECT_0 {
                            log::warn!(
                                "paneflow: WaitForSingleObject({pid}) returned {wait:#x} (expected WAIT_OBJECT_0)"
                            );
                        }
                        let _ = CloseHandle(handle);
                    }
                };
                match executor {
                    Some(bg) => {
                        bg.spawn(async move {
                            smol::Timer::after(std::time::Duration::from_millis(100)).await;
                            terminate();
                        })
                        .detach();
                    }
                    None => {
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            terminate();
                        });
                    }
                }
            }
        }
    }
}

/// Read a process's command line from `/proc/<pid>/cmdline`, falling back to
/// `/proc/<pid>/comm` when the cmdline is empty (kernel thread / zombie).
#[cfg(target_os = "linux")]
fn read_proc_command(pid: u32) -> Option<String> {
    if let Ok(bytes) = std::fs::read(format!("/proc/{pid}/cmdline"))
        && let Some(cmd) = command_from_cmdline(&bytes)
    {
        return Some(cmd);
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let trimmed = comm.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Parse a `/proc/<pid>/cmdline` blob (NUL-separated argv) into a space-joined
/// command string. Pure, so the parsing is unit-testable without `/proc`.
#[cfg(target_os = "linux")]
fn command_from_cmdline(bytes: &[u8]) -> Option<String> {
    let parts: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// macOS (US-019): resolve a pid/pgid to its executable basename via
/// `proc_pidpath`, mirroring the `cwd_now` FFI style. `None` on any error
/// (process gone, SIP/sandbox denial — logged at debug, never panics).
#[cfg(target_os = "macos")]
fn macos_exe_basename(pid: libc::c_int) -> Option<String> {
    use std::ffi::CStr;
    use std::os::raw::c_void;

    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buf` is stack-allocated, zeroed, and sized exactly to
    // PROC_PIDPATHINFO_MAXSIZE; it is only read when the call reports success.
    let written =
        unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if written <= 0 {
        log::debug!("foreground_command: proc_pidpath(pid={pid}) returned {written}");
        return None;
    }
    // SAFETY: `written > 0` and the buffer is zero-filled, so a NUL terminator
    // is guaranteed within range.
    let path = unsafe { CStr::from_ptr(buf.as_ptr() as *const libc::c_char) }
        .to_str()
        .ok()?;
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
}

/// Windows (US-019): walk the process tree from `root_pid` to its deepest
/// descendant (the most-recently-spawned leaf ≈ the foreground job under
/// ConPTY) via a Toolhelp32 snapshot, then resolve that process's executable
/// basename with `QueryFullProcessImageNameW`. Best-effort (Windows recycles
/// PIDs); `None` on any failure → caller falls back to the OSC title.
#[cfg(windows)]
fn windows_foreground_command(root_pid: u32) -> Option<String> {
    use std::mem;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    // SAFETY: Win32 call; the returned snapshot handle is closed below.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }

    // Collect (pid, parent_pid) for every process in the snapshot.
    let mut entries: Vec<(u32, u32)> = Vec::with_capacity(256);
    let mut entry: PROCESSENTRY32W = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: `snap` is valid; `entry` is correctly sized and zeroed.
    if unsafe { Process32FirstW(snap, &mut entry) } != 0 {
        loop {
            entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: same invariants; iterates until the snapshot is exhausted.
            if unsafe { Process32NextW(snap, &mut entry) } == 0 {
                break;
            }
        }
    }
    // SAFETY: `snap` is a valid handle obtained above.
    unsafe { CloseHandle(snap) };

    // Descend parent → child from the shell to the deepest leaf. At each level
    // pick the child with the HIGHEST pid: Windows assigns pids in increasing
    // order, so the highest pid is the most-recently-spawned child — the best
    // available proxy for the foreground job under ConPTY (which, unlike Unix,
    // has no `tcgetpgrp`). `.find()` would instead return snapshot-enumeration
    // order (oldest-first), naming a stale background job in a multi-child
    // shell. A visited set guards against a cycle from pid recycling.
    let mut current = root_pid;
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current) {
        match entries
            .iter()
            .filter(|(_, parent)| *parent == current)
            .max_by_key(|(pid, _)| *pid)
        {
            Some((child, _)) => current = *child,
            None => break,
        }
    }

    // PROCESS_QUERY_LIMITED_INFORMATION needs no elevation (Vista+).
    // SAFETY: Win32 call; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, current) };
    if handle.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `handle` is valid; `buf`/`len` are initialised; the call writes at
    // most `len` u16s and updates `len` to the count written.
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    };
    // SAFETY: `handle` was obtained from OpenProcess above.
    unsafe { CloseHandle(handle) };
    if ok == 0 || len == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn platform_sep() -> char {
        if cfg!(windows) { ';' } else { ':' }
    }

    #[test]
    fn display_only_sender_is_inert() {
        // US-011: the DisplayOnly variant of TerminalType has no EventLoop, so
        // write/resize/shutdown are dropped (no panic) and `is_pty` is false.
        // This is the spawn-failure fallback's write side — input must never
        // reach a child that doesn't exist.
        let s = PtySender::display_only();
        assert!(!s.is_pty());
        s.write(b"echo hi\n".as_slice().into());
        s.resize(AlacWindowSize {
            num_cols: 80,
            num_lines: 24,
            cell_width: 0,
            cell_height: 0,
        });
        s.shutdown();
    }

    #[test]
    fn new_display_only_terminal_has_no_pty() {
        // US-011: the spawn-failure fallback builds a DisplayOnly terminal; its
        // notifier must report no PTY so the input path drops bytes.
        let state = TerminalState::new_display_only(24, 80);
        assert!(!state.notifier.0.is_pty());
    }

    #[test]
    fn new_pending_terminal_starts_display_only_then_promotes_conceptually() {
        // US-012: a pending terminal is display-only (no PTY) until promoted.
        // (Promotion needs a real EventLoop channel, exercised by the live
        // `eventloop_pty_echoes_input_into_grid` smoke via the synchronous
        // `new`, which composes new_pending + open_pty_and_eventloop + promote.)
        let (state, _events_tx) = TerminalState::new_pending(80, 24);
        assert!(!state.notifier.0.is_pty());
        assert_eq!(state.child_pid, 0);
    }

    #[test]
    fn write_to_pty_buffers_input_while_display_only() {
        // US-012 regression: the Agents-view "New thread" picker writes the
        // launch command the instant a thread mounts — before the off-thread
        // fork promotes the PTY. The display-only notifier drops writes, so
        // without this queue the command (e.g. `claude`) is lost and the
        // terminal opens to a bare shell. `write_to_pty` must buffer instead.
        let (state, _events_tx) = TerminalState::new_pending(80, 24);
        assert!(!state.notifier.0.is_pty());
        state.write_to_pty(b"claude\r".to_vec());
        let queued = state
            .pending_input
            .lock()
            .expect("pending_input lock")
            .clone();
        assert_eq!(queued, vec![Cow::from(b"claude\r".to_vec())]);
    }

    #[test]
    fn pending_input_is_bounded() {
        // A terminal that never promotes (spawn failure) must not accumulate
        // input without bound: writes past the cap are dropped, not queued.
        let (state, _events_tx) = TerminalState::new_pending(80, 24);
        let chunk = vec![b'x'; 8 * 1024];
        for _ in 0..16 {
            state.write_to_pty(chunk.clone());
        }
        let queued: usize = state
            .pending_input
            .lock()
            .expect("pending_input lock")
            .iter()
            .map(|c| c.len())
            .sum();
        assert!(
            queued <= MAX_PENDING_INPUT_BYTES,
            "buffered {queued} bytes exceeds the {MAX_PENDING_INPUT_BYTES} cap"
        );
    }

    #[cfg(unix)]
    #[test]
    fn promote_flushes_buffered_input_into_grid() {
        // US-012 end-to-end: input written while display-only must reach the
        // child after promotion. Mirrors the synchronous `new` composition
        // (new_pending + open_pty_and_eventloop + promote) but injects a write
        // *between* new_pending and promote — the exact Agents-view ordering.
        let params = TerminalState::resolve_spawn_params(None, 1, 1, Some((80, 24)), None);
        let (mut state, events_tx) = TerminalState::new_pending(params.cols, params.rows);
        // Buffered while display-only — the live notifier does not exist yet.
        state.write_to_pty(b"echo PANEFLOW_FLUSH_OK\n".to_vec());
        assert!(!state.notifier.0.is_pty());

        let term = state.term.clone();
        let spawned = TerminalState::open_pty_and_eventloop(params, term, events_tx, None)
            .expect("US-012: open a PTY-backed terminal via tty::new + EventLoop");
        state.promote(spawned);
        assert!(state.notifier.0.is_pty());

        let mut found = false;
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            state.sync();
            if grid_to_string(&state.term).contains("PANEFLOW_FLUSH_OK") {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "US-012: input buffered before promotion never reached the shell"
        );
    }

    #[test]
    fn resolve_spawn_params_honors_initial_size() {
        // US-012: the cheap, render-thread-safe half of a spawn picks up the
        // requested grid size (and the 120x40 default when unspecified).
        let p = TerminalState::resolve_spawn_params(None, 1, 1, Some((100, 30)), None);
        assert_eq!((p.cols, p.rows), (100, 30));
        let d = TerminalState::resolve_spawn_params(None, 1, 1, None, None);
        assert_eq!((d.cols, d.rows), (120, 40));
    }

    #[cfg(unix)]
    #[test]
    fn capture_foreground_signal_mask_succeeds_on_unix() {
        // US-012: the foreground mask snapshot must succeed on the main thread
        // so the off-thread spawn can hand it to the child (Ctrl-C parity).
        assert!(capture_foreground_signal_mask().is_some());
    }

    #[test]
    fn cursor_shape_maps_hollow_to_hollow_block() {
        // US-007: config shapes map to renderer (vte) shapes; the config's
        // `Hollow` maps to the renderer's `HollowBlock`.
        use alacritty_terminal::vte::ansi::CursorShape;
        use paneflow_config::schema::CursorShapeConfig as C;
        assert_eq!(map_cursor_shape(C::Block), CursorShape::Block);
        assert_eq!(map_cursor_shape(C::Beam), CursorShape::Beam);
        assert_eq!(map_cursor_shape(C::Underline), CursorShape::Underline);
        assert_eq!(map_cursor_shape(C::Hollow), CursorShape::HollowBlock);
    }

    #[test]
    fn prepend_puts_bin_dir_first_and_preserves_existing_entries() {
        let mut env: HashMap<String, String> = HashMap::new();
        let sep = platform_sep();
        env.insert("PATH".into(), format!("/usr/bin{sep}/usr/local/bin"));

        let bin_dir = PathBuf::from("/home/u/.cache/paneflow/bin/0.2.6");
        prepend_bin_dir_to_path(&mut env, &bin_dir);

        let joined = env.get("PATH").expect("PATH set by helper");
        let components: Vec<PathBuf> = std::env::split_paths(joined).collect();
        assert_eq!(
            components.first(),
            Some(&bin_dir),
            "US-009 AC: bin_dir must be first on PATH; got {components:?}"
        );
        assert!(
            components.iter().any(|p| p == Path::new("/usr/bin")),
            "US-009: original PATH entries must be preserved; got {components:?}"
        );
        assert!(
            components.iter().any(|p| p == Path::new("/usr/local/bin")),
            "US-009: original PATH entries must be preserved; got {components:?}"
        );
    }

    #[test]
    fn prepend_inserts_bin_dir_even_when_env_path_absent() {
        // AC: "If env map has no PATH, helper still sets PATH so the
        // child inherits the shim dir rather than silently no-op."
        let mut env: HashMap<String, String> = HashMap::new();
        let bin_dir = PathBuf::from("/tmp/paneflow-bins");
        prepend_bin_dir_to_path(&mut env, &bin_dir);

        let joined = env.get("PATH").expect("PATH set by helper");
        let components: Vec<PathBuf> = std::env::split_paths(joined).collect();
        assert_eq!(
            components.first(),
            Some(&bin_dir),
            "US-009: bin_dir must be first on PATH in the no-prior-PATH case"
        );
    }

    #[test]
    fn prepend_uses_platform_separator() {
        // Round-trip invariant: split_paths(join_paths(X)) == X. This
        // implicitly tests that `;` on Windows / `:` on Unix is handled
        // correctly — we do not assert the raw bytes because that
        // would hardcode per-OS expectations.
        let mut env: HashMap<String, String> = HashMap::new();
        let sep = platform_sep();
        env.insert("PATH".into(), format!("/a{sep}/b{sep}/c"));
        let bin_dir = PathBuf::from("/z");
        prepend_bin_dir_to_path(&mut env, &bin_dir);

        let joined = env.get("PATH").unwrap();
        let components: Vec<PathBuf> = std::env::split_paths(joined).collect();
        assert_eq!(
            components,
            vec![
                PathBuf::from("/z"),
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ],
            "US-009: split_paths(join_paths(...)) must round-trip on all platforms"
        );
    }

    #[test]
    fn prepend_treats_empty_path_as_absent() {
        // An empty `PATH` is not absent — `split_paths("")` on Unix
        // yields one `PathBuf::from("")` component that `execvp`
        // resolves as the CWD. We must NOT inherit that phantom entry
        // onto the child's PATH (shell-injection surface).
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("PATH".into(), String::new());
        let bin_dir = PathBuf::from("/z");
        prepend_bin_dir_to_path(&mut env, &bin_dir);

        let joined = env.get("PATH").expect("PATH set by helper");
        let components: Vec<PathBuf> = std::env::split_paths(joined).collect();
        assert!(
            !components.iter().any(|p| p.as_os_str().is_empty()),
            "US-009 hardening: empty PATH must not yield a phantom CWD entry; got {components:?}"
        );
        assert_eq!(
            components.first(),
            Some(&bin_dir),
            "US-009: bin_dir must still be first when empty PATH is treated as absent"
        );
    }

    // -----------------------------------------------------------------
    // US-003 — exit-status correctness (real code, first-write-wins).
    // -----------------------------------------------------------------

    #[test]
    fn child_exit_records_real_code_not_sentinel() {
        // US-003 AC: a real child exit code must round-trip into `exited`,
        // not the -1 fallback. The status is built the same way
        // `pty_reader_loop` builds it from `child.wait()` per platform, so
        // this exercises the Windows path on the Windows CI leg and the Unix
        // path elsewhere.
        let mut state = TerminalState::new_display_only(24, 80);
        assert!(state.exited.is_none(), "fresh terminal has no exit code");

        #[cfg(unix)]
        let status: std::process::ExitStatus =
            std::os::unix::process::ExitStatusExt::from_raw(42 << 8);
        #[cfg(windows)]
        let status: std::process::ExitStatus =
            std::os::windows::process::ExitStatusExt::from_raw(42u32);

        #[cfg(any(unix, windows))]
        {
            state.process_event(AlacEvent::ChildExit(status));
            assert_eq!(
                state.exited,
                Some(42),
                "US-003: the real exit code must be recorded, not -1"
            );
        }
    }

    #[test]
    fn exit_fallback_does_not_clobber_real_child_exit_code() {
        // US-003 AC: first-write-wins. A bare `Exit` (EOF, no status) must
        // never overwrite a real code already recorded by `ChildExit`.
        let mut state = TerminalState::new_display_only(24, 80);

        #[cfg(unix)]
        let status: std::process::ExitStatus =
            std::os::unix::process::ExitStatusExt::from_raw(1 << 8);
        #[cfg(windows)]
        let status: std::process::ExitStatus =
            std::os::windows::process::ExitStatusExt::from_raw(1u32);

        #[cfg(any(unix, windows))]
        {
            state.process_event(AlacEvent::ChildExit(status));
            state.process_event(AlacEvent::Exit);
            assert_eq!(
                state.exited,
                Some(1),
                "US-003: Exit must not clobber the real ChildExit code"
            );
        }
    }

    // -----------------------------------------------------------------
    // US-002 — keep pane open on launch failure (keyboard_input_sent).
    // -----------------------------------------------------------------

    #[test]
    fn close_on_exit_discriminator_covers_both_branches() {
        // US-002 AC: clean exit (code 0) closes even with no input.
        let mut clean = TerminalState::new_display_only(24, 80);
        clean.exited = Some(0);
        assert!(
            clean.should_close_on_exit(),
            "US-002: a clean exit (code 0) must close the pane"
        );

        // Non-zero exit with NO user input = spawn/launch failure → stays open
        // so the exit overlay can render the code.
        let mut failed = TerminalState::new_display_only(24, 80);
        failed.exited = Some(127);
        assert!(
            !failed.should_close_on_exit(),
            "US-002: a non-zero exit with no input must keep the pane open"
        );

        // ...but once the user has interacted, ANY exit closes.
        failed.write_to_pty(b"x".as_slice());
        assert!(
            failed.should_close_on_exit(),
            "US-002: after user input, a non-zero exit must close the pane"
        );
    }

    #[test]
    fn write_to_pty_marks_user_input_but_fresh_state_does_not() {
        // US-002: a fresh terminal has not received user input; write_to_pty
        // flips the flag. (Automated writes use notifier.notify and are tested
        // implicitly by the discriminator staying false here.)
        let state = TerminalState::new_display_only(24, 80);
        assert!(
            !state
                .keyboard_input_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            "fresh terminal must report no user input"
        );
        state.write_to_pty(b"a".as_slice());
        assert!(
            state
                .keyboard_input_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            "write_to_pty must mark the session user-initiated"
        );
    }

    // -----------------------------------------------------------------
    // US-001 — graceful teardown sends SIGTERM before SIGKILL.
    // -----------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn terminate_process_group_delivers_sigterm_and_is_honored() {
        // US-001 AC: the process group receives SIGTERM (not a hard SIGKILL).
        // The child is its own session/group leader (setsid) and traps SIGTERM
        // to exit 42; a SIGKILL would instead show signal 9 with no exit code.
        // Proving the trap ran proves SIGTERM was delivered to the group — and
        // by construction `Drop` sends it synchronously *before* scheduling the
        // 100ms-grace SIGKILL.
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let mut cmd = Command::new("sh");
        cmd.args(["-c", "trap 'exit 42' TERM; sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid() runs in the forked child before exec; it detaches
        // the child into its own session/group so kill(-pid, ...) targets
        // exactly this group, with no shared-state hazard.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("spawn test child");
        let pid = child.id() as i32;

        // Let the shell install its TERM trap before we signal.
        std::thread::sleep(Duration::from_millis(150));

        assert!(
            terminate_process_group(pid),
            "US-001: SIGTERM must be delivered to the live process group"
        );

        // The trap exits 42 well within the 100ms grace window; poll for exit
        // with a 5s ceiling so a regression fails fast instead of hanging.
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().expect("try_wait child") {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("US-001: child did not exit after SIGTERM within 5s");
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        assert_eq!(
            status.code(),
            Some(42),
            "US-001: child must exit via its SIGTERM handler (42), not be SIGKILLed (signal={:?})",
            status.signal()
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminate_process_group_is_noop_for_dead_or_invalid_group() {
        // US-001 AC (unhappy path): an empty/invalid group must be a harmless
        // no-op guarded by the kill(-pid, 0) probe — no panic, returns false.
        assert!(
            !terminate_process_group(0),
            "pid 0 must be rejected (would signal the caller's own group)"
        );
        assert!(
            !terminate_process_group(-5),
            "negative pid must be rejected"
        );
        // A very high pid is almost certainly not a live group; the probe
        // returns ESRCH so SIGTERM is never sent.
        assert!(
            !terminate_process_group(0x7FFF_FFF0),
            "non-existent group must be a no-op, not a panic"
        );
    }

    // -----------------------------------------------------------------
    // Env assembly contract. EP-002 US-004 removed the mockable
    // `PtyBackend::spawn` seam (the IO core now opens the PTY via alacritty's
    // `tty::new`), so the env that the child inherits is asserted directly
    // against the pure `assemble_pty_env`.
    // -----------------------------------------------------------------

    #[test]
    fn pty_spawn_injects_paneflow_bin_dir_and_prepends_path() {
        // Skip where the cache dir is unresolvable — the helper silent-fails
        // (correct behavior), but then there's nothing to assert on.
        if dirs::cache_dir().is_none() {
            eprintln!("skip: dirs::cache_dir() unresolvable in this environment");
            return;
        }

        let env = assemble_pty_env(HashMap::new(), 7, 3, None);

        let bin_dir = env
            .get("PANEFLOW_BIN_DIR")
            .expect("US-009 AC: PANEFLOW_BIN_DIR must be set in the child env")
            .clone();
        assert!(
            !bin_dir.is_empty(),
            "US-009: PANEFLOW_BIN_DIR must not be empty"
        );

        let path = env
            .get("PATH")
            .expect("US-009 AC: PATH must be set after injection");
        let first = std::env::split_paths(path)
            .next()
            .expect("PATH must have at least one component");
        assert_eq!(
            first,
            PathBuf::from(&bin_dir),
            "US-009 AC: PANEFLOW_BIN_DIR must be first on PATH"
        );
    }

    // US-014: user-supplied env vars are merged into the child PTY env.
    #[test]
    fn user_env_is_merged_into_pty_env() {
        let mut user = HashMap::new();
        user.insert("ANTHROPIC_API_KEY".to_string(), "sk-test-123".to_string());
        user.insert("MY_CUSTOM_VAR".to_string(), "hello".to_string());
        let env = assemble_pty_env(HashMap::new(), 1, 1, Some(user));

        assert_eq!(
            env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-test-123"),
            "US-014 AC: user env var must be present in the child env"
        );
        assert_eq!(
            env.get("MY_CUSTOM_VAR").map(String::as_str),
            Some("hello"),
            "US-014 AC: a second user env var must also be present"
        );
    }

    // US-014: TERM/COLORTERM are protected and cannot be overridden by user env.
    #[test]
    fn protected_keys_cannot_be_overridden_by_user_env() {
        let mut user = HashMap::new();
        user.insert("TERM".to_string(), "dumb".to_string());
        user.insert("COLORTERM".to_string(), "nope".to_string());
        user.insert("KEEP_ME".to_string(), "yes".to_string());
        let env = assemble_pty_env(HashMap::new(), 1, 1, Some(user));

        assert_eq!(
            env.get("TERM").map(String::as_str),
            Some("xterm-256color"),
            "US-014 AC: TERM must stay Paneflow-owned even if the user sets it"
        );
        assert_eq!(
            env.get("COLORTERM").map(String::as_str),
            Some("truecolor"),
            "US-014 AC: COLORTERM must stay Paneflow-owned even if the user sets it"
        );
        assert_eq!(
            env.get("KEEP_ME").map(String::as_str),
            Some("yes"),
            "US-014: a non-protected user var alongside protected ones still wins"
        );
    }

    // f010: dynamic-loader env vars from an untrusted source (imported
    // session.json surface env / global config env) must never reach the child
    // shell — letting LD_PRELOAD/LD_*/DYLD_* through is an RCE vector.
    #[test]
    fn loader_influencing_env_vars_are_dropped() {
        let mut user = HashMap::new();
        user.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        user.insert("LD_LIBRARY_PATH".to_string(), "/tmp/evil".to_string());
        user.insert("LD_AUDIT".to_string(), "/tmp/audit.so".to_string());
        user.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "/tmp/e.dylib".to_string(),
        );
        user.insert("KEEP_ME".to_string(), "yes".to_string());
        let env = assemble_pty_env(HashMap::new(), 1, 1, Some(user));

        assert_eq!(
            env.get("LD_PRELOAD"),
            None,
            "f010: LD_PRELOAD from untrusted env must be dropped"
        );
        assert_eq!(
            env.get("LD_LIBRARY_PATH"),
            None,
            "f010: LD_LIBRARY_PATH from untrusted env must be dropped"
        );
        assert_eq!(
            env.get("LD_AUDIT"),
            None,
            "f010: LD_AUDIT from untrusted env must be dropped"
        );
        assert_eq!(
            env.get("DYLD_INSERT_LIBRARIES"),
            None,
            "f010: DYLD_* from untrusted env must be dropped"
        );
        assert_eq!(
            env.get("KEEP_ME").map(String::as_str),
            Some("yes"),
            "f010: a benign var alongside loader vars must still pass through"
        );
    }

    // US-001 (prd-pane-context-bridge): /proc cmdline parsing.
    #[cfg(target_os = "linux")]
    #[test]
    fn command_from_cmdline_joins_nul_separated_argv() {
        assert_eq!(
            super::command_from_cmdline(b"cargo\0run\0--release\0"),
            Some("cargo run --release".to_string())
        );
        // Trailing/leading NULs and empty fields are dropped.
        assert_eq!(
            super::command_from_cmdline(b"\0node\0\0server.js\0"),
            Some("node server.js".to_string())
        );
        // Empty blob → None (kernel thread / zombie).
        assert_eq!(super::command_from_cmdline(b""), None);
        assert_eq!(super::command_from_cmdline(b"\0\0"), None);
    }

    // US-019: foreground_command degrades gracefully (no panic, None) on a
    // display-only terminal (child_pid == 0, no real PTY) on every platform.
    #[test]
    fn foreground_command_none_for_display_only() {
        let state = TerminalState::new_display_only(24, 80);
        assert!(
            state.foreground_command().is_none(),
            "display-only terminal has no foreground process to resolve"
        );
    }

    #[test]
    fn restore_scrollback_strips_escape_and_osc_injection() {
        // A tampered session.json line carrying live VT bytes: an OSC8
        // clickable-link injection, an OSC0 title-spoof, a raw CSI, an ESC
        // introducer, a NUL, and a C1 control. None may survive sanitization.
        let hostile = "\x1b]8;;https://evil.example/\x07click\x1b]8;;\x07\
                       \x1b]0;PWNED\x07\x1b[31mred\x00\u{9b}38m";
        let cleaned = TerminalState::sanitize_scrollback_line(hostile);

        // No control byte that could start a VT sequence survives.
        assert!(
            !cleaned.contains('\x1b'),
            "ESC introducer must be stripped; got {cleaned:?}"
        );
        assert!(
            !cleaned.contains('\x07'),
            "BEL (OSC terminator) must be stripped; got {cleaned:?}"
        );
        assert!(
            !cleaned.contains('\x00'),
            "NUL / C0 controls must be stripped; got {cleaned:?}"
        );
        assert!(
            !cleaned.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)),
            "C1 controls must be stripped; got {cleaned:?}"
        );
        // Visible glyphs are preserved verbatim (no live sequence remains, so
        // these read as plain text rather than executing).
        for marker in ["https://evil.example/", "click", "PWNED", "red", "38m"] {
            assert!(
                cleaned.contains(marker),
                "plain glyphs must survive; {marker:?} missing from {cleaned:?}"
            );
        }
        // A tab is the one C0 byte we intentionally keep.
        assert_eq!(
            TerminalState::sanitize_scrollback_line("a\tb"),
            "a\tb",
            "tab must be preserved"
        );
    }

    /// US-011: `extract_scrollback_from` drains a *cloned* `SharedTerm` handle —
    /// the exact handle `serialize_deferred` ships to the background save task —
    /// and round-trips the seeded scrollback. Proves the drain is decoupled from
    /// `&self` so `save_session` can run it off the GPUI main thread. Three
    /// lines fit the visible grid, so the assertion is independent of any
    /// scrollback-history config.
    #[test]
    fn extract_scrollback_from_drains_cloned_handle() {
        let state = TerminalState::new_display_only(24, 80);
        state.restore_scrollback("alpha\nbravo\ncharlie");

        // Clone the Arc the way `LayoutTree::serialize_deferred` does, then
        // drain via the free associated fn (no `&self`).
        let handle = state.term.clone();
        let drained = TerminalState::extract_scrollback_from(&handle)
            .expect("seeded scrollback should not be empty");

        for marker in ["alpha", "bravo", "charlie"] {
            assert!(
                drained.contains(marker),
                "drained scrollback must contain {marker:?}; got:\n{drained}"
            );
        }
    }

    /// U-001: a multibyte codepoint straddling the byte cap must not panic
    /// `String::truncate`; the cut lands on a char boundary at or below the cap.
    #[test]
    fn cap_scrollback_truncates_on_char_boundary() {
        const MAX: usize = 100;
        // 99 ASCII bytes, then a 4-byte '🦀' occupying byte indices 99..103, so
        // byte index `MAX` (100) falls inside the codepoint — the case that
        // panics a raw `truncate(MAX)`. No newline, so the line-trim is a no-op.
        let mut s = "a".repeat(MAX - 1);
        s.push('🦀');
        assert!(s.len() > MAX, "fixture must exceed the cap");

        cap_scrollback_at_char_boundary(&mut s, MAX);

        // `String` already guarantees valid UTF-8; the contract is length ≤ cap
        // and that the straddling char was dropped whole rather than split.
        assert!(s.len() <= MAX, "capped length {} must be ≤ {MAX}", s.len());
        assert_eq!(s, "a".repeat(MAX - 1));
    }

    /// Already-aligned cap is a no-op beyond the existing line trim.
    #[test]
    fn cap_scrollback_noop_under_cap() {
        let mut s = "short line".to_string();
        let before = s.clone();
        cap_scrollback_at_char_boundary(&mut s, 100);
        assert_eq!(s, before);
    }

    /// Dump the viewport grid to a string for the live smoke test.
    #[cfg(unix)]
    fn grid_to_string(term: &Arc<FairMutex<Term<ZedListener>>>) -> String {
        let term = term.lock();
        let grid = term.grid();
        let mut out = String::new();
        for line in 0..grid.screen_lines() {
            for col in 0..grid.columns() {
                out.push(grid[AlacPoint::new(GridLine(line as i32), GridCol(col))].c);
            }
            out.push('\n');
        }
        out
    }

    /// EP-002 US-004 live smoke: spawn a REAL PTY-backed shell via
    /// `alacritty_terminal::tty` + `EventLoop`, write a marker command, and
    /// confirm the EventLoop read->parse path lands the echoed output in the
    /// `Term` grid. This is the only test that exercises `tty::new` +
    /// `EventLoop::spawn` + `Notifier` end-to-end — the others use the
    /// display-only path. Unix-only (drives `/bin/sh`); the process group is
    /// torn down by `Drop` at scope exit.
    #[cfg(unix)]
    #[test]
    fn eventloop_pty_echoes_input_into_grid() {
        let mut state = TerminalState::new(None, 1, 1, Some((80, 24)), None, None)
            .expect("EP-002: spawn a PTY-backed terminal via tty::new + EventLoop");
        assert!(state.child_pid > 0, "a real PTY child must have a pid");

        // Let the shell initialize, then send a unique marker command.
        std::thread::sleep(std::time::Duration::from_millis(250));
        state.notifier.notify(b"echo PANEFLOW_SMOKE_OK\n".to_vec());

        // Poll the grid (the EventLoop mutates it on its own thread) until the
        // echoed marker appears, draining events meanwhile. Generous budget so
        // a slow runner doesn't flake.
        let mut found = false;
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            state.sync();
            if grid_to_string(&state.term).contains("PANEFLOW_SMOKE_OK") {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "EP-002: the EventLoop read path did not deliver shell output to the grid"
        );
    }
}
