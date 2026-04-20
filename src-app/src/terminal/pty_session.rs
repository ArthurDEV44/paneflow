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
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::Rgb as AlacRgb;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};

use crate::pty::PtyBackend;

use super::listener::{SpikeTermSize, ZedListener};
use super::pty_loops::{pty_message_loop, pty_reader_loop};
use super::service_detector::{ServiceInfo, detect_framework, parse_service_line};
use super::shell::{resolve_default_shell, setup_shell_integration};

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
// OSC 52 clipboard mode + OSC 133 prompt marks
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
pub(super) enum ClipboardOp {
    Store(String),
    Load(std::sync::Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

/// Deferred color query response (OSC 10/11/12) from sync().
pub(super) struct ColorOp {
    pub index: usize,
    pub format_fn: std::sync::Arc<dyn Fn(AlacRgb) -> String + Sync + Send + 'static>,
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
    pub(super) pending_clipboard_ops: Vec<ClipboardOp>,
    /// Deferred color query responses (OSC 10/11/12) from sync().
    pub(super) pending_color_ops: Vec<ColorOp>,
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
        // Fallback chain handled by `resolve_default_shell` (US-006):
        // Unix:    config → $SHELL → /bin/sh
        // Windows: config → %ComSpec% → C:\Windows\System32\cmd.exe →
        //          powershell.exe on PATH → bare "cmd.exe"
        let shell = {
            let config = paneflow_config::loader::load_config();
            let configured = config
                .default_shell
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            resolve_default_shell(configured)
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

        // US-013: the AI-hook wrapper-scripts system was removed. The
        // embed targets never shipped, so extraction was a no-op and the
        // PATH-prepend step pointed at an empty directory. A future
        // cross-platform AI-hook system will live in its own PRD and
        // plumb through a fresh env-var + extraction point.

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
            AlacEvent::ClipboardLoad(_selection, format_fn)
                if self.osc52_mode == Osc52Mode::CopyPaste =>
            {
                self.pending_clipboard_ops
                    .push(ClipboardOp::Load(format_fn));
            }
            AlacEvent::ClipboardLoad(..) => {}

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

    /// macOS implementation of `cwd_now`: read the PTY child shell's current
    /// working directory from the kernel via
    /// `proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &buf, size)`.
    #[cfg(target_os = "macos")]
    pub fn cwd_now(&self) -> Option<std::path::PathBuf> {
        use std::ffi::CStr;
        use std::mem::MaybeUninit;
        use std::os::raw::c_void;

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

impl Drop for TerminalState {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);

        // Grace period + force-kill: if the child process ignores the PTY
        // master close signal (SIGHUP on Unix, ClosePseudoConsole on Windows),
        // force-kill it after 100ms.
        #[cfg(unix)]
        {
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
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(100));
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
                });
            }
        }
    }
}
