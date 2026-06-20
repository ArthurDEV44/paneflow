//! `paneflow wait` - block until a regex appears in a pane (US-013/US-014/US-015).
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

use paneflow_ipc_client::{IpcClient, IpcTransport, StreamEvent};
use regex::Regex;
use serde_json::{Value, json};

use super::selector::{resolve_all, resolve_target};
use super::{CliError, EXIT_OK, EXIT_TIMEOUT};

const POLL_INTERVAL_MS: u64 = 500;
const DEFAULT_TIMEOUT_SECS: u64 = 300;
/// EP-003 US-007: default quiescence window for `wait --idle` when `--for` is
/// omitted. 1 s of no `output_generation` change reads as "the turn settled"
/// without false-positiving on a brief silence mid-turn (the skill combines a
/// sentinel `--pattern` for the agent that "thinks" silently longer).
const DEFAULT_IDLE_FOR_MS: u64 = 1000;
/// Recv-timeout slice for the idle subscription. Caps the detection latency at
/// `--for + IDLE_SLICE` (NFR: `<= for + 100 ms`) because the slice - not server
/// events - drives the quiescence clock even when the pane is wholly silent.
const IDLE_SLICE_CAP_MS: u64 = 100;
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
        // behavior - fail rather than spin to the deadline).
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
        // EP-003 US-011: `wait` regex-matches raw scrollback; the untrusted
        // fence wrapper would corrupt the match window, so opt out of it.
        json!({ "surface_id": id, "lines": READ_WINDOW_LINES, "fenced": false }),
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
        // A down instance is fatal - propagate the "is Paneflow running?" error.
        Err(e) if e.contains("unreachable") => Err(CliError::runtime(e)),
        // Anything else (e.g. -32602 surface not found) means the pane closed.
        Err(_) => Ok(PaneState::Gone),
    }
}

// ---------------------------------------------------------------------------
// EP-003 US-007/US-008: `wait --idle` - block on output quiescence via the
// pushed `surface_changed` stream, with zero client-side polling.
// ---------------------------------------------------------------------------

/// What a single event (or recv-timeout slice) signals to the idle wait.
#[derive(Clone, Copy, Debug, PartialEq)]
enum IdleSignal {
    /// New output (`surface_changed`) or a backpressure `dropped` marker: the
    /// pane is NOT quiescent; the caller resets the quiet clock.
    Activity,
    /// A liveness line (`heartbeat` / `subscribed` ack / unknown): ignore - it
    /// proves the server is alive but is not output, so it neither resets the
    /// clock nor triggers an idle check.
    Quiet,
    /// The recv slice elapsed with no complete line: check whether the pane has
    /// been quiet for the full window.
    Tick,
    /// EOF / socket error: the server vanished.
    Closed,
}

/// The verdict for one loop iteration of the idle wait.
#[derive(Clone, Copy, Debug, PartialEq)]
enum IdleOutcome {
    /// Keep waiting.
    Continue,
    /// Quiescent for the window (or the sentinel matched): exit 0.
    Idle,
    /// The subscription died before idle: exit 1, never a hang (US-008 AC2).
    Dead,
    /// The overall `--timeout` elapsed without idle: exit 4 (US-007 AC2).
    TimedOut,
}

/// Pure quiescence rule, factored so the exit-code matrix (US-008 AC1) and the
/// dead-detection (AC2) are unit-tested without a socket. `since_change` is the
/// elapsed time since the last `Activity`; `for_window` is `--for`; the loop
/// passes `past_deadline` for the overall `--timeout`. Idle on a tick wins over
/// the deadline (a wait that just succeeded is a success, not a timeout); a
/// dead stream wins over everything.
fn idle_decision(
    sig: IdleSignal,
    since_change: Duration,
    for_window: Duration,
    past_deadline: bool,
) -> IdleOutcome {
    match sig {
        IdleSignal::Closed => IdleOutcome::Dead,
        IdleSignal::Tick => {
            if since_change >= for_window {
                IdleOutcome::Idle
            } else if past_deadline {
                IdleOutcome::TimedOut
            } else {
                IdleOutcome::Continue
            }
        }
        IdleSignal::Activity | IdleSignal::Quiet => {
            if past_deadline {
                IdleOutcome::TimedOut
            } else {
                IdleOutcome::Continue
            }
        }
    }
}

/// Map a server event line to its [`IdleSignal`] by its `type` field. Anything
/// that is not a known output-bearing event (heartbeat, subscribed ack, garbage)
/// is `Quiet`, so a malformed line can never be mistaken for activity.
fn classify_event_line(line: &str) -> IdleSignal {
    let kind = serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_owned));
    match kind.as_deref() {
        Some("surface_changed") | Some("dropped") => IdleSignal::Activity,
        _ => IdleSignal::Quiet,
    }
}

/// Best-effort "does the pane's recent scrollback match `re` right now?". Any
/// read failure (pane gone, server unreachable) reads as `false`; a vanished
/// server is caught by the subscription's `Closed` event instead.
fn pane_matches(client: &impl IpcTransport, id: u64, re: &Regex) -> bool {
    matches!(read_matches(client, id, re), Ok(PaneState::Matched(_)))
}

/// `paneflow wait --idle <sel> [--for <ms>] [--timeout <s>] [--pattern <re>]`.
///
/// Subscribes to the pane's `surface_changed` push stream and returns exit 0
/// once `output_generation` has been stable for `--for` ms - no client poll of
/// pane content. With `--pattern`, the sentinel is checked on each new output
/// (event-driven) and EITHER signal (pattern match OR quiescence) wins, first
/// to fire (US-008). Exit codes: 0 idle/match, 1 dead stream, 3 no instance /
/// bad selector / unsupported platform, 4 timeout.
///
/// Platform note: quiescence needs a recv-timeout-capable socket. On Linux and
/// macOS (Unix domain socket) that works. On Windows the named pipe rejects the
/// recv timeout AND the push bus is itself Unix-only until EP-006, so `--idle`
/// fails fast (exit 3) with a pointer to the cross-platform `wait --pattern`
/// rather than hanging - a documented stub, not a silent miss.
pub fn wait_idle(
    client: &IpcClient,
    target: &str,
    for_ms: Option<u64>,
    timeout_secs: Option<u64>,
    pattern: Option<&str>,
) -> Result<i32, CliError> {
    let id = resolve_target(client, target)?;
    let re: Option<Regex> = match pattern {
        Some(p) => Some(
            Regex::new(p).map_err(|e| CliError::runtime(format!("invalid regex '{p}': {e}")))?,
        ),
        None => None,
    };
    let window_ms = for_ms.unwrap_or(DEFAULT_IDLE_FOR_MS);
    let for_window = Duration::from_millis(window_ms);
    let slice = Duration::from_millis(window_ms.clamp(1, IDLE_SLICE_CAP_MS));
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
    let deadline = Instant::now() + timeout;

    let socket = paneflow_ipc_client::resolve_socket_path().ok_or_else(|| {
        CliError::target(
            "cannot locate the IPC socket; is Paneflow running? \
             (set PANEFLOW_SOCKET_PATH if you launched the CLI outside a Paneflow pane)",
        )
    })?;

    // Sentinel already present before any new output (the turn finished before
    // we subscribed): succeed immediately (US-008 OR semantics).
    if let Some(re) = &re
        && pane_matches(client, id, re)
    {
        super::print_json(&json!({ "surface_id": id, "idle": false, "matched": true }))?;
        return Ok(EXIT_OK);
    }

    // Ctrl-C is a clean stop; dropping the socket frees the server-side
    // subscription on its next write (RAII), so nothing leaks (US-007 AC4).
    let _ = ctrlc::set_handler(|| std::process::exit(130));

    let params = json!({ "surfaces": [id], "types": ["surface_changed"] });
    let mut since_change = Instant::now();
    let mut outcome = IdleOutcome::Dead;
    let mut matched = false;

    let stream_result = paneflow_ipc_client::subscribe_stream_timed(&socket, params, slice, |ev| {
        let past_deadline = Instant::now() >= deadline;
        let sig = match ev {
            StreamEvent::Line(l) => classify_event_line(l),
            StreamEvent::Tick => IdleSignal::Tick,
            StreamEvent::Closed => IdleSignal::Closed,
        };
        if sig == IdleSignal::Activity {
            // New output is the ONLY moment a sentinel can appear: check it
            // here, event-driven, never on a blind poll. First to fire wins.
            if let Some(re) = &re
                && pane_matches(client, id, re)
            {
                matched = true;
                outcome = IdleOutcome::Idle;
                return false;
            }
            since_change = Instant::now();
        }
        match idle_decision(sig, since_change.elapsed(), for_window, past_deadline) {
            IdleOutcome::Continue => true,
            other => {
                outcome = other;
                false
            }
        }
    });

    match stream_result {
        Ok(()) => match outcome {
            IdleOutcome::Idle => {
                super::print_json(
                    &json!({ "surface_id": id, "idle": !matched, "matched": matched }),
                )?;
                Ok(EXIT_OK)
            }
            IdleOutcome::TimedOut => {
                eprintln!(
                    "paneflow: timeout after {}s waiting for surface {id} to go idle",
                    timeout.as_secs()
                );
                Ok(EXIT_TIMEOUT)
            }
            // The stream died before idle: exit 1 (runtime), not a silent hang.
            IdleOutcome::Dead => Err(CliError::runtime(
                "the Paneflow event stream closed before the pane went idle (did Paneflow exit?)",
            )),
            IdleOutcome::Continue => Err(CliError::runtime(
                "idle wait ended without a verdict (internal)",
            )),
        },
        // A failed connect (no reachable instance) OR an unsupported recv
        // timeout (Windows named pipe) -> exit 3. Both `e` messages are already
        // actionable (start Paneflow / use `wait --pattern`), so surface them
        // verbatim without a misleading "is Paneflow running?" suffix.
        Err(e) => Err(CliError::target(format!("wait --idle failed: {e}"))),
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

    // ---------- EP-003 US-007/US-008: idle quiescence rule ----------

    const FW: Duration = Duration::from_millis(1000);

    #[test]
    fn idle_decision_tick_idles_only_after_window() {
        // US-007 AC1: quiet for >= the window on a tick -> Idle (exit 0).
        assert_eq!(
            idle_decision(IdleSignal::Tick, Duration::from_millis(1000), FW, false),
            IdleOutcome::Idle
        );
        assert_eq!(
            idle_decision(IdleSignal::Tick, Duration::from_millis(1500), FW, false),
            IdleOutcome::Idle
        );
        // Not quiet long enough -> keep waiting.
        assert_eq!(
            idle_decision(IdleSignal::Tick, Duration::from_millis(300), FW, false),
            IdleOutcome::Continue
        );
    }

    #[test]
    fn idle_decision_exit_code_matrix() {
        // US-007 AC2 / US-008 AC1: a spinner never goes quiet; past the overall
        // deadline -> TimedOut (exit 4), on either an activity or a tick.
        assert_eq!(
            idle_decision(IdleSignal::Activity, Duration::from_millis(10), FW, true),
            IdleOutcome::TimedOut
        );
        assert_eq!(
            idle_decision(IdleSignal::Tick, Duration::from_millis(10), FW, true),
            IdleOutcome::TimedOut
        );
        // Idle still wins over the deadline on the same tick (success > timeout).
        assert_eq!(
            idle_decision(IdleSignal::Tick, Duration::from_millis(1000), FW, true),
            IdleOutcome::Idle
        );
        // US-008 AC2: a vanished server -> Dead (exit 1) regardless of timing.
        assert_eq!(
            idle_decision(IdleSignal::Closed, Duration::from_millis(10), FW, false),
            IdleOutcome::Dead
        );
        assert_eq!(
            idle_decision(IdleSignal::Closed, Duration::from_millis(9999), FW, true),
            IdleOutcome::Dead
        );
    }

    #[test]
    fn idle_decision_activity_and_heartbeat_keep_waiting() {
        // Fresh activity (even with a huge stale `since_change`) just continues;
        // the caller resets the clock. A heartbeat is liveness-only, same verdict.
        assert_eq!(
            idle_decision(IdleSignal::Activity, Duration::from_millis(9999), FW, false),
            IdleOutcome::Continue
        );
        assert_eq!(
            idle_decision(IdleSignal::Quiet, Duration::from_millis(9999), FW, false),
            IdleOutcome::Continue
        );
    }

    #[test]
    fn classify_event_line_only_surface_changed_is_activity() {
        assert_eq!(
            classify_event_line(
                r#"{"type":"surface_changed","surface_id":1,"output_generation":5}"#
            ),
            IdleSignal::Activity
        );
        // A backpressure marker means we missed real output - treat as activity.
        assert_eq!(
            classify_event_line(r#"{"type":"dropped","count":2}"#),
            IdleSignal::Activity
        );
        // Liveness lines must NOT reset the quiet clock.
        assert_eq!(
            classify_event_line(r#"{"type":"heartbeat"}"#),
            IdleSignal::Quiet
        );
        assert_eq!(
            classify_event_line(r#"{"type":"subscribed","id":1}"#),
            IdleSignal::Quiet
        );
        // Garbage / missing type -> Quiet (never a false activity).
        assert_eq!(classify_event_line("not json at all"), IdleSignal::Quiet);
        assert_eq!(classify_event_line(r#"{"no":"type"}"#), IdleSignal::Quiet);
    }
}
