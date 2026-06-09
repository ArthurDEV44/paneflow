//! `send` — inject text into a pane WITHOUT submitting it (US-006).
//!
//! Wraps `surface.send_text`. The human-in-loop invariant is enforced server
//! side: `send_text` writes the bytes verbatim with no trailing carriage
//! return, so the text lands in the agent's input box and the user/agent
//! presses Enter themselves — the CLI never submits on their behalf.
//!
//! `surface.send_text` is gated behind `PANEFLOW_IPC_SCRIPTING=1` on the
//! RUNNING instance (the gate is read from Paneflow's own process env, not the
//! CLI's), so when it is off the server returns `-32601` and we translate that
//! into an actionable hint rather than a bare JSON-RPC code.

use paneflow_ipc_client::IpcTransport;
use serde_json::json;

use super::selector::resolve_target;
use super::{CliError, EXIT_OK};

/// `paneflow send <target> <text>`.
pub fn send(client: &impl IpcTransport, target: &str, text: &str) -> Result<i32, CliError> {
    let surface_id = resolve_target(client, target)?;
    match client.call(
        "surface.send_text",
        json!({ "surface_id": surface_id, "text": text }),
    ) {
        Ok(result) => {
            super::print_json(&result)?;
            Ok(EXIT_OK)
        }
        // The scripting gate is off on the running instance.
        Err(e) if e.contains("-32601") => Err(CliError::runtime(format!(
            "send is disabled on the running Paneflow instance; relaunch it with \
             PANEFLOW_IPC_SCRIPTING=1 to enable text injection (server said: {e})"
        ))),
        Err(e) => Err(CliError::runtime(e)),
    }
}
