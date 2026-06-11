//! Command Outcome Marks (EP-003, prd-cli-cockpit-ergonomics-2026-Q3.md).
//!
//! US-006: a zero-allocation byte scanner recognizing OSC 133 A/B/C/D
//! (FinalTerm/iTerm2 semantic prompt marks) from the raw PTY stream, and a
//! bounded per-terminal ring of [`CommandMark`]s keyed on an absolute grid
//! line counter.
//!
//! The scanner runs on the PTY reader thread inside [`super::tee_pty::TeePty`]
//! — the byte tap chosen by the US-006 spike (alacritty_terminal 0.26 is the
//! latest crates.io release, has no OSC 133 support and no pre-parse hook;
//! a fork is unnecessary because the public `tty::EventedReadWrite` traits
//! allow wrapping the PTY reader). It is pure byte processing: no OS APIs,
//! identical on Linux/macOS/Windows.
//!
//! Marks are session-local by design (Non-Goal: the session restore strips
//! escapes, so persisted marks would have nothing to anchor to).

use std::time::Instant;

/// Which OSC 133 marker was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    /// `133;A` — prompt start.
    PromptStart,
    /// `133;B` — command (input) start.
    CommandStart,
    /// `133;C` — command output start (pre-exec).
    OutputStart,
    /// `133;D[;code]` — command finished.
    CommandFinished,
}

/// A marker as detected on the PTY thread — no grid coordinates yet (the
/// parser hasn't necessarily consumed the chunk when the tap sees it). The
/// GPUI drain resolves the absolute line right after the batch is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMark {
    pub kind: MarkKind,
    /// Exit code carried by `D;<code>`. `None` for A/B/C, for a bare `D`,
    /// and for a malformed / oversized code (hostile input is ignored, never
    /// an error).
    pub exit_code: Option<i32>,
}

/// A marker anchored to the grid.
#[derive(Debug, Clone, Copy)]
pub struct CommandMark {
    pub kind: MarkKind,
    pub exit_code: Option<i32>,
    /// Absolute grid line: `history_size + cursor_line` at drain time.
    /// Exact while the scrollback history is below its limit; once the
    /// history saturates and drops its oldest lines this anchor drifts
    /// (documented v1 tolerance — same posture as the resize/reflow drift
    /// accepted by US-008). Stale marks age out through the ring cap.
    pub abs_line: i64,
    /// When the mark was recorded (drives the hover tooltip's relative
    /// timestamp).
    pub at: Instant,
}

/// Ring capacity per terminal (US-006: bounded, drop-oldest).
pub const MAX_MARKS: usize = 1_000;

/// Bounded mark store for one terminal.
#[derive(Default)]
pub struct MarkRing {
    marks: std::collections::VecDeque<CommandMark>,
}

impl MarkRing {
    pub fn push(&mut self, mark: CommandMark) {
        if self.marks.len() == MAX_MARKS {
            self.marks.pop_front();
        }
        self.marks.push_back(mark);
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Drop marks pointing BELOW the live bottom (a grid clear/reset pulled
    /// `history_size` back while old marks still carry larger anchors —
    /// US-006 AC5: never a mark pointing outside the grid).
    pub fn retain_at_or_below(&mut self, max_abs_line: i64) {
        self.marks.retain(|m| m.abs_line <= max_abs_line);
    }

    /// Iterate all marks, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &CommandMark> {
        self.marks.iter()
    }

    /// The nearest `PromptStart` strictly above `abs_line` (US-008 jump
    /// backward). `None` at the top extremity — callers no-op silently.
    pub fn prompt_before(&self, abs_line: i64) -> Option<i64> {
        self.marks
            .iter()
            .rev()
            .filter(|m| m.kind == MarkKind::PromptStart)
            .map(|m| m.abs_line)
            .find(|&l| l < abs_line)
    }

    /// The nearest `PromptStart` strictly below `abs_line` (jump forward).
    pub fn prompt_after(&self, abs_line: i64) -> Option<i64> {
        self.marks
            .iter()
            .filter(|m| m.kind == MarkKind::PromptStart)
            .map(|m| m.abs_line)
            .find(|&l| l > abs_line)
    }
}

// ---------------------------------------------------------------------------
// OSC 133 byte scanner
// ---------------------------------------------------------------------------

/// Cap on accepted OSC 133 payload bytes after `133;` (`D;-2147483648` is
/// 12 bytes). Hostile oversized params flip the scanner into a skip state
/// that waits for the terminator without buffering (US-006 AC4: bounded,
/// panic-free on adversarial input).
const PAYLOAD_CAP: usize = 16;

/// The prefix every interesting sequence starts with after `ESC ]`.
const OSC_PREFIX: &[u8] = b"133;";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    /// Looking for ESC. The hot state — a chunk with no ESC byte stays
    /// entirely in this state and costs one scan, zero allocations.
    Ground,
    /// Saw ESC, expecting `]` (otherwise back to Ground).
    Esc,
    /// Inside an OSC, matching the `133;` prefix byte by byte.
    Prefix(u8),
    /// Prefix matched — buffering the payload (`A`, `D;0`, …).
    Payload,
    /// Inside an OSC that is not ours, or whose payload overflowed —
    /// consuming bytes until the terminator without buffering.
    SkipToTerminator,
    /// Saw ESC inside Payload/Skip — if the next byte is `\` (ST) the
    /// sequence ends; anything else aborts back to Ground (an ESC inside
    /// an OSC body is invalid anyway).
    PayloadEsc,
    SkipEsc,
}

/// Incremental, allocation-free OSC 133 recognizer. Sequences may be split
/// across `feed` calls at any byte boundary (PTY reads are arbitrary
/// chunks). Accepts both ST (`ESC \`) and BEL terminators.
pub struct Osc133Scanner {
    state: ScanState,
    payload: [u8; PAYLOAD_CAP],
    payload_len: usize,
}

impl Default for Osc133Scanner {
    fn default() -> Self {
        Self {
            state: ScanState::Ground,
            payload: [0; PAYLOAD_CAP],
            payload_len: 0,
        }
    }
}

impl Osc133Scanner {
    /// Scan a chunk, invoking `on_mark` for each complete, well-formed
    /// OSC 133 sequence. Never allocates; never panics on hostile input.
    pub fn feed(&mut self, bytes: &[u8], on_mark: &mut impl FnMut(RawMark)) {
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                ScanState::Ground => {
                    // Skip to the next ESC in one pass (the no-sequence hot
                    // path: one position scan per chunk, no per-byte state
                    // machine churn).
                    match bytes[i..].iter().position(|&b| b == 0x1b) {
                        Some(off) => {
                            i += off + 1;
                            self.state = ScanState::Esc;
                        }
                        None => return,
                    }
                    continue;
                }
                ScanState::Esc => {
                    self.state = if bytes[i] == b']' {
                        ScanState::Prefix(0)
                    } else {
                        ScanState::Ground
                    };
                }
                ScanState::Prefix(matched) => {
                    let b = bytes[i];
                    if b == OSC_PREFIX[matched as usize] {
                        let next = matched + 1;
                        self.state = if next as usize == OSC_PREFIX.len() {
                            self.payload_len = 0;
                            ScanState::Payload
                        } else {
                            ScanState::Prefix(next)
                        };
                    } else if b == 0x07 || b == 0x18 || b == 0x1a {
                        // Short foreign OSC terminated by BEL, or a CAN/SUB
                        // abort (ECMA-48).
                        self.state = ScanState::Ground;
                    } else if b == 0x1b {
                        self.state = ScanState::SkipEsc;
                    } else {
                        // Some other OSC (e.g. `0;title`, `7;file://…`).
                        self.state = ScanState::SkipToTerminator;
                    }
                }
                ScanState::Payload => match bytes[i] {
                    0x07 => {
                        self.emit(on_mark);
                        self.state = ScanState::Ground;
                    }
                    0x1b => self.state = ScanState::PayloadEsc,
                    // CAN/SUB abort the sequence without emitting (ECMA-48).
                    0x18 | 0x1a => self.state = ScanState::Ground,
                    b => {
                        if self.payload_len < PAYLOAD_CAP {
                            self.payload[self.payload_len] = b;
                            self.payload_len += 1;
                        } else {
                            // Hostile oversized payload: ignore the whole
                            // sequence, bounded state only.
                            self.state = ScanState::SkipToTerminator;
                        }
                    }
                },
                ScanState::PayloadEsc => {
                    self.state = match bytes[i] {
                        b'\\' => {
                            self.emit(on_mark);
                            ScanState::Ground
                        }
                        // A fresh ESC right after the aborting ESC starts a
                        // new escape — it must not be swallowed (review F1:
                        // `…ESC ESC ]133;A` would otherwise lose the mark).
                        0x1b => ScanState::Esc,
                        _ => ScanState::Ground,
                    };
                }
                ScanState::SkipToTerminator => match bytes[i] {
                    0x07 | 0x18 | 0x1a => self.state = ScanState::Ground,
                    0x1b => self.state = ScanState::SkipEsc,
                    _ => {}
                },
                ScanState::SkipEsc => {
                    // ST ends the foreign OSC; `]` means the prior OSC never
                    // got its ST and a new opener arrived; a fresh ESC keeps
                    // the escape pending (review F1 — never swallow it).
                    self.state = match bytes[i] {
                        b'\\' => ScanState::Ground,
                        b']' => ScanState::Prefix(0),
                        0x1b => ScanState::Esc,
                        _ => ScanState::Ground,
                    };
                }
            }
            i += 1;
        }
    }

    fn emit(&mut self, on_mark: &mut impl FnMut(RawMark)) {
        if let Some(mark) = parse_payload(&self.payload[..self.payload_len]) {
            on_mark(mark);
        }
        self.payload_len = 0;
    }
}

/// Parse the bytes after `133;` (e.g. `A`, `D;0`, `C;extra=1`). Unknown
/// kinds, empty payloads and malformed exit codes yield `None`/`None`-code —
/// hostile input is dropped, never an error (US-006 AC4).
fn parse_payload(payload: &[u8]) -> Option<RawMark> {
    let (kind_byte, rest) = payload.split_first()?;
    let kind = match kind_byte {
        b'A' => MarkKind::PromptStart,
        b'B' => MarkKind::CommandStart,
        b'C' => MarkKind::OutputStart,
        b'D' => MarkKind::CommandFinished,
        _ => return None,
    };
    let exit_code = if kind == MarkKind::CommandFinished {
        rest.strip_prefix(b";").and_then(|code| {
            // Stop at the first `;` — `D;0;aid=…` extensions exist.
            let code = code.split(|&b| b == b';').next().unwrap_or(code);
            std::str::from_utf8(code).ok()?.parse::<i32>().ok()
        })
    } else {
        None
    };
    Some(RawMark { kind, exit_code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<RawMark> {
        let mut scanner = Osc133Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            scanner.feed(c, &mut |m| out.push(m));
        }
        out
    }

    #[test]
    fn recognizes_all_kinds_with_bel_and_st() {
        let marks = scan(&[b"\x1b]133;A\x07ls -la\r\n\x1b]133;C\x1b\\out\x1b]133;D;0\x07"]);
        assert_eq!(
            marks,
            vec![
                RawMark {
                    kind: MarkKind::PromptStart,
                    exit_code: None
                },
                RawMark {
                    kind: MarkKind::OutputStart,
                    exit_code: None
                },
                RawMark {
                    kind: MarkKind::CommandFinished,
                    exit_code: Some(0)
                },
            ]
        );
    }

    #[test]
    fn sequences_split_across_chunks_at_every_boundary() {
        // US-006 AC8: PTY reads chunk arbitrarily — split the same sequence
        // at every possible byte boundary and expect one identical mark.
        let seq = b"\x1b]133;D;127\x1b\\";
        for cut in 1..seq.len() {
            let marks = scan(&[&seq[..cut], &seq[cut..]]);
            assert_eq!(
                marks,
                vec![RawMark {
                    kind: MarkKind::CommandFinished,
                    exit_code: Some(127)
                }],
                "split at {cut}"
            );
        }
    }

    #[test]
    fn binary_interleaving_and_foreign_oscs_are_ignored() {
        let marks = scan(&[
            b"\x00\xff\x1b[31mred\x1b]0;title\x07\x1b]7;file://h/p\x1b\\\x1b]133;A\x07\xfe\xfd",
        ]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::PromptStart);
    }

    #[test]
    fn hostile_payloads_are_bounded_and_dropped() {
        // Non-numeric exit code → mark kept, code None.
        let marks = scan(&[b"\x1b]133;D;notanumber\x07"]);
        assert_eq!(
            marks,
            vec![RawMark {
                kind: MarkKind::CommandFinished,
                exit_code: None
            }]
        );
        // Giant payload → whole sequence ignored, state machine survives
        // and the next legit sequence still parses.
        let big = vec![b'x'; 64 * 1024];
        let marks = scan(&[b"\x1b]133;D;", &big, b"\x07\x1b]133;A\x07"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::PromptStart);
        // Unknown kind → dropped.
        assert!(scan(&[b"\x1b]133;Z\x07"]).is_empty());
        // Empty payload → dropped.
        assert!(scan(&[b"\x1b]133;\x07"]).is_empty());
    }

    #[test]
    fn d_with_extension_params_parses_code() {
        let marks = scan(&[b"\x1b]133;D;1;aid=42\x07"]);
        assert_eq!(marks[0].exit_code, Some(1));
    }

    #[test]
    fn esc_inside_payload_aborts_cleanly() {
        // ESC followed by something else than `\` aborts the sequence; the
        // stream keeps scanning.
        let marks = scan(&[b"\x1b]133;A\x1bX\x1b]133;C\x07"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::OutputStart);
    }

    #[test]
    fn double_esc_does_not_swallow_the_next_sequence() {
        // Review F1: a fresh ESC right after an aborting ESC must start a
        // new escape, not be consumed — in both skip and payload states.
        let marks = scan(&[b"\x1b]0;junk\x1b\x1b]133;A\x07"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::PromptStart);
        let marks = scan(&[b"\x1b]133;D;1\x1b\x1b]133;A\x07"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::PromptStart);
    }

    #[test]
    fn can_sub_abort_the_sequence_without_emitting() {
        // ECMA-48: CAN (0x18) / SUB (0x1a) abort an in-flight control string.
        let marks = scan(&[b"\x1b]133;D;1\x18\x1b]133;A\x07"]);
        assert_eq!(
            marks,
            vec![RawMark {
                kind: MarkKind::PromptStart,
                exit_code: None
            }]
        );
        let marks = scan(&[b"\x1b]0;junk\x1a\x1b]133;C\x07"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, MarkKind::OutputStart);
    }

    #[test]
    fn ring_caps_and_purges() {
        let mut ring = MarkRing::default();
        for i in 0..(MAX_MARKS + 10) {
            ring.push(CommandMark {
                kind: MarkKind::PromptStart,
                exit_code: None,
                abs_line: i as i64,
                at: Instant::now(),
            });
        }
        assert_eq!(ring.iter().count(), MAX_MARKS);
        // Oldest dropped first.
        assert_eq!(ring.iter().next().unwrap().abs_line, 10);
        // Grid clear/reset purge: anchors beyond the new bottom are dropped.
        ring.retain_at_or_below(500);
        assert!(ring.iter().all(|m| m.abs_line <= 500));
    }

    #[test]
    fn prompt_navigation_finds_neighbors() {
        let mut ring = MarkRing::default();
        for (line, kind) in [
            (10, MarkKind::PromptStart),
            (12, MarkKind::CommandFinished),
            (20, MarkKind::PromptStart),
            (30, MarkKind::PromptStart),
        ] {
            ring.push(CommandMark {
                kind,
                exit_code: None,
                abs_line: line,
                at: Instant::now(),
            });
        }
        assert_eq!(ring.prompt_before(25), Some(20));
        assert_eq!(ring.prompt_before(10), None); // top extremity → no-op
        assert_eq!(ring.prompt_after(20), Some(30));
        assert_eq!(ring.prompt_after(30), None); // bottom extremity → no-op
    }
}
