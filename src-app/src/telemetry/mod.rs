//! Desktop telemetry subsystem (opt-in, anonymous).
//!
//! Submodules:
//! - `id` (US-010) — stable anonymous UUID per installation.
//! - `client` (US-012) — ureq-based PostHog capture with background flush.
//! - `tags` (US-013) — canonical `&'static str` labels for `install_method`
//!   and `error_category` properties (flattens internal enums).
//!
//! The `PaneFlowApp`-bound `emit_*` wrappers live in `crate::app::telemetry_events`;
//! this module owns the plumbing (client, id, tag flattening) while the
//! capture-site helpers stay close to the rest of the app surface.
//!
//! Invariants shared across the subsystem:
//! - No event is ever emitted unless `config.telemetry.enabled == Some(true)`
//!   **and** no kill-switch env var is set (`PANEFLOW_NO_TELEMETRY`,
//!   `DO_NOT_TRACK`, `NO_TELEMETRY`).
//! - No PII, no paths, no terminal content is ever transmitted — enforced
//!   at every call site, documented in `tasks/compliance-analytics.md §5`.

pub mod client;
pub mod id;
pub mod tags;
