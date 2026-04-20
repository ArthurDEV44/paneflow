//! Byte-level scanners run on raw PTY bytes **before** they reach the VTE parser.
//!
//! Alacritty silently drops OSC 7 and OSC 133, and Zed's listener channel
//! doesn't surface XTVERSION queries either, so we intercept them here in
//! `pty_reader_loop` and emit structured events through dedicated channels.
//!
//! Each scanner maintains a small byte-by-byte FSM. Keep the state machines
//! local to this module — they are intentionally not part of any public API.

use std::sync::Arc;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::sync::FairMutex;

use futures::channel::mpsc::UnboundedSender;

use super::{PromptMark, PromptMarkKind, ZedListener};

// ---------------------------------------------------------------------------
// XTVERSION query scanner
// ---------------------------------------------------------------------------

/// Byte-level scanner for XTVERSION queries (`\x1b[>0q` or `\x1b[>q`).
/// When detected, emits a DCS response via the event listener.
pub(super) struct XtversionScanner {
    match_pos: usize,
}

/// Sequence: ESC [ > 0 q  (also accept ESC [ > q with implicit 0 param)
const XTVERSION_SEQ: [u8; 5] = [0x1b, b'[', b'>', b'0', b'q'];

impl XtversionScanner {
    pub(super) fn new() -> Self {
        Self { match_pos: 0 }
    }

    /// Scan `buf` for XTVERSION queries. If found, send a DCS response via `listener`.
    pub(super) fn scan(&mut self, buf: &[u8], listener: &ZedListener) {
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
pub(super) struct Osc7Scanner {
    state: u8,
    payload: Vec<u8>,
}

impl Osc7Scanner {
    pub(super) fn new() -> Self {
        Self {
            state: 0,
            payload: Vec::new(),
        }
    }

    pub(super) fn scan(&mut self, buf: &[u8], cwd_tx: &UnboundedSender<String>) {
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
pub(super) struct Osc133Scanner {
    state: u8,
}

impl Osc133Scanner {
    pub(super) fn new() -> Self {
        Self { state: 0 }
    }

    pub(super) fn scan(
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
