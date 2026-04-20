//! App-layer modules extracted from `main.rs`.
//!
//! See `tasks/prd-src-app-refactor.md` for the ongoing decomposition plan.

pub mod actions;
pub mod bootstrap;
pub mod event_handlers;
pub mod ipc_handler;
pub mod self_update_flow;
pub mod session;
pub mod settings;
pub mod sidebar;
pub mod workspace_ops;
