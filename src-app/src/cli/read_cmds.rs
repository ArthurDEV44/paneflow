//! Read surface of the CLI: `ls` / `read` / `search` (US-004).
//!
//! Thin wrappers over the existing `surface.list` / `surface.read` /
//! `surface.search` IPC methods. Introspection (`ls`, `search`) emits JSON by
//! default for scripts; `read` prints the raw scrollback text by default (it's
//! terminal output, not structured data) and only wraps it in the
//! `{text, lines, total_lines, eof}` envelope under `--json`.

use paneflow_ipc_client::IpcTransport;
use serde_json::{Value, json};

use super::selector::resolve_target;
use super::{CliError, EXIT_OK};

/// `paneflow ls [--human]` — list the active workspace's surfaces.
pub fn ls(client: &impl IpcTransport, human: bool) -> Result<i32, CliError> {
    let result = client
        .call("surface.list", json!({}))
        .map_err(CliError::runtime)?;
    if human {
        print_surfaces_table(&result);
    } else {
        super::print_json(&result)?;
    }
    Ok(EXIT_OK)
}

/// `paneflow read <target> [--lines N] [--offset N] [--json]`.
pub fn read(
    client: &impl IpcTransport,
    target: &str,
    lines: Option<u64>,
    offset: Option<u64>,
    json_out: bool,
) -> Result<i32, CliError> {
    let surface_id = resolve_target(client, target)?;
    let mut params = json!({ "surface_id": surface_id });
    if let Some(lines) = lines {
        params["lines"] = json!(lines);
    }
    if let Some(offset) = offset {
        params["offset"] = json!(offset);
    }
    let result = client
        .call("surface.read", params)
        .map_err(CliError::runtime)?;
    if json_out {
        super::print_json(&result)?;
    } else {
        // Raw scrollback: print verbatim (it carries its own newlines). No
        // trailing newline is appended so the output round-trips exactly.
        let text = result.get("text").and_then(Value::as_str).unwrap_or("");
        print!("{text}");
    }
    Ok(EXIT_OK)
}

/// `paneflow search <target> <pattern> [--max N] [--human]`.
pub fn search(
    client: &impl IpcTransport,
    target: &str,
    pattern: &str,
    max: Option<u64>,
    human: bool,
) -> Result<i32, CliError> {
    let surface_id = resolve_target(client, target)?;
    let mut params = json!({ "surface_id": surface_id, "pattern": pattern });
    if let Some(max) = max {
        params["max_matches"] = json!(max);
    }
    let result = client
        .call("surface.search", params)
        .map_err(CliError::runtime)?;
    if human {
        if let Some(matches) = result.get("matches").and_then(Value::as_array) {
            for m in matches {
                let line = m.get("line").and_then(Value::as_u64).unwrap_or(0);
                let text = m.get("text").and_then(Value::as_str).unwrap_or("");
                println!("{line}: {text}");
            }
        }
    } else {
        super::print_json(&result)?;
    }
    Ok(EXIT_OK)
}

fn print_surfaces_table(result: &Value) {
    let surfaces = result.get("surfaces").and_then(Value::as_array);
    let Some(surfaces) = surfaces else {
        println!("(no surfaces)");
        return;
    };
    if surfaces.is_empty() {
        println!("(no surfaces)");
        return;
    }
    println!("{:>4}  {:<20}  {:<24}  CWD", "ID", "NAME", "CMD");
    for s in surfaces {
        let id = s.get("surface_id").and_then(Value::as_u64).unwrap_or(0);
        let name = s.get("name").and_then(Value::as_str).unwrap_or("");
        let cmd = s.get("cmd").and_then(Value::as_str).unwrap_or("");
        let cwd = s.get("cwd").and_then(Value::as_str).unwrap_or("");
        println!("{id:>4}  {name:<20}  {cmd:<24}  {cwd}");
    }
}
