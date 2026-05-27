//! Agents-view main-area surfaces.
//!
//! Sibling of [`crate::agents_view`] (the auth/missing-agents shell):
//! this module owns the post-auth, per-thread surfaces — the
//! conversation timeline (`thread_view`), the composer, the inline
//! thinking + tool rendering, and the persisted-runtime bridge.

pub(crate) mod agent_terminal;
pub(crate) mod composer;
pub(crate) mod composer_ext;
pub(crate) mod continuous_spinner;
pub(crate) mod edit_tool_block;
pub(crate) mod external_editor;
pub(crate) mod inline_thinking;
pub(crate) mod inline_tool_call;
pub(crate) mod markdown_style;
pub(crate) mod message_render;
pub mod notifications;
pub mod panel_config;
pub mod parent_guard;
pub mod runtime;
pub mod thread_view;
pub(crate) mod title_summarizer;
