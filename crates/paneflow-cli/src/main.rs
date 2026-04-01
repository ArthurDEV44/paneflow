mod client;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use client::{resolve_socket, JsonRpcResponse, SocketClient};

/// PaneFlow — a modern terminal multiplexer.
#[derive(Parser)]
#[command(name = "paneflow", version, about, long_about = None)]
struct Cli {
    /// Path to the PaneFlow daemon socket (overrides auto-discovery).
    #[arg(long = "socket", global = true)]
    socket: Option<PathBuf>,

    /// Emit output as JSON instead of human-readable text.
    #[arg(long = "json", global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, ValueEnum)]
enum SplitDirection {
    Right,
    Down,
}

impl std::fmt::Display for SplitDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Right => write!(f, "right"),
            Self::Down => write!(f, "down"),
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Send a ping to the running PaneFlow daemon.
    Ping,

    /// List all workspaces managed by the daemon.
    ListWorkspaces,

    /// Create a new workspace.
    NewWorkspace {
        /// Human-readable name for the workspace.
        #[arg(long)]
        name: Option<String>,

        /// Working directory for the workspace.
        #[arg(long)]
        cwd: Option<String>,

        /// Initial command to run in the workspace.
        #[arg(long)]
        command: Option<String>,
    },

    /// Switch focus to a workspace by ID.
    SelectWorkspace {
        /// Workspace ID to select.
        id: String,
    },

    /// Close a workspace by ID.
    CloseWorkspace {
        /// Workspace ID to close.
        id: String,
    },

    /// Send text to a surface (terminal pane).
    Send {
        /// Target surface ID.
        #[arg(long)]
        surface: String,

        /// Text to send to the surface.
        #[arg(long)]
        text: String,
    },

    /// Create a new split in the current or specified surface.
    NewSplit {
        /// Surface ID to split (defaults to the focused surface).
        #[arg(long)]
        surface: Option<String>,

        /// Split direction.
        #[arg(long, value_enum, default_value = "right")]
        direction: SplitDirection,
    },

    /// List surfaces (terminal panes), optionally filtered by workspace.
    ListSurfaces {
        /// Filter surfaces by workspace ID.
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Print the currently focused context (workspace, surface, cwd).
    Identify,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        eprintln!("No command specified. Run `paneflow --help` for usage.");
        return ExitCode::FAILURE;
    };

    let ctx = CmdContext {
        socket_override: cli.socket,
        json_output: cli.json,
    };

    match command {
        Command::Ping => handle_ping(&ctx).await,
        Command::ListWorkspaces => handle_list_workspaces(&ctx).await,
        Command::NewWorkspace { name, cwd, command } => {
            handle_new_workspace(&ctx, name, cwd, command).await
        }
        Command::SelectWorkspace { id } => handle_select_workspace(&ctx, &id).await,
        Command::CloseWorkspace { id } => handle_close_workspace(&ctx, &id).await,
        Command::Send { surface, text } => handle_send(&ctx, &surface, &text).await,
        Command::NewSplit { surface, direction } => {
            handle_new_split(&ctx, surface.as_deref(), &direction).await
        }
        Command::ListSurfaces { workspace } => {
            handle_list_surfaces(&ctx, workspace.as_deref()).await
        }
        Command::Identify => handle_identify(&ctx).await,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Common context threaded through every command handler.
struct CmdContext {
    socket_override: Option<PathBuf>,
    json_output: bool,
}

/// Create a `SocketClient` from the shared context, printing errors as needed.
fn make_client(ctx: &CmdContext) -> Result<SocketClient, ExitCode> {
    match resolve_socket(ctx.socket_override.as_deref()) {
        Ok(p) => Ok(SocketClient::new(p)),
        Err(e) => {
            print_error(ctx.json_output, &format!("{e:#}"));
            Err(ExitCode::FAILURE)
        }
    }
}

/// Print an error in the appropriate format.
fn print_error(json_output: bool, msg: &str) {
    if json_output {
        eprintln!("{}", json!({ "error": msg }));
    } else {
        eprintln!("Error: {msg}");
    }
}

/// Evaluate a JSON-RPC response and return the `result` value on success.
fn eval_response(
    json_output: bool,
    resp: JsonRpcResponse,
) -> Result<Option<serde_json::Value>, ExitCode> {
    if let Some(err) = resp.error {
        if json_output {
            eprintln!("{}", json!({ "error": err.message, "code": err.code }));
        } else {
            eprintln!("Server error ({}): {}", err.code, err.message);
        }
        return Err(ExitCode::FAILURE);
    }
    Ok(resp.result)
}

/// Perform the full request cycle: resolve socket, send RPC, evaluate response.
async fn rpc(
    ctx: &CmdContext,
    method: &str,
    params: Option<serde_json::Value>,
) -> Result<Option<serde_json::Value>, ExitCode> {
    let client = make_client(ctx)?;
    match client.request(method, params).await {
        Ok(resp) => eval_response(ctx.json_output, resp),
        Err(e) => {
            print_error(ctx.json_output, &format!("{e:#}"));
            Err(ExitCode::FAILURE)
        }
    }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

async fn handle_ping(ctx: &CmdContext) -> ExitCode {
    match rpc(ctx, "system.ping", None).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                println!("pong");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_list_workspaces(ctx: &CmdContext) -> ExitCode {
    match rpc(ctx, "workspace.list", None).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                print_workspace_table(result.as_ref());
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Render a table of workspaces from the JSON-RPC result.
fn print_workspace_table(result: Option<&serde_json::Value>) {
    let empty_vec = vec![];
    let workspaces = result
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_vec);

    if workspaces.is_empty() {
        println!("No workspaces.");
        return;
    }

    // Compute column widths.
    let header_id = "ID";
    let header_title = "TITLE";
    let header_cwd = "CWD";

    let mut max_id = header_id.len();
    let mut max_title = header_title.len();

    for ws in workspaces {
        let id = ws.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let title = ws.get("title").and_then(|v| v.as_str()).unwrap_or("");
        max_id = max_id.max(id.len());
        max_title = max_title.max(title.len());
    }

    println!(
        "{:<id_w$}  {:<title_w$}  {}",
        header_id,
        header_title,
        header_cwd,
        id_w = max_id,
        title_w = max_title,
    );

    for ws in workspaces {
        let id = ws.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let title = ws.get("title").and_then(|v| v.as_str()).unwrap_or("-");
        let cwd = ws.get("cwd").and_then(|v| v.as_str()).unwrap_or("-");
        println!(
            "{:<id_w$}  {:<title_w$}  {}",
            id,
            title,
            cwd,
            id_w = max_id,
            title_w = max_title,
        );
    }
}

async fn handle_new_workspace(
    ctx: &CmdContext,
    name: Option<String>,
    cwd: Option<String>,
    command: Option<String>,
) -> ExitCode {
    let mut params = serde_json::Map::new();
    if let Some(n) = name {
        params.insert("name".into(), json!(n));
    }
    if let Some(c) = cwd {
        params.insert("cwd".into(), json!(c));
    }
    if let Some(cmd) = command {
        params.insert("command".into(), json!(cmd));
    }

    let params_value = if params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(params))
    };

    match rpc(ctx, "workspace.create", params_value).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                let id = result
                    .as_ref()
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                println!("Workspace created: {id}");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_select_workspace(ctx: &CmdContext, id: &str) -> ExitCode {
    let params = json!({ "id": id });
    match rpc(ctx, "workspace.select", Some(params)).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                println!("Workspace selected: {id}");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_close_workspace(ctx: &CmdContext, id: &str) -> ExitCode {
    let params = json!({ "id": id });
    match rpc(ctx, "workspace.close", Some(params)).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                println!("Workspace closed: {id}");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_send(ctx: &CmdContext, surface: &str, text: &str) -> ExitCode {
    let params = json!({ "surface_id": surface, "text": text });
    match rpc(ctx, "surface.send_text", Some(params)).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                println!("Text sent to surface {surface}.");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_new_split(
    ctx: &CmdContext,
    surface: Option<&str>,
    direction: &SplitDirection,
) -> ExitCode {
    let mut params = serde_json::Map::new();
    if let Some(s) = surface {
        params.insert("surface_id".into(), json!(s));
    }
    params.insert("direction".into(), json!(direction.to_string()));

    match rpc(ctx, "surface.split", Some(serde_json::Value::Object(params))).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                let id = result
                    .as_ref()
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                println!("Split created: {id}");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

async fn handle_list_surfaces(ctx: &CmdContext, workspace: Option<&str>) -> ExitCode {
    let params = workspace.map(|ws| json!({ "workspace_id": ws }));

    match rpc(ctx, "surface.list", params).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else {
                print_surface_table(result.as_ref());
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Render a table of surfaces from the JSON-RPC result.
fn print_surface_table(result: Option<&serde_json::Value>) {
    let empty_vec = vec![];
    let surfaces = result
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_vec);

    if surfaces.is_empty() {
        println!("No surfaces.");
        return;
    }

    let header_id = "ID";
    let header_title = "TITLE";
    let header_ws = "WORKSPACE";

    let mut max_id = header_id.len();
    let mut max_title = header_title.len();

    for s in surfaces {
        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("");
        max_id = max_id.max(id.len());
        max_title = max_title.max(title.len());
    }

    println!(
        "{:<id_w$}  {:<title_w$}  {}",
        header_id,
        header_title,
        header_ws,
        id_w = max_id,
        title_w = max_title,
    );

    for s in surfaces {
        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("-");
        let ws = s
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        println!(
            "{:<id_w$}  {:<title_w$}  {}",
            id,
            title,
            ws,
            id_w = max_id,
            title_w = max_title,
        );
    }
}

async fn handle_identify(ctx: &CmdContext) -> ExitCode {
    match rpc(ctx, "system.identify", None).await {
        Ok(result) => {
            if ctx.json_output {
                println!("{}", json!({ "result": result }));
            } else if let Some(val) = result {
                let workspace = val
                    .get("workspace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let surface = val
                    .get("surface")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let cwd = val.get("cwd").and_then(|v| v.as_str()).unwrap_or("-");
                println!("Workspace: {workspace}");
                println!("Surface:   {surface}");
                println!("CWD:       {cwd}");
            } else {
                println!("No active context.");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}
