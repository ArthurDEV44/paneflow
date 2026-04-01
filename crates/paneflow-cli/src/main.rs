mod client;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use client::{resolve_socket, SocketClient};

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

#[derive(Subcommand)]
enum Command {
    /// Send a ping to the running PaneFlow daemon.
    Ping,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        // No subcommand — clap already printed help via `--help`, and
        // `--version` is handled automatically. If the user just runs
        // `paneflow` with no args, print a short usage hint.
        eprintln!("No command specified. Run `paneflow --help` for usage.");
        return ExitCode::FAILURE;
    };

    match command {
        Command::Ping => handle_ping(&cli.socket, cli.json).await,
    }
}

async fn handle_ping(socket_override: &Option<PathBuf>, json_output: bool) -> ExitCode {
    let socket_path = match resolve_socket(socket_override.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            if json_output {
                let err = serde_json::json!({
                    "error": format!("{e:#}")
                });
                eprintln!("{err}");
            } else {
                eprintln!("Error: {e:#}");
            }
            return ExitCode::FAILURE;
        }
    };

    let client = SocketClient::new(socket_path);

    match client.request("system.ping", None).await {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if json_output {
                    let out = serde_json::json!({
                        "error": err.message,
                        "code": err.code,
                    });
                    eprintln!("{out}");
                } else {
                    eprintln!("Server error ({}): {}", err.code, err.message);
                }
                ExitCode::FAILURE
            } else {
                if json_output {
                    let out = serde_json::json!({
                        "result": resp.result,
                    });
                    println!("{out}");
                } else {
                    println!("pong");
                }
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            if json_output {
                let err = serde_json::json!({
                    "error": format!("{e:#}")
                });
                eprintln!("{err}");
            } else {
                eprintln!("Error: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}
