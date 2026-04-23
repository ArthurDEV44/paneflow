//! Anonymous per-installation telemetry identifier (US-010).
//!
//! A 128-bit UUID v4 is generated on first launch and persisted at
//! `data_dir()/telemetry_id`. Every subsequent launch re-reads the file and
//! returns the same UUID — so events from the same installation correlate
//! without PostHog ever learning anything about the user.
//!
//! The file is the only state written to disk by the telemetry subsystem.
//! It can be deleted at any time: a fresh UUID is generated on the next
//! launch, which effectively disassociates the user from prior events.
//!
//! Degraded modes — all three return an ephemeral (session-scoped) UUID
//! and log at DEBUG, never surface to the user:
//! 1. `data_dir()` returns `None` — broken environment (no XDG, no
//!    Application Support, no `%LOCALAPPDATA%`).
//! 2. The directory exists but the file is unwritable or unreadable
//!    (read-only FS, permission denied, ENOSPC).
//! 3. The file exists but contains a value that does not parse as a UUID
//!    (user edited it manually, partial write, disk corruption) — we do
//!    not overwrite it, so an intentional edit by a curious user is
//!    preserved; telemetry simply runs session-scoped until they fix or
//!    delete the file.

use std::path::Path;

use uuid::Uuid;

use crate::runtime_paths;

const TELEMETRY_ID_FILE: &str = "telemetry_id";

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
        Some(dir) => telemetry_id_in(&dir),
        None => (ephemeral("no data_local_dir resolved"), false),
    }
}

/// Testable variant: reads/writes the telemetry_id file under the given
/// base directory. Returns `(id, is_first_run)` where `is_first_run` is
/// `true` iff this call created the file from scratch.
fn telemetry_id_in(base: &Path) -> (String, bool) {
    let file = base.join(TELEMETRY_ID_FILE);

    // Happy path: file exists and contains a valid UUID.
    if let Ok(contents) = std::fs::read_to_string(&file) {
        let trimmed = contents.trim();
        if Uuid::parse_str(trimmed).is_ok() {
            return (trimmed.to_string(), false);
        }
        // File exists but is corrupt / not a UUID. Respect the user's file
        // (don't overwrite) and fall back to ephemeral for this session.
        return (
            ephemeral(&format!(
                "telemetry_id file {} did not parse as UUID",
                file.display()
            )),
            false,
        );
    }

    // First launch: generate, persist, return. If persistence fails (disk
    // full, read-only FS), surface the freshly minted UUID anyway — it
    // only stays alive for this process. A failed persist is NOT a first
    // run for telemetry purposes: next launch will also fail to read, so
    // every launch would otherwise claim first-run — we suppress the flag
    // to avoid that double-count.
    let fresh = Uuid::new_v4().to_string();
    match std::fs::write(&file, &fresh) {
        Ok(()) => (fresh, true),
        Err(e) => {
            log::debug!(
                "paneflow: could not persist telemetry_id at {} ({e}); using ephemeral id for this session",
                file.display()
            );
            (fresh, false)
        }
    }
}

/// Ephemeral UUID for degraded modes — never persisted.
fn ephemeral(reason: &str) -> String {
    log::debug!("paneflow: telemetry running session-scoped ({reason})");
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn parses_as_uuid(s: &str) -> bool {
        Uuid::parse_str(s).is_ok()
    }

    #[test]
    fn first_call_creates_file_with_v4_uuid_and_flags_first_run() {
        let dir = TempDir::new().unwrap();
        let (id, first_run) = telemetry_id_in(dir.path());
        assert!(parses_as_uuid(&id), "expected a valid UUID, got {id:?}");
        assert!(first_run, "first call must report is_first_run = true");

        let file = dir.path().join(TELEMETRY_ID_FILE);
        assert!(file.exists(), "first call must create the persistence file");
        let on_disk = fs::read_to_string(&file).unwrap();
        assert_eq!(on_disk.trim(), id);

        // UUID v4 variant 4 bit pattern (RFC 4122 §4.4): the third group
        // starts with '4', and the fourth group starts with 8/9/a/b.
        let parsed = Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn second_call_returns_same_id_and_not_first_run() {
        let dir = TempDir::new().unwrap();
        let (first_id, first_flag) = telemetry_id_in(dir.path());
        let (second_id, second_flag) = telemetry_id_in(dir.path());
        assert_eq!(
            first_id, second_id,
            "subsequent calls must return the same persisted UUID"
        );
        assert!(first_flag, "first call flags is_first_run = true");
        assert!(
            !second_flag,
            "second call must flag is_first_run = false (already persisted)"
        );
    }

    #[test]
    fn corrupt_file_contents_yield_ephemeral_and_preserve_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join(TELEMETRY_ID_FILE);
        fs::write(&file, "not-a-uuid-garbage").unwrap();

        let (id, first_run) = telemetry_id_in(dir.path());
        assert!(parses_as_uuid(&id), "ephemeral id must still be a UUID");
        assert_ne!(id, "not-a-uuid-garbage");
        assert!(!first_run, "ephemeral fallback must not claim is_first_run");

        // The bad file is left untouched (don't overwrite something the
        // user may have edited intentionally).
        assert_eq!(fs::read_to_string(&file).unwrap(), "not-a-uuid-garbage");
    }

    #[test]
    fn missing_directory_yields_ephemeral_and_does_not_panic() {
        // Use a path under a temp dir that we then drop so the parent
        // exists only for the duration the TempDir lives. To simulate a
        // non-existent parent, use a sub-path that was never created.
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nope");
        let (id, first_run) = telemetry_id_in(&missing);
        // Parent is absent → read fails → write fails → ephemeral UUID.
        assert!(parses_as_uuid(&id));
        assert!(
            !first_run,
            "write-failed ephemeral path must not claim is_first_run"
        );
        assert!(
            !missing.exists(),
            "function must not create the missing parent"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_file_yields_ephemeral() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let file = dir.path().join(TELEMETRY_ID_FILE);
        // Pre-seed with a valid UUID, then make the file unreadable.
        let seed = Uuid::new_v4().to_string();
        fs::write(&file, &seed).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();

        let (id, first_run) = telemetry_id_in(dir.path());
        // With mode 000, read fails → write also fails → ephemeral.
        assert!(parses_as_uuid(&id));
        assert!(
            !first_run,
            "ephemeral read-only fallback must not claim is_first_run"
        );

        // Restore perms so TempDir drop can clean up.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        // Seed must remain untouched.
        assert_eq!(fs::read_to_string(&file).unwrap(), seed);
    }
}
