//! Structured classification of update failures (US-013) plus the `is_disk_full`
//! predicate. The main thread calls [`UpdateError::classify`] at the boundary to
//! bucket `anyhow::Error`s into renderable variants; the toast renderer picks
//! its copy per variant (`Network`, `IntegrityMismatch`, `Fuse2Missing`,
//! `DiskFull`, `Other`).
//!
//! Extracted from `self_update/mod.rs` per US-031. The `libc::ENOSPC` arm of
//! `is_disk_full` is gated `#[cfg(unix)]` per US-034 — on Windows the
//! `std::io::ErrorKind::StorageFull` branch alone is sufficient.

use std::path::PathBuf;

/// Structured, user-facing classification of an update failure (US-013).
///
/// Lower layers return `anyhow::Error`; the main thread calls
/// [`UpdateError::classify`] at the boundary to bucket the failure into one
/// of the variants the title bar knows how to render. Keep variants
/// *exhaustive from the renderer's perspective*: adding a new failure mode
/// without a matching toast string is a UX regression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// DNS/TCP/TLS/socket-level failure. The wrapped string is preserved for
    /// logs; the toast uses a fixed human message.
    Network(#[allow(dead_code)] String),
    /// SHA-256 of the downloaded asset did not match its `.sha256` sibling.
    IntegrityMismatch { expected: String, got: String },
    /// AppImage runtime needs libfuse2 and it's not installed. The toast
    /// suggests `--appimage-extract-and-run` as an immediate workaround.
    Fuse2Missing,
    /// ENOSPC on a write inside the update flow. `path` is best-effort — we
    /// don't always know which write failed, in which case this is an empty
    /// `PathBuf` and the toast renders without the "at {path}" clause.
    DiskFull { path: PathBuf },
    /// Classifier couldn't bucket the error. The wrapped message is shown
    /// verbatim so the user sees *something* actionable instead of a
    /// generic "update failed".
    Other(String),
}

impl UpdateError {
    /// Render the PRD-mandated toast copy for this variant. The strings are
    /// intentionally frozen — changing them in one place but not the other
    /// would break US-013 acceptance tests.
    pub fn user_message(&self) -> String {
        match self {
            UpdateError::Network(_) => {
                "Update failed: no connection. Retry when online.".to_string()
            }
            UpdateError::IntegrityMismatch { .. } => {
                "Update failed: downloaded file is corrupt or tampered. Retry or download manually."
                    .to_string()
            }
            UpdateError::Fuse2Missing => {
                "Update requires FUSE 2. Run: `./paneflow-*.AppImage --appimage-extract-and-run` — or install libfuse2."
                    .to_string()
            }
            UpdateError::DiskFull { path } => {
                if path.as_os_str().is_empty() {
                    "Update failed: disk full. Free space and retry.".to_string()
                } else {
                    format!(
                        "Update failed: disk full at `{}`. Free space and retry.",
                        path.display()
                    )
                }
            }
            UpdateError::Other(msg) => msg.clone(),
        }
    }

    /// Bucket an `anyhow::Error` into a variant.
    ///
    /// Preference order (most specific first):
    ///   1. Downcast to `UpdateError` — lower layers can bail with a
    ///      pre-classified error for free.
    ///   2. Downcast to [`IntegrityMismatch`] — carries `expected`/`got`.
    ///   3. Walk the chain looking for `std::io::Error` with ENOSPC.
    ///   4. Substring-match on the formatted error chain for FUSE /
    ///      network / integrity / disk-full keywords.
    ///   5. Fall back to `Other` with the raw formatted message.
    pub fn classify(err: &anyhow::Error) -> Self {
        // Walk the full cause chain. `Error::downcast_ref` only inspects the
        // outermost error, which would miss a pre-classified tag wrapped by
        // `.context(...)` — probe every cause so callers are free to add
        // context without losing classification.
        for cause in err.chain() {
            if let Some(tag) = cause.downcast_ref::<UpdateError>() {
                return tag.clone();
            }
            if let Some(mm) = cause.downcast_ref::<IntegrityMismatch>() {
                return UpdateError::IntegrityMismatch {
                    expected: mm.expected.clone(),
                    got: mm.got.clone(),
                };
            }
            if let Some(io) = cause.downcast_ref::<std::io::Error>()
                && is_disk_full(io)
            {
                return UpdateError::DiskFull {
                    path: PathBuf::new(),
                };
            }
        }
        let full = format!("{err:#}");
        let lower = full.to_ascii_lowercase();
        if lower.contains("libfuse.so.2")
            || lower.contains("libfuse2")
            || lower.contains("appimage-extract-and-run")
            || lower.contains("failed to exec fusermount")
        {
            return UpdateError::Fuse2Missing;
        }
        if lower.contains("no space left") || lower.contains("disk full") {
            return UpdateError::DiskFull {
                path: PathBuf::new(),
            };
        }
        if lower.contains("could not fetch integrity checksum")
            || lower.contains("could not download update")
            || lower.contains("could not download update tool")
            || lower.contains("try again when online")
            || lower.contains("could not resolve host")
            || lower.contains("could not connect")
            || lower.contains("failed to connect")
            || lower.contains("network is unreachable")
            || lower.contains("no such host")
        {
            return UpdateError::Network(full);
        }
        if lower.contains("failed integrity check")
            || lower.contains("integrity check")
            || lower.contains("checksum")
            || lower.contains("hash mismatch")
        {
            return UpdateError::IntegrityMismatch {
                expected: String::new(),
                got: String::new(),
            };
        }
        UpdateError::Other(full)
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for UpdateError {}

/// Tag error for SHA-256 verification failures. Attached via `anyhow::bail!`
/// so the outer classifier can [`downcast_ref`](anyhow::Error::downcast_ref)
/// to recover the exact `expected`/`got` digests — substring-parsing a
/// human message would lose them.
#[derive(Debug, Clone)]
pub struct IntegrityMismatch {
    pub expected: String,
    pub got: String,
}

impl std::fmt::Display for IntegrityMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Downloaded file failed integrity check. Retry or download manually. (expected {}, got {})",
            self.expected, self.got
        )
    }
}

impl std::error::Error for IntegrityMismatch {}

/// True if `err` represents ENOSPC. Covers both the typed `StorageFull`
/// variant (platform-independent, stable since Rust 1.83) and — on Unix —
/// the raw-errno fallback for older syscalls that surface `28` via
/// `raw_os_error()` without setting the typed kind.
///
/// US-034: the `libc::ENOSPC` arm is gated `#[cfg(unix)]` so Windows builds
/// can rely on `ErrorKind::StorageFull` alone.
pub fn is_disk_full(err: &std::io::Error) -> bool {
    if matches!(err.kind(), std::io::ErrorKind::StorageFull) {
        return true;
    }
    #[cfg(unix)]
    {
        if err.raw_os_error() == Some(libc::ENOSPC) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_err(kind: std::io::ErrorKind) -> std::io::Error {
        std::io::Error::new(kind, "synthetic")
    }

    #[test]
    fn classify_direct_update_error_roundtrips() {
        let tagged = UpdateError::Fuse2Missing;
        let err = anyhow::Error::new(tagged);
        assert_eq!(UpdateError::classify(&err), UpdateError::Fuse2Missing);
    }

    #[test]
    fn classify_recovers_tag_through_context_wrapping() {
        // If a caller wraps `bail!(UpdateError::…)` with `.context(…)`, the
        // root error becomes the context string. The classifier must still
        // recover the tag by walking the chain instead of stopping at the
        // outermost layer.
        let err = anyhow::Error::new(UpdateError::Network("ureq hit EOF".into()))
            .context("fetch release asset")
            .context("self-update/targz");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::Network(_)
        ));
    }

    #[test]
    fn classify_extracts_integrity_mismatch_digests() {
        let mm = IntegrityMismatch {
            expected: "a".repeat(64),
            got: "b".repeat(64),
        };
        let err = anyhow::Error::new(mm)
            .context("download asset")
            .context("self-update/targz");
        match UpdateError::classify(&err) {
            UpdateError::IntegrityMismatch { expected, got } => {
                assert_eq!(expected, "a".repeat(64));
                assert_eq!(got, "b".repeat(64));
            }
            other => panic!("expected IntegrityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn classify_disk_full_via_storage_full_kind() {
        let err = anyhow::Error::new(io_err(std::io::ErrorKind::StorageFull))
            .context("write chunk to disk");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::DiskFull { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn classify_disk_full_via_raw_errno() {
        // On some platforms the typed kind isn't set but errno is 28.
        let err = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::ENOSPC))
            .context("create cache dir");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::DiskFull { .. }
        ));
    }

    #[test]
    fn classify_disk_full_via_substring_fallback() {
        // When the io::Error is already stringified (e.g., came out of a
        // subprocess stderr), we fall back to text matching.
        let err = anyhow::anyhow!("extract tar.gz into scratch dir: No space left on device");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::DiskFull { .. }
        ));
    }

    #[test]
    fn classify_network_via_context_message() {
        let err = anyhow::anyhow!("Could not download update. Try again when online.");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::Network(_)
        ));
    }

    #[test]
    fn classify_network_via_resolve_host() {
        let err = anyhow::anyhow!("curl: (6) Could not resolve host: github.com");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::Network(_)
        ));
    }

    #[test]
    fn classify_fuse2_missing_variants() {
        for msg in [
            "error while loading shared libraries: libfuse.so.2",
            "failed to exec fusermount: No such file or directory",
            "try running with --appimage-extract-and-run",
            "libfuse2 is not installed",
        ] {
            let err = anyhow::Error::msg(msg.to_string());
            assert!(
                matches!(UpdateError::classify(&err), UpdateError::Fuse2Missing),
                "msg {msg:?} → {:?}",
                UpdateError::classify(&err)
            );
        }
    }

    #[test]
    fn classify_integrity_via_keyword_fallback() {
        // No downcast available (e.g., message came from an external tool).
        let err = anyhow::anyhow!("zsync2: checksum verification failed");
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::IntegrityMismatch { .. }
        ));
    }

    #[test]
    fn classify_other_for_unclassifiable_error() {
        let err = anyhow::anyhow!("some totally unexpected garbage");
        match UpdateError::classify(&err) {
            UpdateError::Other(msg) => assert!(msg.contains("unexpected garbage")),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn user_message_matches_prd_copy_network() {
        assert_eq!(
            UpdateError::Network("any".into()).user_message(),
            "Update failed: no connection. Retry when online."
        );
    }

    #[test]
    fn user_message_matches_prd_copy_integrity() {
        assert_eq!(
            UpdateError::IntegrityMismatch {
                expected: "a".into(),
                got: "b".into(),
            }
            .user_message(),
            "Update failed: downloaded file is corrupt or tampered. Retry or download manually."
        );
    }

    #[test]
    fn user_message_matches_prd_copy_fuse2() {
        let got = UpdateError::Fuse2Missing.user_message();
        assert!(got.contains("FUSE 2"));
        assert!(got.contains("--appimage-extract-and-run"));
        assert!(got.contains("libfuse2"));
    }

    #[test]
    fn user_message_disk_full_includes_path_when_set() {
        let err = UpdateError::DiskFull {
            path: PathBuf::from("/home/u/.cache/paneflow"),
        };
        let msg = err.user_message();
        assert!(msg.contains("disk full"));
        assert!(msg.contains("/home/u/.cache/paneflow"));
        assert!(msg.contains("Free space and retry"));
    }

    #[test]
    fn user_message_disk_full_omits_path_when_empty() {
        let err = UpdateError::DiskFull {
            path: PathBuf::new(),
        };
        let msg = err.user_message();
        assert!(msg.contains("disk full"));
        assert!(!msg.contains("at `"));
    }

    #[test]
    fn user_message_other_passes_through_raw() {
        let err = UpdateError::Other("raw detail".into());
        assert_eq!(err.user_message(), "raw detail");
    }
}
