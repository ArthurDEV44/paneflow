//! App-layer modules extracted from `main.rs`.
//!
//! See `tasks/prd-src-app-refactor.md` for the ongoing decomposition plan.

pub mod about_dialog;
pub mod actions;
pub mod agents_sidebar;
pub mod agents_view_actions;
pub mod bootstrap;
pub mod constants;
pub mod custom_buttons_modal;
pub mod drag;
pub mod event_handlers;
pub mod ipc_handler;
pub mod notifications;
pub mod profile_menu;
pub mod project_ops;
pub mod self_update_flow;
pub mod session;
pub mod sessions_menu;
pub mod settings;
pub mod sidebar;
pub mod sidebar_actions_menu;
pub mod telemetry_events;
pub mod theme_picker;
pub mod workspace_ops;
