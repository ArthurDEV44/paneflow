//! Control surface of the CLI: `new` / `select` / `split` (US-005).
//!
//! Thin wrappers over `workspace.create` / `workspace.select` / `surface.split`.
//! Each prints the server's `result` envelope as JSON so scripts can read back
//! the new workspace index / pane count. Server-side caps (MAX_WORKSPACES,
//! MAX_PANES) and validation (a non-existent `--cwd` is rejected with -32602)
//! propagate as a clear message + non-zero exit.

use paneflow_ipc_client::IpcTransport;
use serde_json::json;

use super::{CliError, EXIT_OK};

/// `paneflow new [--name N] [--cwd DIR]`.
pub fn new_workspace(
    client: &impl IpcTransport,
    name: Option<&str>,
    cwd: Option<&str>,
) -> Result<i32, CliError> {
    let mut params = json!({});
    if let Some(name) = name {
        params["name"] = json!(name);
    }
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    let result = client
        .call("workspace.create", params)
        .map_err(CliError::runtime)?;
    super::print_json(&result)?;
    Ok(EXIT_OK)
}

/// `paneflow select <index>`.
pub fn select(client: &impl IpcTransport, index: u64) -> Result<i32, CliError> {
    let result = client
        .call("workspace.select", json!({ "index": index }))
        .map_err(CliError::runtime)?;
    super::print_json(&result)?;
    Ok(EXIT_OK)
}

/// `paneflow split <h|v>`. `direction` is the IPC string ("horizontal" |
/// "vertical"), already resolved from the `SplitDir` value enum by the caller.
pub fn split(client: &impl IpcTransport, direction: &str) -> Result<i32, CliError> {
    let result = client
        .call("surface.split", json!({ "direction": direction }))
        .map_err(CliError::runtime)?;
    super::print_json(&result)?;
    Ok(EXIT_OK)
}
