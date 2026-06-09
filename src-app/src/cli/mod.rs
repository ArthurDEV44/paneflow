//! `paneflow <verb>` scriptable CLI (EP-001, prd-cli-agent-orchestration).
//!
//! Talks to a RUNNING Paneflow instance over the existing IPC JSON-RPC socket
//! (`paneflow-ipc-client`) and exits before any GPUI init. `main.rs` dispatches
//! here only when `argv[1]` names a known verb ([`is_cli_verb`]) — mirroring the
//! `paneflow mcp …` intercept — so every other invocation (no args, unknown
//! args, `--help`/`--version`/`--update-and-exit`) is left untouched and the GUI
//! launch path is preserved. clap therefore never has to own the "no subcommand
//! => launch the GUI" default, and never eats the manually-parsed top-level
//! flags handled above it.

use clap::{Parser, Subcommand, ValueEnum};
use paneflow_ipc_client::IpcClient;
use serde_json::Value;

mod control_cmds;
mod read_cmds;
mod selector;
mod send_cmd;
mod up_cmd;
mod wait_cmd;
mod workspace_spec;

/// Process exit codes. Kept distinct so scripts can branch on the failure
/// kind. clap owns `2` for its own usage/parse errors (and `0` for
/// `--help`/`--version`), so the runtime codes start at `1` and avoid `2`.
pub const EXIT_OK: i32 = 0;
pub const EXIT_RUNTIME: i32 = 1;
pub const EXIT_TARGET: i32 = 3;
/// `wait` reached its deadline without the pattern appearing. Distinct from
/// EXIT_TARGET (no/ambiguous match) and EXIT_RUNTIME (instance down / pane
/// closed) so scripts can tell a timeout apart from a hard failure.
pub const EXIT_TIMEOUT: i32 = 4;

/// The verbs this CLI owns. `main.rs` gates the whole CLI dispatch (and the
/// manual `--help`/`--version` scans) on membership here so the GUI launch
/// path stays byte-for-byte unchanged for any other `argv[1]`.
const VERBS: &[&str] = &[
    "ls", "read", "search", "new", "select", "split", "send", "up", "wait",
];

/// True when `argv[1]` names one of our subcommands.
pub fn is_cli_verb(arg: Option<&str>) -> bool {
    matches!(arg, Some(v) if VERBS.contains(&v))
}

#[derive(Parser, Debug)]
#[command(
    name = "paneflow",
    version,
    about = "Drive a running Paneflow instance from the shell",
    // The GUI launch (no subcommand) is handled in main.rs, never here, so a
    // bare `paneflow` never reaches clap. `Option<Commands>` keeps clap from
    // forcing `subcommand_required` / `arg_required_else_help` regardless.
    subcommand_required = false,
    arg_required_else_help = false
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List the panes (surfaces) of the active workspace.
    Ls {
        /// Human-readable table instead of the default JSON.
        #[arg(long)]
        human: bool,
    },
    /// Print a pane's scrollback (raw text by default).
    Read {
        /// Target: surface id, name, `cmdline:<substr>`, or `cwd:<path>`.
        target: String,
        /// Number of trailing lines (server clamps to 1..4000).
        #[arg(long)]
        lines: Option<u64>,
        /// Offset from the end of the buffer.
        #[arg(long)]
        offset: Option<u64>,
        /// Emit the `{text, lines, total_lines, eof}` envelope as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Search a pane's scrollback for a substring/pattern.
    Search {
        /// Target: surface id, name, `cmdline:<substr>`, or `cwd:<path>`.
        target: String,
        /// Pattern to search for.
        pattern: String,
        /// Cap the number of matches (server clamps to 1..1000).
        #[arg(long)]
        max: Option<u64>,
        /// Human-readable lines instead of the default JSON.
        #[arg(long)]
        human: bool,
    },
    /// Create a new workspace.
    New {
        /// Workspace title.
        #[arg(long)]
        name: Option<String>,
        /// Working directory for the first pane (must exist).
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Select a workspace by index.
    Select {
        /// Zero-based workspace index.
        index: u64,
    },
    /// Split the active pane horizontally or vertically.
    Split {
        /// `h`/`horizontal` (panes stacked) or `v`/`vertical` (side by side).
        direction: SplitDir,
    },
    /// Inject text into a pane WITHOUT submitting it (human-in-loop).
    ///
    /// Requires `PANEFLOW_IPC_SCRIPTING=1` on the running instance; the text is
    /// written verbatim with no trailing newline so the user/agent reviews and
    /// presses Enter themselves.
    Send {
        /// Target: surface id, name, `cmdline:<substr>`, or `cwd:<path>`.
        target: String,
        /// Text to inject (no trailing carriage return is ever added).
        text: String,
    },
    /// Spawn a declarative agent workspace from a TOML file ("compose for agents").
    Up {
        /// Path to a `paneflow.workspace.toml` spec.
        file: String,
        /// Validate + print the resolved plan without touching the instance.
        #[arg(long)]
        dry_run: bool,
    },
    /// Block until a regex appears in a pane's recent output (orchestration).
    Wait {
        /// Target: surface id, name, `cmdline:<substr>`, or `cwd:<path>`.
        /// Note: `cmdline:` matches the full argv on Linux but only the
        /// executable basename on macOS/Windows; prefer `cwd:` or a name for a
        /// portable selector.
        #[arg(long = "match", value_name = "SELECTOR")]
        selector: String,
        /// Regex to wait for in the pane's recent scrollback.
        #[arg(long)]
        pattern: String,
        /// Max seconds to wait before giving up (default 300).
        #[arg(long)]
        timeout: Option<u64>,
        /// Succeed as soon as ANY matching pane matches (selector may hit several).
        #[arg(long, conflicts_with = "all")]
        any: bool,
        /// Require ALL matching panes to match the pattern.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum SplitDir {
    #[value(name = "horizontal", alias = "h")]
    Horizontal,
    #[value(name = "vertical", alias = "v")]
    Vertical,
}

impl SplitDir {
    /// The `direction` string the `surface.split` IPC method expects.
    fn as_ipc(self) -> &'static str {
        match self {
            SplitDir::Horizontal => "horizontal",
            SplitDir::Vertical => "vertical",
        }
    }
}

/// A CLI failure carrying the process exit code to surface for it.
#[derive(Debug)]
pub struct CliError {
    pub code: i32,
    pub message: String,
}

impl CliError {
    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_RUNTIME,
            message: message.into(),
        }
    }

    pub fn target(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_TARGET,
            message: message.into(),
        }
    }
}

/// Entry point invoked by `main.rs` when `argv[1]` is a known verb. Parses the
/// args with clap, opens a client to the running instance, dispatches, and
/// returns the process exit code.
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // clap prints `--help`/`--version` (exit 0) and usage errors (exit 2)
        // itself; we just relay its code rather than letting `parse()` abort
        // the process from inside a GUI binary.
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };

    // `command` is always `Some` here: main.rs only calls `run` when argv[1]
    // is a known verb. The `None` arm is unreachable in practice.
    let Some(command) = cli.command else {
        return EXIT_OK;
    };

    let client = match connect() {
        Ok(client) => client,
        Err(message) => {
            eprintln!("{message}");
            return EXIT_RUNTIME;
        }
    };

    match dispatch(command, &client) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("paneflow: {}", e.message);
            e.code
        }
    }
}

/// Resolve the socket path and build a client. The path is resolved eagerly
/// (honoring `PANEFLOW_SOCKET_PATH`), but a missing instance only surfaces as
/// an "unreachable … is Paneflow running?" error on the first `call`, so a
/// resolvable-but-dead socket is not a `connect` failure.
fn connect() -> Result<IpcClient, String> {
    let socket = paneflow_ipc_client::resolve_socket_path().ok_or_else(|| {
        "paneflow: cannot locate the IPC socket; is Paneflow running? \
         (set PANEFLOW_SOCKET_PATH if you launched the CLI outside a Paneflow pane)"
            .to_string()
    })?;
    Ok(IpcClient::new(socket))
}

/// Route a parsed subcommand to its handler. Handlers land per story:
/// `read`/`search` + the target selector (US-003/US-004), `new`/`select`/`split`
/// (US-005), `send` (US-006). The scaffold (US-002) wires the surface and the
/// transport; each arm returns an explicit "not yet implemented" runtime error
/// until its story fills it in.
fn dispatch(command: Commands, client: &IpcClient) -> Result<i32, CliError> {
    match command {
        Commands::Ls { human } => read_cmds::ls(client, human),
        Commands::Read {
            target,
            lines,
            offset,
            json,
        } => read_cmds::read(client, &target, lines, offset, json),
        Commands::Search {
            target,
            pattern,
            max,
            human,
        } => read_cmds::search(client, &target, &pattern, max, human),
        Commands::New { name, cwd } => {
            control_cmds::new_workspace(client, name.as_deref(), cwd.as_deref())
        }
        Commands::Select { index } => control_cmds::select(client, index),
        Commands::Split { direction } => control_cmds::split(client, direction.as_ipc()),
        Commands::Send { target, text } => send_cmd::send(client, &target, &text),
        Commands::Up { file, dry_run } => up_cmd::up(client, &file, dry_run),
        Commands::Wait {
            selector,
            pattern,
            timeout,
            any,
            all,
        } => {
            let mode = if all {
                wait_cmd::MatchMode::All
            } else if any {
                wait_cmd::MatchMode::Any
            } else {
                wait_cmd::MatchMode::Single
            };
            wait_cmd::wait(client, &selector, &pattern, timeout, mode)
        }
    }
}

/// Render a JSON-RPC `result` value as pretty JSON to stdout. Shared by the
/// read and control command modules so every machine-readable output uses one
/// renderer.
pub(super) fn print_json(value: &Value) -> Result<(), CliError> {
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|e| CliError::runtime(format!("failed to render JSON: {e}")))?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_cli_verb_matches_known_verbs() {
        assert!(is_cli_verb(Some("ls")));
        assert!(is_cli_verb(Some("send")));
        assert!(!is_cli_verb(Some("mcp")));
        assert!(!is_cli_verb(Some("--version")));
        assert!(!is_cli_verb(None));
    }

    #[test]
    fn cli_parses_a_verb_with_flags() {
        let cli = Cli::try_parse_from(["paneflow", "ls", "--human"]).expect("parse");
        assert!(matches!(cli.command, Some(Commands::Ls { human: true })));
    }

    #[test]
    fn split_accepts_short_aliases() {
        let cli = Cli::try_parse_from(["paneflow", "split", "h"]).expect("parse");
        assert!(matches!(
            cli.command,
            Some(Commands::Split {
                direction: SplitDir::Horizontal
            })
        ));
    }

    #[test]
    fn read_requires_a_target() {
        // Missing the required positional `target` is a clap usage error (2).
        let err = Cli::try_parse_from(["paneflow", "read"]).expect_err("usage");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn no_subcommand_parses_to_none() {
        // Defensive: a bare invocation never reaches `run` (main.rs gates on a
        // known verb), but clap must not force-error on it.
        let cli = Cli::try_parse_from(["paneflow"]).expect("parse");
        assert!(cli.command.is_none());
    }
}
