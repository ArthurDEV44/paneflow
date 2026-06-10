//! `paneflow wait` — block until a regex appears in a pane (US-013/US-014/US-015).
//!
//! The orchestration primitive ("Playwright-for-terminals"): poll a target
//! pane's recent scrollback until a regex matches, with a bounded timeout and
//! distinct exit codes (0 match, EXIT_TIMEOUT on timeout) so a shell pipeline
//! can chain "launch agent -> wait for done -> next step".
//!
//! Matching uses a real client-side regex over a bounded recent window
//! (`surface.read`, last `READ_WINDOW_LINES`): `wait` watches for NEW output,
//! which lands at the tail, and the window stays well under the IPC client's
//! 256 KiB response cap (a full-buffer read could blow it). Each poll opens and
//! closes exactly one connection, so a long `wait` never holds a socket open
//! between polls and never approaches the server's 16-connection cap.

use std::thread::sleep;
use std::time::{Duration, Instant};

use paneflow_ipc_client::IpcTransport;
use regex::Regex;
use serde_json::{Value, json};

use super::selector::{resolve_all, resolve_target};
use super::{CliError, EXIT_OK, EXIT_TIMEOUT};

const POLL_INTERVAL_MS: u64 = 500;
const DEFAULT_TIMEOUT_SECS: u64 = 300;
/// Recent scrollback window read per poll. Bounded well under the client's
/// 256 KiB response cap.
const READ_WINDOW_LINES: u64 = 500;

/// How a multi-pane selector is satisfied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatchMode {
    /// Exactly one pane must match the selector (ambiguity is an error).
    Single,
    /// Succeed when ANY matching pane matches the pattern.
    Any,
    /// Succeed when ALL matching panes match the pattern.
    All,
}

enum PaneState {
    /// The regex matched. Carries the matching line(s) from the read window so
    /// `wait` can surface them (US-013 AC2). May be empty if the match spans
    /// lines: the done-decision uses the full window, the line list is a
    /// per-line best-effort for display.
    Matched(Vec<String>),
    NoMatch,
    Gone,
}

/// `paneflow wait --match <sel> --pattern <regex> [--timeout N] [--any|--all]`.
pub fn wait(
    client: &impl IpcTransport,
    target: &str,
    pattern: &str,
    timeout_secs: Option<u64>,
    mode: MatchMode,
) -> Result<i32, CliError> {
    let re = Regex::new(pattern)
        .map_err(|e| CliError::runtime(format!("invalid regex '{pattern}': {e}")))?;

    // Snapshot the target set once. Single mode requires a unique match
    // (ambiguity is an error, consistent with read/search); any/all watch the
    // whole matching set.
    let ids: Vec<u64> = match mode {
        MatchMode::Single => vec![resolve_target(client, target)?],
        MatchMode::Any | MatchMode::All => resolve_all(client, target)?,
    };

    let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
    let deadline = Instant::now() + timeout;

    loop {
        let mut matched_ids: Vec<u64> = Vec::new();
        let mut matches_out: Vec<Value> = Vec::new();
        let mut alive = 0usize;
        for &id in &ids {
            match read_matches(client, id, &re)? {
                PaneState::Matched(lines) => {
                    alive += 1;
                    matched_ids.push(id);
                    matches_out.push(json!({ "surface_id": id, "lines": lines }));
                }
                PaneState::NoMatch => alive += 1,
                PaneState::Gone => {}
            }
        }

        if is_done(mode, matched_ids.len(), ids.len()) {
            super::print_json(
                &json!({ "matched": true, "panes": matched_ids, "matches": matches_out }),
            )?;
            return Ok(EXIT_OK);
        }

        // Every watched pane closed: no outcome is reachable (US-014 defined
        // behavior — fail rather than spin to the deadline).
        if alive == 0 {
            return Err(CliError::runtime(
                "all target panes closed before the pattern appeared",
            ));
        }

        if Instant::now() >= deadline {
            eprintln!(
                "paneflow: timeout after {}s waiting for /{}/",
                timeout.as_secs(),
                pattern
            );
            return Ok(EXIT_TIMEOUT);
        }
        sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

/// Pure outcome rule: is the wait satisfied given how many panes matched out of
/// the watched set?
fn is_done(mode: MatchMode, matched: usize, total: usize) -> bool {
    match mode {
        MatchMode::Single | MatchMode::Any => matched > 0,
        MatchMode::All => matched == total,
    }
}

fn read_matches(client: &impl IpcTransport, id: u64, re: &Regex) -> Result<PaneState, CliError> {
    match client.call(
        "surface.read",
        json!({ "surface_id": id, "lines": READ_WINDOW_LINES }),
    ) {
        Ok(result) => {
            let text = result.get("text").and_then(Value::as_str).unwrap_or("");
            // Decide on the full window (a regex may span lines), but surface
            // the individual matching lines for the caller (US-013 AC2).
            Ok(if re.is_match(text) {
                let hits = text
                    .lines()
                    .filter(|l| re.is_match(l))
                    .map(str::to_string)
                    .collect();
                PaneState::Matched(hits)
            } else {
                PaneState::NoMatch
            })
        }
        // A down instance is fatal — propagate the "is Paneflow running?" error.
        Err(e) if e.contains("unreachable") => Err(CliError::runtime(e)),
        // Anything else (e.g. -32602 surface not found) means the pane closed.
        Err(_) => Ok(PaneState::Gone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_done_single_and_any_need_one_match() {
        assert!(!is_done(MatchMode::Single, 0, 1));
        assert!(is_done(MatchMode::Single, 1, 1));
        assert!(!is_done(MatchMode::Any, 0, 3));
        assert!(is_done(MatchMode::Any, 1, 3));
    }

    #[test]
    fn is_done_all_needs_every_pane() {
        assert!(!is_done(MatchMode::All, 2, 3));
        assert!(is_done(MatchMode::All, 3, 3));
    }

    /// A transport that must never be reached (the invalid-regex guard returns
    /// before any IPC call).
    struct NeverCalled;
    impl IpcTransport for NeverCalled {
        fn call(&self, _: &str, _: Value) -> Result<Value, String> {
            Err("transport should not be called".to_string())
        }
    }

    #[test]
    fn invalid_regex_fails_before_any_ipc_call() {
        let err = wait(&NeverCalled, "x", "(unclosed", None, MatchMode::Single).unwrap_err();
        assert!(
            err.message.contains("invalid regex"),
            "got: {}",
            err.message
        );
    }

    /// Fake transport for the poll loop: `surface.list` resolves the selector to
    /// one pane; `surface.read` returns `read_text` (or a "not found" error,
    /// modelling a closed pane). No real socket, no sleeps on the tested paths
    /// (each case resolves on the first poll).
    struct FakeWait {
        read_text: Option<&'static str>,
    }
    impl IpcTransport for FakeWait {
        fn call(&self, method: &str, _params: Value) -> Result<Value, String> {
            match method {
                "surface.list" => Ok(json!({
                    "surfaces": [{ "surface_id": 1u64, "name": "agent", "cmd": "claude", "cwd": "/tmp" }]
                })),
                "surface.read" => match self.read_text {
                    Some(t) => Ok(json!({ "text": t })),
                    None => Err("paneflow error -32602: surface_id 1 not found".to_string()),
                },
                other => Err(format!("unexpected method {other}")),
            }
        }
    }

    #[test]
    fn wait_succeeds_and_surfaces_matched_line() {
        // Matches on the first poll -> EXIT_OK with no sleep. The matched line
        // is surfaced (US-013 AC2): read_matches collects it from the window.
        let fake = FakeWait {
            read_text: Some("compiling...\nBuild DONE in 3s\n"),
        };
        let code = wait(&fake, "1", "DONE", Some(5), MatchMode::Single).expect("ok");
        assert_eq!(code, EXIT_OK);
    }

    #[test]
    fn wait_times_out_with_dedicated_code() {
        // No match + a zero timeout -> the first deadline check fires, returning
        // the dedicated EXIT_TIMEOUT (distinct from EXIT_TARGET / EXIT_RUNTIME).
        let fake = FakeWait {
            read_text: Some("still working\n"),
        };
        let code = wait(&fake, "1", "DONE", Some(0), MatchMode::Single).expect("ok");
        assert_eq!(code, EXIT_TIMEOUT);
    }

    #[test]
    fn wait_fails_fast_when_target_pane_gone() {
        // surface.read errors (not "unreachable") -> the pane is treated as Gone;
        // with the whole watched set gone, wait fails fast instead of spinning.
        let fake = FakeWait { read_text: None };
        let err = wait(&fake, "1", "DONE", Some(30), MatchMode::Single).unwrap_err();
        assert!(err.message.contains("closed"), "got: {}", err.message);
    }
}
