//! v1 desktop telemetry events — `app_started`, `app_exited`,
//! `update_installed` (US-013). Thin wrappers over the client in
//! `telemetry::client`; all property construction lives here so the
//! event schema is auditable in one place (cross-referenced with the
//! compliance record at `tasks/compliance-analytics.md §5`).
//!
//! None of these helpers check consent — that's already the
//! `TelemetryClient::from_config` factory's job in bootstrap. If the
//! client is `Null`, every `capture` call is a no-op and no HTTP
//! request is made, satisfying PRD AC #6.

use std::time::Duration;

use serde_json::json;

use crate::PaneFlowApp;
use crate::telemetry::tags::{error_category_tag, install_method_tag};
use crate::update::{self, UpdateError};

/// Upper bound on how long we wait for the shutdown flush to complete
/// before the batch is dropped and process exit continues. Matches the
/// US-012 client's 2-second contract.
const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

impl PaneFlowApp {
    /// Fire the once-per-launch `app_started` event. Called at the end
    /// of `PaneFlowApp::new` (bootstrap), after the telemetry client
    /// handle has been constructed from the loaded config.
    ///
    /// Property set matches US-013 AC #1 verbatim: `os`, `arch`,
    /// `app_version`, `install_method`, `is_first_run`.
    pub(crate) fn emit_app_started(&self, is_first_run: bool) {
        self.telemetry.capture(
            "app_started",
            json!({
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "app_version": env!("CARGO_PKG_VERSION"),
                "install_method": install_method_tag(&self.install_method),
                "is_first_run": is_first_run,
            }),
        );
    }

    /// Fire `app_exited` and block up to [`SHUTDOWN_FLUSH_TIMEOUT`] for
    /// the batch to reach PostHog. Called from `on_window_should_close`
    /// before `cx.quit()` — we accept a ≤2 s shutdown delay so the last
    /// session's data lands; the client itself detaches its worker on
    /// timeout, so process exit never waits longer than that.
    ///
    /// `session_duration_seconds` is computed from `self.launch_instant`
    /// captured in bootstrap — monotonic, wall-clock-change-proof.
    pub(crate) fn emit_app_exited_and_flush(&self) {
        let duration_seconds = self.launch_instant.elapsed().as_secs();
        self.telemetry.capture(
            "app_exited",
            json!({
                "session_duration_seconds": duration_seconds,
            }),
        );
        self.telemetry.flush_blocking(SHUTDOWN_FLUSH_TIMEOUT);
    }

    /// Fire `update_installed { success: true, ... }` and block up to
    /// [`SHUTDOWN_FLUSH_TIMEOUT`] so the event lands before the
    /// `cx.restart()` call that replaces the running process.
    ///
    /// Called from every success path in `self_update_flow.rs`
    /// immediately before the restart. `to_version` is read from
    /// `self.update_status` (populated during the update check); an
    /// unknown `Some(UpdateStatus::Available { version })` state is
    /// unreachable here by construction, but if it happens we still
    /// emit with `to_version: "unknown"` rather than silently dropping.
    pub(crate) fn emit_update_success_and_flush(&self) {
        let to_version = match self.update_status.as_ref() {
            Some(update::checker::UpdateStatus::Available { version, .. }) => version.clone(),
            _ => "unknown".to_string(),
        };
        self.telemetry.capture(
            "update_installed",
            json!({
                "from_version": env!("CARGO_PKG_VERSION"),
                "to_version": to_version,
                "install_method": install_method_tag(&self.install_method),
                "success": true,
            }),
        );
        self.telemetry.flush_blocking(SHUTDOWN_FLUSH_TIMEOUT);
    }

    /// Fire `update_installed { success: false, error_category: ... }`
    /// without blocking — the process is NOT about to die, so we let
    /// the background flush loop pick this up on its next tick.
    ///
    /// Called from the single choke-point `PaneFlowApp::record_update_failure`
    /// after the error has been classified. Only a canonical
    /// `error_category` label is sent — never the error message
    /// (PRD AC #4: "no error message details — just category").
    pub(crate) fn emit_update_failure(&self, err: &UpdateError) {
        let to_version = match self.update_status.as_ref() {
            Some(update::checker::UpdateStatus::Available { version, .. }) => version.clone(),
            _ => "unknown".to_string(),
        };
        self.telemetry.capture(
            "update_installed",
            json!({
                "from_version": env!("CARGO_PKG_VERSION"),
                "to_version": to_version,
                "install_method": install_method_tag(&self.install_method),
                "success": false,
                "error_category": error_category_tag(err),
            }),
        );
    }
}
