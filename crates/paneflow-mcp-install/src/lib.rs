//! `paneflow-mcp-install` — the GPU-free engine behind
//! `paneflow mcp install | uninstall | status`.
//!
//! Why a separate crate (PRD EP-002, decision D3):
//! - **Testable without GPU.** All config-merge / detection logic lives
//!   here as pure-ish functions driven by `serde_json::Value` and
//!   `toml_edit::DocumentMut`, so `cargo test -p paneflow-mcp-install` runs
//!   with zero GPUI / Vulkan dependency.
//! - **Keeps `toml_edit` out of the embedded bridge.** The size-budgeted
//!   `paneflow-mcp` binary (EP-001 US-002) depends only on
//!   `serde`/`serde_json`/`interprocess`. This crate links *only* into
//!   `paneflow-app`, never into `paneflow-mcp` — verifiable with
//!   `cargo tree -p paneflow-mcp` (it must list neither `toml_edit` nor
//!   `paneflow-mcp-install`).
//!
//! Layering:
//! - [`io`] — backup + atomic write + write-if-changed (idempotency).
//! - [`merge`] — no-clobber JSON / TOML upsert + removal + safe parse.
//! - [`detect`] — agent presence (CLI on PATH OR config file/dir exists).
//! - [`agents`] — the [`agents::AgentConfigWriter`] trait + the registry of
//!   concrete per-agent writers (EP-003 fills `default_writers`).
//! - [`cli`] — [`run_cli`]: parse the subcommand, drive every writer,
//!   format a per-agent report, return a process exit code.
//!
//! The whole crate is panic-free in non-test paths (workspace lints:
//! `panic = deny`, `unwrap_used`/`expect_used` = warn). Errors flow through
//! `anyhow::Result` and are reported per-agent so one failing agent never
//! aborts the others.

// Integration tests are compiled as a separate crate, so the `clippy.toml`
// `allow-*-in-tests` keys do not reach them (clippy #13981). Belt-and-suspenders.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod agents;
pub mod api;
pub mod cli;
pub mod detect;
pub mod io;
pub mod merge;

pub use api::{
    install_all, overall_state, status_all, uninstall_all, AgentResult, InstallKind, InstallReport,
    OverallState, StatusKind, StatusReport, UninstallKind, UninstallReport,
};
pub use cli::run_cli;
