//! Agents view (US-005 host for the auth card + missing-agents empty
//! state).
//!
//! This is the minimal Agents-view shell that hosts the UI
//! deliverables of US-005 -- the auth-required card and the
//! missing-agents empty state -- so the PRD acceptance criteria can
//! be exercised in the running app without waiting on US-008 (full
//! AppMode + render branch + title-bar icon) or US-013 (full
//! ThreadView).
//!
//! When US-008/US-013 land, this shell becomes a sibling host that
//! the proper Agents view renders for "no thread selected yet" /
//! "thread needs auth" states; the cards themselves do not move.
//!
//! Toggled with `Ctrl+Shift+A` (`Cmd+Shift+A` on macOS) -- this is
//! the same binding US-008 will repurpose for the full Agents mode
//! toggle.

mod cards;
mod skills;
mod state;
mod view;

pub(crate) use skills::{SkillsTab, render_skills_page};
pub(crate) use view::{AgentsView, CloseRequested};
