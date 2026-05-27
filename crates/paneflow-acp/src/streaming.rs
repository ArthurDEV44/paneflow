//! Streaming buffer that paces agent message reveals.
//!
//! Goal: smooth typewriter playback in the UI. ACP agents emit
//! `AgentMessageChunk` notifications at variable cadence -- often very
//! fast bursts (10+ chunks per tick). Rendering each chunk immediately
//! produces a choppy or instant-reveal feel. Buffering each tick and
//! revealing a fraction targets a steady ~200 ms reveal-per-burst at a
//! 16 ms tick (~60 fps).
//!
//! The buffer is deterministic (no real-time clock) -- callers drive ticks
//! from a `tokio::time::interval` in production and from explicit
//! `tick()` calls in tests. See US-002 of `tasks/prd-agents-view.md`.

use std::time::Duration;

/// Pacing knobs. Defaults match Zed (`acp_thread.rs:1825`): 16 ms tick,
/// 200 ms reveal target.
#[derive(Clone, Copy, Debug)]
pub struct PacingConfig {
    pub tick_interval: Duration,
    pub target_reveal: Duration,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_millis(16),
            target_reveal: Duration::from_millis(200),
        }
    }
}

/// Accumulates incoming chunks and drains them in even slices, one per
/// [`tick`][StreamingBuffer::tick] call.
///
/// `pending_chars` mirrors `pending.chars().count()` and is maintained
/// incrementally on every push / tick / flush. The previous design
/// called `chars().count()` on every tick, which is O(n) over the
/// pending buffer; under sustained streaming bursts that becomes the
/// hottest allocation-adjacent path in the agents UI. Keeping the
/// count in sync drops that cost to O(1) per tick.
#[derive(Debug)]
pub struct StreamingBuffer {
    pending: String,
    pending_chars: usize,
    pacing: PacingConfig,
    current_tick: u64,
    completion_tick: Option<u64>,
}

impl StreamingBuffer {
    pub fn new(pacing: PacingConfig) -> Self {
        Self {
            pending: String::new(),
            pending_chars: 0,
            pacing,
            current_tick: 0,
            completion_tick: None,
        }
    }

    /// Queue `chunk` for paced reveal. Empty chunks are no-ops.
    ///
    /// Each push extends the completion deadline to `current_tick +
    /// target_ticks`, where `target_ticks = ceil(target_reveal /
    /// tick_interval)`. The buffer never *shrinks* an existing deadline,
    /// so a fast-arriving burst followed by silence still drains at the
    /// pacing target rather than slamming the UI.
    pub fn push(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        let was_idle = self.pending.is_empty();
        self.pending.push_str(chunk);
        self.pending_chars += chunk.chars().count();
        let new_deadline = self.current_tick.saturating_add(self.target_ticks());
        self.completion_tick = Some(if was_idle {
            new_deadline
        } else {
            self.completion_tick
                .map(|d| d.max(new_deadline))
                .unwrap_or(new_deadline)
        });
    }

    /// Advance one tick and return the chunk to reveal *this* tick. The
    /// returned string may be empty if no content is pending.
    ///
    /// The drain rate is `ceil(pending_chars / ticks_left)` per tick, with
    /// `ticks_left = max(1, completion_tick - current_tick)`. This makes
    /// the algorithm converge: pending always reaches zero by the
    /// completion tick, and ticks after that yield empty strings.
    pub fn tick(&mut self) -> String {
        self.current_tick = self.current_tick.saturating_add(1);
        if self.pending.is_empty() {
            self.completion_tick = None;
            return String::new();
        }
        let deadline = self.completion_tick.unwrap_or(self.current_tick);
        let ticks_left = deadline.saturating_sub(self.current_tick).max(1) as usize;
        let total = self.pending_chars;
        let per_tick = total.div_ceil(ticks_left).clamp(1, total);
        let split = self
            .pending
            .char_indices()
            .nth(per_tick)
            .map(|(i, _)| i)
            .unwrap_or(self.pending.len());
        let revealed = self.pending[..split].to_string();
        let revealed_chars = revealed.chars().count();
        self.pending.drain(..split);
        self.pending_chars = self.pending_chars.saturating_sub(revealed_chars);
        if self.pending.is_empty() {
            self.pending_chars = 0;
            self.completion_tick = None;
        }
        revealed
    }

    /// Drain all remaining content immediately, bypassing pacing. Used on
    /// stream-end (`StopReason` received) and on thread-close to avoid
    /// orphaning content in the buffer.
    pub fn flush(&mut self) -> String {
        self.completion_tick = None;
        self.pending_chars = 0;
        std::mem::take(&mut self.pending)
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn pending_chars(&self) -> usize {
        self.pending_chars
    }

    fn target_ticks(&self) -> u64 {
        let target = self.pacing.target_reveal.as_micros();
        let tick = self.pacing.tick_interval.as_micros().max(1);
        (target.div_ceil(tick) as u64).max(1)
    }
}

impl Default for StreamingBuffer {
    fn default() -> Self {
        Self::new(PacingConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drain pending content in a single burst and report how many ticks
    /// were consumed. Used to assert pacing in milliseconds.
    fn drain_until_idle(buf: &mut StreamingBuffer) -> (u64, String) {
        let mut ticks = 0;
        let mut revealed = String::new();
        while !buf.is_idle() {
            revealed.push_str(&buf.tick());
            ticks += 1;
            assert!(ticks < 10_000, "runaway drain loop");
        }
        (ticks, revealed)
    }

    #[test]
    fn drain_completes_within_target_window() {
        let mut buf = StreamingBuffer::default();
        let content: String = "x".repeat(192);
        buf.push(&content);
        let (ticks, revealed) = drain_until_idle(&mut buf);
        assert_eq!(revealed, content);
        let elapsed_ms = ticks * 16;
        let diff = (elapsed_ms as i64 - 200).abs();
        assert!(
            diff <= 20,
            "drain elapsed {elapsed_ms} ms (target 200 ms ±20 ms, took {ticks} ticks)",
        );
    }

    #[test]
    fn bursty_5ms_arrivals_drain_within_200ms_after_last_push() {
        // Simulate: 20 chunks of 10 chars arrive every 5 ms over 100 ms;
        // ticks fire every 16 ms. Assert all content is revealed and the
        // buffer drains within ~200 ms after the LAST push (matches Zed
        // pacing semantics).
        let mut buf = StreamingBuffer::default();
        let mut revealed = String::new();
        let mut chunks_pushed = 0;
        let total_chunks = 20;
        let last_push_ms = 100;
        let mut last_push_tick = 0u64;
        let mut drain_complete_tick: Option<u64> = None;

        for ms in 1..=1_000u64 {
            if chunks_pushed < total_chunks && ms.is_multiple_of(5) && ms <= last_push_ms {
                buf.push("0123456789");
                chunks_pushed += 1;
                last_push_tick = buf.current_tick;
            }
            if ms.is_multiple_of(16) {
                revealed.push_str(&buf.tick());
                if buf.is_idle() && chunks_pushed == total_chunks {
                    drain_complete_tick = Some(buf.current_tick);
                    break;
                }
            }
        }

        assert_eq!(revealed.len(), 200, "all chunks must be revealed");
        let end_tick = drain_complete_tick.expect("buffer must drain within the test window");
        let drain_elapsed_ticks = end_tick.saturating_sub(last_push_tick);
        let drain_elapsed_ms = drain_elapsed_ticks * 16;
        // Steady-state pacing target is 200 ms after the last push; allow
        // a generous ±32 ms slack (two tick periods) since chunk arrivals
        // up-to but not exceeding 100 ms keep extending the deadline.
        assert!(
            drain_elapsed_ms <= 232,
            "drained {drain_elapsed_ms} ms after last push (max 232 ms)",
        );
    }

    #[test]
    fn flush_reveals_everything_immediately() {
        let mut buf = StreamingBuffer::default();
        buf.push("abcdef");
        let flushed = buf.flush();
        assert_eq!(flushed, "abcdef");
        assert!(buf.is_idle());
    }

    #[test]
    fn empty_chunk_is_noop() {
        let mut buf = StreamingBuffer::default();
        buf.push("");
        assert!(buf.is_idle());
        assert_eq!(buf.tick(), "");
    }

    #[test]
    fn tick_on_idle_buffer_returns_empty() {
        let mut buf = StreamingBuffer::default();
        assert_eq!(buf.tick(), "");
        assert_eq!(buf.tick(), "");
    }

    #[test]
    fn ac7_us_015_thousand_chars_over_50ms_reveal_within_200ms() {
        // US-015 AC #7: produce 1000 chars over 50 ms via mock agent;
        // reveal completes in 200 ms +/- 20 ms.
        //
        // Map onto deterministic ticks: 10 pushes of 100 chars each
        // arrive at 5 ms cadence (covering 5..=50 ms). Ticks fire
        // every 16 ms. We measure the drain elapsed time *after the
        // last push* because that is when the pacing deadline is
        // anchored (see `StreamingBuffer::push` doc -- the deadline
        // extends with every push, so total elapsed = burst window +
        // 200 ms steady-state target).
        let mut buf = StreamingBuffer::default();
        let mut revealed = String::new();
        let chunk = "0123456789".repeat(10); // 100 chars per push
        let total_pushes = 10;
        let mut pushes_done = 0;
        let mut last_push_tick = 0u64;
        let mut drain_complete_tick: Option<u64> = None;

        for ms in 1..=1_000u64 {
            // Mock agent: 10 chunks of 100 chars over 50 ms (one
            // every 5 ms). Total 1000 chars matches the AC verbatim.
            if pushes_done < total_pushes && ms.is_multiple_of(5) && ms <= 50 {
                buf.push(&chunk);
                pushes_done += 1;
                last_push_tick = buf.current_tick;
            }
            // Tick the buffer at 16 ms cadence (matches the UI ticker
            // in `agents::thread_view::ThreadView::begin_assistant_stream`).
            if ms.is_multiple_of(16) {
                revealed.push_str(&buf.tick());
                if buf.is_idle() && pushes_done == total_pushes {
                    drain_complete_tick = Some(buf.current_tick);
                    break;
                }
            }
        }

        assert_eq!(revealed.len(), 1000, "all 1000 chars must reveal");
        let end_tick =
            drain_complete_tick.expect("buffer must drain within the 1000 ms test window");
        let drain_ms = end_tick.saturating_sub(last_push_tick) * 16;
        // 200 ms +/- 20 ms per AC. The buffer can only complete on a
        // tick boundary (every 16 ms), so the realised drain time is
        // a multiple of 16 -- still well inside the +/- 20 ms window.
        let diff = (drain_ms as i64 - 200).abs();
        assert!(
            diff <= 20,
            "AC #7: drained in {drain_ms} ms after last push (target 200 ms +/- 20 ms)",
        );
    }

    #[test]
    fn utf8_multibyte_split_safely() {
        let mut buf = StreamingBuffer::default();
        // Each "é" is 2 bytes but 1 char; mix with ASCII.
        let content = "héllo wörld ".repeat(20); // 240 bytes, 240 chars
        buf.push(&content);
        let (_, revealed) = drain_until_idle(&mut buf);
        assert_eq!(revealed, content);
    }
}
