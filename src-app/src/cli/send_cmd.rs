//! `send` / `key` - write-side CLI verbs (US-006; orchestration-v2
//! US-003/US-004/US-005).
//!
//! Wraps `surface.send_text` / `surface.send_keystroke`. The human-in-loop
//! invariant is enforced server side: `send_text` writes the bytes verbatim
//! with no trailing carriage return, so the text lands in the agent's input
//! box and the user/agent presses Enter themselves - UNLESS `--submit` is
//! passed explicitly (US-005), the only sanctioned submission path.
//! `key` refuses submitting keystrokes (`enter`, `ctrl-m`, …) server-side.
//!
//! Both methods are gated behind `PANEFLOW_IPC_SCRIPTING=1` on the RUNNING
//! instance (the gate is read from Paneflow's own process env, not the
//! CLI's), so when it is off the server returns `-32601` and we translate
//! that into an actionable hint rather than a bare JSON-RPC code.

use paneflow_ipc_client::IpcTransport;
use serde_json::json;

use super::selector::{resolve_all, resolve_target};
use super::{CliError, EXIT_OK, EXIT_RUNTIME};

/// `paneflow send <target> <text> [--broadcast] [--submit] [--paste]`.
pub fn send(
    client: &impl IpcTransport,
    target: &str,
    text: &str,
    broadcast: bool,
    submit: bool,
    paste: bool,
) -> Result<i32, CliError> {
    if broadcast {
        return send_broadcast(client, target, text, submit, paste);
    }
    let surface_id = resolve_target(client, target)?;
    match send_to(client, surface_id, text, submit, paste) {
        Ok(result) => {
            super::print_json(&result)?;
            Ok(EXIT_OK)
        }
        Err(e) => Err(e),
    }
}

/// One `surface.send_text` round for a resolved surface. Maps the legacy
/// `{"error": …}` result shape (empty text / >64 KiB / surface gone) to a
/// non-zero `CliError` (US-006 AC3) and the `-32601` gate-off reply to an
/// actionable hint.
fn send_to(
    client: &impl IpcTransport,
    surface_id: u64,
    text: &str,
    submit: bool,
    paste: bool,
) -> Result<serde_json::Value, CliError> {
    let mut params = json!({ "surface_id": surface_id, "text": text, "submit": submit });
    // EP-001 US-002: only forward `paste` when the user explicitly passed
    // `--paste`. Absent, the server auto-decides (agent pane -> bracketed paste
    // + deferred submit, bare shell -> verbatim); sending an explicit `false`
    // here would instead PIN the verbatim path and defeat the auto-detection.
    if paste {
        params["paste"] = json!(true);
    }
    match client.call("surface.send_text", params) {
        Ok(result) => super::reject_legacy_error(result),
        // The scripting gate is off on the running instance.
        Err(e) if e.contains("-32601") => Err(CliError::runtime(format!(
            "send is disabled on the running Paneflow instance; relaunch it with \
             PANEFLOW_IPC_SCRIPTING=1 to enable text injection (server said: {e})"
        ))),
        Err(e) => Err(CliError::runtime(e)),
    }
}

/// `send --broadcast`: every pane matching the selector gets the text. A pane
/// failing mid-loop (closed between resolve and send) does not abort the
/// rest; the report lists both sets and the exit is non-zero when anything
/// failed (US-003). The gate-off error DOES abort: every send would fail the
/// same way, so the actionable hint surfaces immediately instead of N times.
fn send_broadcast(
    client: &impl IpcTransport,
    target: &str,
    text: &str,
    submit: bool,
    paste: bool,
) -> Result<i32, CliError> {
    let ids = resolve_all(client, target)?;
    let mut sent: Vec<u64> = Vec::new();
    let mut failed: Vec<serde_json::Value> = Vec::new();
    for id in ids {
        match send_to(client, id, text, submit, paste) {
            Ok(_) => sent.push(id),
            // Gate off: abort with the hint; nothing was partially injected
            // (the very first call already failed the same way).
            Err(e) if e.message.contains("PANEFLOW_IPC_SCRIPTING") && sent.is_empty() => {
                return Err(e);
            }
            Err(e) => failed.push(json!({ "surface_id": id, "error": e.message })),
        }
    }
    let all_ok = failed.is_empty();
    super::print_json(&json!({ "sent": sent, "failed": failed, "submitted": submit }))?;
    Ok(if all_ok { EXIT_OK } else { EXIT_RUNTIME })
}

/// `paneflow key <target> <keystroke>`. Wraps `surface.send_keystroke`; the
/// server refuses CR/LF-resolving keystrokes (`enter`, `ctrl-m`, `ctrl-j`) so
/// submission stays exclusive to `send --submit` (US-004/US-005).
pub fn key(client: &impl IpcTransport, target: &str, keystroke: &str) -> Result<i32, CliError> {
    let surface_id = resolve_target(client, target)?;
    match client.call(
        "surface.send_keystroke",
        json!({ "surface_id": surface_id, "keystroke": keystroke }),
    ) {
        Ok(result) => {
            let result = super::reject_legacy_error(result)?;
            super::print_json(&result)?;
            Ok(EXIT_OK)
        }
        Err(e) if e.contains("-32601") => Err(CliError::runtime(format!(
            "key is disabled on the running Paneflow instance; relaunch it with \
             PANEFLOW_IPC_SCRIPTING=1 to enable keystroke injection (server said: {e})"
        ))),
        Err(e) => Err(CliError::runtime(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::cell::RefCell;

    /// Method-routed fake: fixed `surface.list`, scripted per-call replies for
    /// the write methods (popped front-to-back), and a (method, params) log.
    struct ScriptedTransport {
        calls: RefCell<Vec<(String, Value)>>,
        replies: RefCell<Vec<Result<Value, String>>>,
    }
    impl ScriptedTransport {
        fn new(replies: Vec<Result<Value, String>>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                replies: RefCell::new(replies),
            }
        }
    }
    impl IpcTransport for ScriptedTransport {
        fn call(&self, method: &str, params: Value) -> Result<Value, String> {
            if method == "surface.list" {
                return Ok(json!({ "surfaces": [
                    { "surface_id": 12, "name": "shard-api" },
                    { "surface_id": 18, "name": "shard-ui" },
                ]}));
            }
            self.calls
                .borrow_mut()
                .push((method.to_string(), params.clone()));
            let mut replies = self.replies.borrow_mut();
            if replies.is_empty() {
                return Ok(json!({ "sent": true }));
            }
            replies.remove(0)
        }
    }

    #[test]
    fn send_passes_submit_flag_through() {
        let fake = ScriptedTransport::new(vec![Ok(json!({ "sent": true, "submitted": true }))]);
        assert_eq!(
            send(&fake, "shard-api", "run", false, true, false).expect("ok"),
            EXIT_OK
        );
        let calls = fake.calls.borrow();
        assert_eq!(calls[0].0, "surface.send_text");
        assert_eq!(calls[0].1["submit"], true);
        assert_eq!(calls[0].1["surface_id"], 12);
        // EP-001 US-002: no `--paste` -> the key is omitted so the server
        // auto-decides agent-vs-shell rather than being pinned to verbatim.
        assert!(calls[0].1.get("paste").is_none());
    }

    #[test]
    fn send_default_is_not_submitting() {
        let fake = ScriptedTransport::new(vec![Ok(json!({ "sent": true }))]);
        send(&fake, "shard-api", "run", false, false, false).expect("ok");
        assert_eq!(fake.calls.borrow()[0].1["submit"], false);
    }

    #[test]
    fn paste_flag_is_forwarded_only_when_set() {
        // EP-001 US-002 AC2: `--paste` -> the param rides through to the server.
        let fake = ScriptedTransport::new(vec![Ok(json!({ "sent": true, "paste": true }))]);
        send(&fake, "shard-api", "hi", false, true, true).expect("ok");
        let calls = fake.calls.borrow();
        assert_eq!(calls[0].1["paste"], true);
        assert_eq!(calls[0].1["submit"], true);
    }

    #[test]
    fn send_multi_match_without_broadcast_is_target_error() {
        // "shard" prefixes both panes: single-target send must refuse, not
        // pick one silently (US-003 keeps the existing single semantics).
        let fake = ScriptedTransport::new(vec![]);
        let err = send(&fake, "shard", "x", false, false, false).expect_err("ambiguous");
        assert_eq!(err.code, crate::cli::EXIT_TARGET);
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn broadcast_hits_every_match() {
        let fake = ScriptedTransport::new(vec![
            Ok(json!({ "sent": true })),
            Ok(json!({ "sent": true })),
        ]);
        assert_eq!(
            send(&fake, "shard", "x", true, false, false).expect("ok"),
            EXIT_OK
        );
        let calls = fake.calls.borrow();
        let ids: Vec<&Value> = calls.iter().map(|(_, p)| &p["surface_id"]).collect();
        assert_eq!(ids, vec![&json!(12), &json!(18)]);
    }

    #[test]
    fn broadcast_partial_failure_serves_the_rest_and_exits_nonzero() {
        // First pane vanished mid-loop (legacy error shape): the second pane
        // must still be served and the exit must be non-zero (US-003 AC4).
        let fake = ScriptedTransport::new(vec![
            Ok(json!({ "error": "Surface not found" })),
            Ok(json!({ "sent": true })),
        ]);
        let code = send(&fake, "shard", "x", true, false, false).expect("report, not abort");
        assert_eq!(code, EXIT_RUNTIME);
        assert_eq!(fake.calls.borrow().len(), 2, "second pane still served");
    }

    #[test]
    fn broadcast_no_match_is_target_error() {
        let fake = ScriptedTransport::new(vec![]);
        let err = send(&fake, "zzz", "x", true, false, false).expect_err("no match");
        assert_eq!(err.code, crate::cli::EXIT_TARGET);
        assert!(fake.calls.borrow().is_empty(), "no partial send");
    }

    #[test]
    fn broadcast_gate_off_aborts_with_actionable_hint() {
        let fake = ScriptedTransport::new(vec![Err(
            "server error -32601: surface.send_text disabled".to_string(),
        )]);
        let err = send(&fake, "shard", "x", true, false, false).expect_err("gate off");
        assert_eq!(err.code, EXIT_RUNTIME);
        assert!(err.message.contains("PANEFLOW_IPC_SCRIPTING"));
        assert_eq!(fake.calls.borrow().len(), 1, "aborted after first reply");
    }

    #[test]
    fn key_translates_gate_off_and_passes_keystroke() {
        let fake = ScriptedTransport::new(vec![Err(
            "server error -32601: surface.send_keystroke disabled".to_string(),
        )]);
        let err = key(&fake, "shard-api", "escape").expect_err("gate off");
        assert!(err.message.contains("PANEFLOW_IPC_SCRIPTING"));

        let fake = ScriptedTransport::new(vec![Ok(json!({ "sent": true }))]);
        assert_eq!(key(&fake, "shard-api", "escape").expect("ok"), EXIT_OK);
        let calls = fake.calls.borrow();
        assert_eq!(calls[0].0, "surface.send_keystroke");
        assert_eq!(calls[0].1["keystroke"], "escape");
    }

    #[test]
    fn key_enter_refusal_is_nonzero_exit() {
        // The server refuses submitting keystrokes with a legacy error shape
        // (TerminalView::send_keystroke -> {"error": …}); the CLI must exit
        // non-zero and surface the `send --submit` hint (US-004 AC3).
        let fake = ScriptedTransport::new(vec![Ok(
            json!({ "error": "keystroke 'enter' would submit (CR/LF); use surface.send_text with submit=true (`paneflow send --submit`) instead" }),
        )]);
        let err = key(&fake, "shard-api", "enter").expect_err("refused");
        assert_eq!(err.code, EXIT_RUNTIME);
        assert!(err.message.contains("send --submit"), "hint present");
    }

    #[test]
    fn submit_forwards_a_full_64_kib_payload_intact() {
        // US-005 AC6: the one explicitly-mandated stress case - a 64 KiB text
        // submitted in a single round. The server enforces the 64 KiB ceiling
        // and appends the `\r` after the last byte (ipc_handler.rs:1344-1363);
        // the CLI's job, pinned here, is to forward the max-size payload WHOLE
        // (no client-side chunking/truncation) with `submit:true`, so the lone
        // server-side CR lands after the complete text rather than mid-stream.
        let payload = "x".repeat(64 * 1024);
        let fake = ScriptedTransport::new(vec![Ok(json!({
            "sent": true, "length": payload.len(), "submitted": true
        }))]);
        assert_eq!(
            send(&fake, "shard-api", &payload, false, true, false).expect("ok"),
            EXIT_OK
        );
        let calls = fake.calls.borrow();
        assert_eq!(calls[0].0, "surface.send_text");
        assert_eq!(calls[0].1["submit"], true);
        assert_eq!(
            calls[0].1["text"].as_str().map(str::len),
            Some(64 * 1024),
            "the 64 KiB payload must reach the server intact, not chunked"
        );
    }
}
