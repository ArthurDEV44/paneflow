//! portable-pty I/O threads — replace alacritty's `EventLoop`.
//!
//! Two detached threads per terminal:
//! - `pty_reader_loop` reads PTY bytes, scans for OSC sequences, drives the
//!   VTE parser, and emits `Wakeup`/`ChildExit`/`Exit` events.
//! - `pty_message_loop` receives `Msg` from `PtyNotifier` and forwards input
//!   or resize commands back to the PTY master.
//!
//! Extracted from `terminal.rs` per US-011 of the src-app refactor PRD.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::sync::FairMutex;
use futures::channel::mpsc::UnboundedSender;
use portable_pty::PtySize;

use super::scanners::{Osc7Scanner, Osc133Scanner, XtversionScanner};
use super::{PromptMark, ZedListener};

/// Reader thread: reads PTY output, feeds through VTE parser into Term, sends Wakeup events.
/// Owns the child handle to capture exit status after the read loop ends.
/// DEC 2026: scans raw bytes for BSU/ESU before VTE processing, suppresses Wakeup during sync.
pub(super) fn pty_reader_loop(
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
pub(super) fn pty_message_loop(
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
