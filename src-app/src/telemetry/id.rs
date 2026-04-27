//! Desktop shim around `paneflow_telemetry::id` (US-003).
//!
//! Resolves the platform-specific data directory via `runtime_paths` and
//! delegates persistence to the workspace crate. Public surface is
//! identical to the pre-extraction module: callers continue to use
//! `crate::telemetry::id::telemetry_id()` and
//! `crate::telemetry::id::telemetry_id_with_first_run()` unchanged.

use crate::runtime_paths;

/// Returns the stable anonymous telemetry UUID for this installation.
///
/// On first call for a given installation, generates a fresh UUID v4 and
/// writes it to `data_dir()/telemetry_id`. On subsequent calls, reads and
/// returns the persisted value. If anything goes wrong (no data dir, file
/// unwritable, file contents invalid), returns an ephemeral UUID for this
/// session and logs at DEBUG — the caller treats the subsystem as "running
/// in session-scoped mode" and carries on.
pub fn telemetry_id() -> String {
    telemetry_id_with_first_run().0
}

/// Sibling of [`telemetry_id`] that also reports whether the persistence
/// file was freshly created during this call (i.e. "first run for this
/// installation"). US-013 uses the flag to stamp `is_first_run` on the
/// `app_started` event without probing the filesystem a second time.
///
/// Ephemeral fallbacks (no `data_local_dir`, unwritable dir, corrupt
/// file) report `is_first_run = false` — we only claim first-run when
/// we actually persisted a new UUID, matching the PRD wording
/// "telemetry_id file was just created this launch".
pub fn telemetry_id_with_first_run() -> (String, bool) {
    match runtime_paths::data_dir() {
        Some(dir) => paneflow_telemetry::id::telemetry_id_at(&dir),
        None => (
            paneflow_telemetry::id::ephemeral_id("no data_local_dir resolved"),
            false,
        ),
    }
}
