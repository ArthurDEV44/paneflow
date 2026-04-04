//! PaneFlow CLI — controls the running PaneFlow app via JSON-RPC over Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand, ValueEnum};

// ---------------------------------------------------------------------------
// CLI argument definitions
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "paneflow",
    version,
    about = "Control PaneFlow from the command line"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage workspaces
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    /// Send text to the active terminal pane
    SendText {
        /// Text to send to the terminal
        text: String,
    },
    /// Split the active workspace's terminal pane
    Split {
        /// Split direction
        direction: SplitDir,
    },
}

#[derive(Clone, ValueEnum)]
enum SplitDir {
    Horizontal,
    Vertical,
}

#[derive(Subcommand)]
enum WorkspaceAction {
    /// List all workspaces
    List,
    /// Create a new workspace
    Create {
        /// Workspace name (default: "Terminal")
        name: Option<String>,
    },
    /// Select a workspace by index
    Select {
        /// Workspace index (0-based)
        id: usize,
    },
}

// ---------------------------------------------------------------------------
// Socket path resolution (mirrors ipc.rs logic)
// ---------------------------------------------------------------------------

fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs::runtime_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime_dir.join("paneflow").join("paneflow.sock")
}

// ---------------------------------------------------------------------------
// JSON-RPC client
// ---------------------------------------------------------------------------

fn send_request(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let path = socket_path();

    let stream = UnixStream::connect(&path).map_err(|_| "PaneFlow is not running".to_string())?;

    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set timeout: {e}"))?;

    let mut writer = stream
        .try_clone()
        .map_err(|e| format!("Clone failed: {e}"))?;
    let reader = BufReader::new(stream);

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });

    let mut request_str = serde_json::to_string(&request).unwrap();
    request_str.push('\n');
    writer
        .write_all(request_str.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    // Read one line response
    let mut response_line = String::new();
    let mut buf_reader = reader;
    buf_reader
        .read_line(&mut response_line)
        .map_err(|e| format!("Read failed: {e}"))?;

    let response: serde_json::Value =
        serde_json::from_str(&response_line).map_err(|e| format!("Invalid response: {e}"))?;

    // Check JSON-RPC level error
    if let Some(error) = response.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error");
        return Err(msg.to_string());
    }

    let result = response
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Check application-level error (server returns {"error": "..."} inside result)
    if let Some(err_msg) = result.get("error").and_then(|e| e.as_str()) {
        return Err(err_msg.to_string());
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn print_workspaces(result: &serde_json::Value) {
    if let Some(workspaces) = result.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            let idx = ws.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
            let title = ws.get("title").and_then(|t| t.as_str()).unwrap_or("?");
            let cwd = ws.get("cwd").and_then(|c| c.as_str()).unwrap_or("");
            let panes = ws.get("panes").and_then(|p| p.as_u64()).unwrap_or(0);
            let active = ws.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
            let marker = if active { " *" } else { "" };
            println!("{idx}: {title} ({panes} panes) {cwd}{marker}");
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Workspace { action } => match action {
            WorkspaceAction::List => {
                let res = send_request("workspace.list", serde_json::json!({}));
                match res {
                    Ok(val) => {
                        print_workspaces(&val);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            WorkspaceAction::Create { name } => {
                let params = match name {
                    Some(ref n) => serde_json::json!({"name": n}),
                    None => serde_json::json!({}),
                };
                send_request("workspace.create", params).map(|val| {
                    let idx = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let title = val.get("title").and_then(|t| t.as_str()).unwrap_or("?");
                    println!("Created workspace {idx}: {title}");
                })
            }
            WorkspaceAction::Select { id } => {
                send_request("workspace.select", serde_json::json!({"index": id})).map(|_| {
                    println!("Selected workspace {id}");
                })
            }
        },
        Commands::SendText { text } => {
            send_request("surface.send_text", serde_json::json!({"text": text})).map(|val| {
                let len = val.get("length").and_then(|l| l.as_u64()).unwrap_or(0);
                println!("Sent {len} bytes");
            })
        }
        Commands::Split { direction } => {
            let dir = match direction {
                SplitDir::Horizontal => "horizontal",
                SplitDir::Vertical => "vertical",
            };
            send_request("surface.split", serde_json::json!({"direction": dir})).map(|_| {
                println!("Split {dir}");
            })
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
