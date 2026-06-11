//! PTY byte tap (EP-003 US-006, prd-cli-cockpit-ergonomics-2026-Q3.md).
//!
//! Spike verdict (Q1, traced here): alacritty_terminal 0.26.0 is the latest
//! crates.io release; it has no OSC 133 support (alacritty issue #5850,
//! open/unprioritized since 2022) and no pre-parse hook — and contrary to
//! the PRD's premise, Paneflow does NOT vendor the event loop (the 2-thread
//! reader was removed in 3126ffd). Neither a version bump nor a fork is
//! needed though: `tty::EventedReadWrite` / `EventedPty` are public traits
//! and `EventLoop<T, U>` is generic over the PTY, so a wrapper can tap the
//! byte stream without touching the crate. Shutdown, drain-on-exit, signal
//! handling and the parser all stay upstream.
//!
//! The ownership trick: `EventedReadWrite::reader(&mut self) -> &mut
//! Self::Reader` makes it impossible to wrap the inner reader by value (the
//! PTY owns it), so `TeePty` declares `type Reader = Self` — it IS its own
//! reader, delegating `read()` to the inner PTY's reader and then feeding
//! the freshly read bytes to the OSC 133 scanner. Pure byte processing on
//! the existing reader thread: platform-neutral (Linux/macOS/Windows), no
//! extra thread, no lock, and zero allocation on chunks without sequences
//! (US-006 AC3 — the scanner's hot path is a single ESC position scan).

use std::io;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use polling::{Event, PollMode, Poller};

use super::marks::{Osc133Scanner, RawMark};

pub struct TeePty<P: EventedPty> {
    inner: P,
    scanner: Osc133Scanner,
    /// Marks detected on the reader thread, drained by the GPUI poll loop
    /// which anchors them to the grid (`TerminalState::drain_marks`). An
    /// unbounded std channel is fine: the producer only sends on actual
    /// OSC 133 sequences (shell-integration prompts, human-paced), and a
    /// hostile flood is implicitly back-pressured by the parser consuming
    /// the same bytes on the same thread - the queue holds at most one
    /// GPUI poll tick's worth of tiny `RawMark`s (security review, INFO).
    marks_tx: Sender<RawMark>,
}

impl<P: EventedPty> TeePty<P> {
    pub fn new(inner: P, marks_tx: Sender<RawMark>) -> Self {
        Self {
            inner,
            scanner: Osc133Scanner::default(),
            marks_tx,
        }
    }
}

impl<P: EventedPty> io::Read for TeePty<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.reader().read(buf)?;
        if n > 0 {
            let tx = &self.marks_tx;
            self.scanner.feed(&buf[..n], &mut |mark| {
                // A send can only fail when the terminal state was dropped
                // while the IO thread drains — nothing to do with the mark.
                let _ = tx.send(mark);
            });
        }
        Ok(n)
    }
}

impl<P: EventedPty> EventedReadWrite for TeePty<P> {
    type Reader = Self;
    type Writer = P::Writer;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        // SAFETY: same contract as the inner PTY — the underlying sources
        // are owned by `inner`, which lives exactly as long as `self`.
        unsafe { self.inner.register(poll, interest, mode) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl<P: EventedPty> EventedPty for TeePty<P> {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl<P: EventedPty + OnResize> OnResize for TeePty<P> {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}
