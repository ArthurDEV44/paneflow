// Test-only allow for the CLAUDE.md-mandated clippy restrictions. Mirrors
// the `paneflow-app` belt: `clippy.toml`'s `allow-{unwrap,expect}-in-tests`
// keys cover the unwrap/expect family but not `clippy::panic`, which the
// `client.rs` test module uses to assert variant invariants
// (`panic!("expected Active variant")`).
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unwrap_in_result,
        clippy::panic
    )
)]

//! PaneFlow telemetry plumbing — PostHog capture client, anonymous
//! per-installation id, and canonical-tag format invariants.
//!
//! Extracted from `paneflow-app` per US-003 of the cmux-port PRD so that
//! future workspace members can emit events without taking a dependency
//! on the desktop binary.
//!
//! Submodules:
//! - [`client`] — `TelemetryClient` factory, capture API, batched flush.
//! - [`id`] — anonymous, per-installation UUID v4 with first-run flag.
//! - [`tags`] — canonical-tag format invariant helper used by
//!   consumers that map their domain enums to PostHog properties.
//!
//! Subsystem invariants (enforced at every call site, documented in
//! `tasks/compliance-analytics.md §5`):
//! - No event is ever emitted unless the caller has resolved opt-in
//!   consent **and** the kill-switch env vars are absent
//!   (`PANEFLOW_NO_TELEMETRY`, `DO_NOT_TRACK`, `NO_TELEMETRY`).
//! - No PII, no paths, no terminal content is ever transmitted — the
//!   crate provides no facilities for it.

pub mod client;
pub mod id;
pub mod tags;
