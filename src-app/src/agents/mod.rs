//! Agents-view support modules that survived the removal of the in-app
//! ACP chat and the "Connect" discovery shell. The conversation
//! timeline, composer, message/tool rendering, the persisted ACP
//! runtime, and the sign-in/welcome surface were all deleted when the
//! Agents view became terminal-only (each thread launches a CLI agent
//! in a PTY - see [`crate::agent_launcher`]).
//!
//! What remains:
//! - [`notifications`] - window/panel visibility flags.
//! - [`parent_guard`] - Windows Job Object that kills PTY children with
//!   the parent process.

pub mod notifications;
pub mod parent_guard;
