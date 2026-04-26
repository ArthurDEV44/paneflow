//! US-008 — atomic, idempotent extraction of the embedded AI-hook
//! binaries into the user's per-OS cache directory.
//!
//! Layout produced by `ensure_binaries_extracted`:
//!
//! ```text
//! <dirs::cache_dir()>/paneflow/bin/<version>/
//!     ├── claude[.exe]            ← copy of paneflow-shim
//!     ├── codex[.exe]             ← copy of paneflow-shim
//!     └── paneflow-ai-hook[.exe]  ← copy of paneflow-ai-hook
//! ```
//!
//! Why two shim copies instead of a hardlink: `std::fs::hard_link` is
//! cross-filesystem-fragile on macOS (APFS ↔ tmpfs) and has surprising
//! semantics on Windows (NTFS only, blocked on ReFS/network shares).
//! Writing the bytes twice is a few-hundred-kilobyte cost on first
//! extraction and zero cost thereafter (SHA256 match → skip).
//!
//! Idempotency: every target path's contents are SHA256-matched against
//! the embedded bytes before rewriting. Re-extraction is a no-op when the
//! cache is already up to date — verified by the `re_extraction_is_noop`
//! unit test below.
//!
//! Unhappy path: every IO error surfaces as `anyhow::Err` so the caller
//! (PTY spawn, US-009) can log-and-continue without the PATH-prepend
//! instead of aborting the user's terminal session. Constraint C4 of the
//! PRD mandates silent fail outside the PTY — the caller, not this
//! module, owns the log-and-skip policy.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};

use crate::assets::Bins;

/// Target triple the outer build staged binaries for. Injected by
/// `build.rs` via `cargo:rustc-env=PANEFLOW_TARGET_TRIPLE=<triple>` (see
/// `src-app/build.rs`). Used to look up the correct sub-folder inside the
/// `Bins` `RustEmbed` archive.
const TARGET_TRIPLE: &str = env!("PANEFLOW_TARGET_TRIPLE");

/// Crate version — pins the cache-dir sub-folder so a `0.2.6 → 0.2.7`
/// upgrade re-extracts rather than reusing stale bytes. Matches
/// `CARGO_PKG_VERSION` from the outer build.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Three filenames the extractor materializes in the cache dir.
/// `(basename, source)` where `source` is the name inside the embed
/// folder (same basename for the ai-hook callback; `paneflow-shim` for
/// both `claude` and `codex`).
const EXTRACT_PLAN: &[(&str, &str)] = &[
    ("claude", "paneflow-shim"),
    ("codex", "paneflow-shim"),
    ("paneflow-ai-hook", "paneflow-ai-hook"),
];

/// Platform-appropriate executable extension. Empty on Unix, `.exe` on
/// Windows. Used for both the embed-side filename and the extracted
/// filename.
#[inline]
fn exe_suffix() -> &'static str {
    if cfg!(windows) { ".exe" } else { "" }
}

/// Pull the raw bytes of `name` out of the `Bins` rust-embed archive.
/// `name` is the `<binary>[.exe]` basename staged by build.rs under
/// `target/embed/bin/<triple>/`.
fn embedded_bytes(name: &str) -> Result<std::borrow::Cow<'static, [u8]>> {
    let key = format!("bin/{TARGET_TRIPLE}/{name}");
    Bins::get(&key)
        .map(|f| f.data)
        .ok_or_else(|| anyhow!("US-008: embed entry {key} missing — did build.rs stage it?"))
}

/// Internal layout-free pair used by `extract_into`. Decouples the
/// embed-source-of-truth (`Bins`) from the extraction algorithm so unit
/// tests can inject synthetic bytes without depending on build.rs having
/// populated the staging dir.
pub(crate) struct Entry<'a> {
    pub filename: String,
    pub bytes: &'a [u8],
}

/// Materialize the AI-hook binaries into
/// `<dirs::cache_dir()>/paneflow/bin/<version>/` and return the
/// containing directory.
///
/// - Creates parent directories on demand.
/// - Atomic per-file: writes to a temp file in the same dir, then
///   renames into place.
/// - Unix: sets mode `0o755` on every extracted file.
/// - Idempotent: if every file already exists with a matching SHA256,
///   returns the target dir without writing.
///
/// Errors surface via `anyhow::Result` for log-and-skip handling in
/// `src-app/src/terminal/pty_session.rs::inject_ai_hook_env` (US-009).
pub fn ensure_binaries_extracted() -> Result<PathBuf> {
    let cache_root = dirs::cache_dir()
        .ok_or_else(|| anyhow!("US-008: dirs::cache_dir() returned None; cannot extract"))?;
    let target_dir = cache_root.join("paneflow").join("bin").join(VERSION);

    let suffix = exe_suffix();
    let mut buffers: Vec<(String, std::borrow::Cow<'static, [u8]>)> =
        Vec::with_capacity(EXTRACT_PLAN.len());
    for (out_name, src_name) in EXTRACT_PLAN {
        let src_full = format!("{src_name}{suffix}");
        let out_full = format!("{out_name}{suffix}");
        buffers.push((out_full, embedded_bytes(&src_full)?));
    }
    let entries: Vec<Entry<'_>> = buffers
        .iter()
        .map(|(n, b)| Entry {
            filename: n.clone(),
            bytes: b.as_ref(),
        })
        .collect();

    extract_into(&entries, &target_dir)?;
    Ok(target_dir)
}

/// Core extraction loop. Factored out of `ensure_binaries_extracted` so
/// unit tests can drive it with synthetic `Entry` slices and a
/// `TempDir`-backed output path without depending on `Bins` or
/// `dirs::cache_dir()`.
pub(crate) fn extract_into(entries: &[Entry<'_>], target_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("US-008: create cache dir {} failed", target_dir.display()))?;

    for entry in entries {
        // Defense in depth: `EXTRACT_PLAN` contains only constant ASCII
        // basenames, but the crate-private `Entry` constructor is
        // reachable from anywhere in the crate. Reject any non-basename
        // filename — both `/` and `\` regardless of host — so a future
        // caller cannot produce a write outside `target_dir`, and a
        // `..\\` injected on a Linux build-host still fires on a
        // Windows target.
        if entry.filename.contains('/')
            || entry.filename.contains('\\')
            || entry.filename == ".."
            || entry.filename == "."
            || entry.filename.is_empty()
        {
            return Err(anyhow!(
                "US-008: refusing to extract entry with non-basename filename {:?}",
                entry.filename
            ));
        }
        let final_path = target_dir.join(&entry.filename);

        // Idempotency fast-path: existing file with matching digest is
        // kept as-is — avoids rewriting the file on every launch and
        // therefore avoids bumping its mtime, which some extraction-path
        // auditors (AV / code-signing verifiers on Windows) flag.
        if file_matches_digest(&final_path, entry.bytes)? {
            continue;
        }

        write_atomic(&final_path, entry.bytes)
            .with_context(|| format!("US-008: atomic write of {} failed", final_path.display()))?;

        // Re-verify the just-written file. Catches the narrow race window
        // where an AV scanner (Windows Defender real-time protection) or a
        // FUSE filesystem rewrites the file between persist() and the
        // shim's next exec — without this check, the corrupted bytes would
        // sit on disk forever because the idempotency fast-path above
        // would re-detect them as "matching whatever's on disk now".
        // Cost: one re-read + sha256 per *new* extraction (~600 KB), zero
        // on the fast-path.
        if !file_matches_digest(&final_path, entry.bytes)? {
            return Err(anyhow!(
                "US-008: post-write digest mismatch for {} — \
                 filesystem or AV interference suspected",
                final_path.display()
            ));
        }
    }

    Ok(())
}

/// Compute the SHA256 of `bytes`.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// Return `true` iff `path` exists and its contents hash to the same
/// SHA256 as `expected`. A missing file returns `false` (not an error);
/// any other IO error propagates so the caller does not silently
/// overwrite what might be a tampered binary.
fn file_matches_digest(path: &Path, expected: &[u8]) -> Result<bool> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("US-008: open {} failed", path.display()));
        }
    };

    let mut hasher = Sha256::new();
    // 8 KiB buffer — the hook binaries are small (~200-600 KB), so
    // streaming makes no measurable difference, but it keeps the
    // comparator working even if a future binary ever grows past the
    // 1 MB cap.
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)
            .with_context(|| format!("US-008: read {} failed", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual: [u8; 32] = hasher.finalize().into();
    Ok(actual == sha256(expected))
}

/// Write `bytes` to `final_path` atomically: create a temp file in the
/// same directory, flush + chmod + rename. The rename is atomic on
/// POSIX and on Windows NTFS (via `MoveFileEx` + `REPLACE_EXISTING`
/// semantics inside `tempfile::NamedTempFile::persist`).
fn write_atomic(final_path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = final_path
        .parent()
        .ok_or_else(|| anyhow!("US-008: {} has no parent dir", final_path.display()))?;

    // NamedTempFile placed in the same parent dir → rename is a same-
    // filesystem operation (atomic) rather than a cross-device copy.
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("US-008: tempfile in {} failed", parent.display()))?;
    tmp.write_all(bytes)
        .context("US-008: write_all to tempfile failed")?;
    tmp.as_file_mut()
        .sync_all()
        .context("US-008: sync_all on tempfile failed")?;

    // Unix: make the binary executable before renaming into place so the
    // rename publishes an already-runnable file (no "file created without
    // +x" window for a PATH scanner to race).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(tmp.path(), perms)
            .with_context(|| format!("US-008: chmod 0o755 on {} failed", tmp.path().display()))?;
    }

    tmp.persist(final_path).map_err(|e| {
        anyhow!(
            "US-008: atomic rename {} -> {} failed: {}",
            e.file.path().display(),
            final_path.display(),
            e.error
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Deterministic synthetic bytes — not real executables. The extraction
    // algorithm is content-agnostic, so non-executable payloads exercise
    // every code path (atomic write, chmod, SHA256 match) without
    // invoking a nested cargo build to produce the real binaries.
    const FAKE_SHIM: &[u8] = b"paneflow-shim synthetic bytes v0";
    const FAKE_HOOK: &[u8] = b"paneflow-ai-hook synthetic bytes v0";

    fn synthetic_entries() -> Vec<Entry<'static>> {
        let suffix = exe_suffix();
        vec![
            Entry {
                filename: format!("claude{suffix}"),
                bytes: FAKE_SHIM,
            },
            Entry {
                filename: format!("codex{suffix}"),
                bytes: FAKE_SHIM,
            },
            Entry {
                filename: format!("paneflow-ai-hook{suffix}"),
                bytes: FAKE_HOOK,
            },
        ]
    }

    #[test]
    fn extracts_all_three_filenames() {
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();
        extract_into(&entries, dir.path()).unwrap();

        let suffix = exe_suffix();
        for expected in [
            format!("claude{suffix}"),
            format!("codex{suffix}"),
            format!("paneflow-ai-hook{suffix}"),
        ] {
            let p = dir.path().join(&expected);
            assert!(
                p.is_file(),
                "US-008 AC: expected {} to exist after extraction",
                p.display()
            );
        }
    }

    #[test]
    fn extracted_bytes_match_input_sha256() {
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();
        extract_into(&entries, dir.path()).unwrap();

        for entry in &entries {
            let p = dir.path().join(&entry.filename);
            let on_disk = std::fs::read(&p).unwrap();
            assert_eq!(
                sha256(&on_disk),
                sha256(entry.bytes),
                "US-008 AC: extracted {} must SHA256-match the input bytes",
                p.display()
            );
        }
    }

    #[test]
    fn shim_copies_are_identical() {
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();
        extract_into(&entries, dir.path()).unwrap();

        let suffix = exe_suffix();
        let claude = std::fs::read(dir.path().join(format!("claude{suffix}"))).unwrap();
        let codex = std::fs::read(dir.path().join(format!("codex{suffix}"))).unwrap();
        assert_eq!(
            claude, codex,
            "US-008 AC: claude and codex are both copies of paneflow-shim"
        );
    }

    #[test]
    fn re_extraction_is_noop() {
        // First extraction — record each file's mtime. Second extraction
        // must leave mtimes untouched (idempotency via SHA256 fast-path).
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();
        extract_into(&entries, dir.path()).unwrap();

        let mut first_mtimes = Vec::new();
        for entry in &entries {
            let p = dir.path().join(&entry.filename);
            first_mtimes.push((
                p.clone(),
                std::fs::metadata(&p).unwrap().modified().unwrap(),
            ));
        }

        // Sleep a hair so a second write would produce a distinguishable
        // mtime on filesystems with ms resolution (ext4 default, NTFS,
        // APFS). 50 ms is enough to cross millisecond granularity while
        // keeping `cargo test` wall-clock tight.
        std::thread::sleep(std::time::Duration::from_millis(50));

        extract_into(&entries, dir.path()).unwrap();

        for (p, first_mtime) in first_mtimes {
            let second_mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
            assert_eq!(
                first_mtime,
                second_mtime,
                "US-008 AC: re-extraction of unchanged bytes must be a no-op (mtime unchanged) for {}",
                p.display()
            );
        }
    }

    #[test]
    fn stale_bytes_are_overwritten() {
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();

        // Pre-populate one file with STALE bytes of the wrong size.
        let stale_path = dir.path().join(&entries[0].filename);
        std::fs::write(&stale_path, b"stale").unwrap();

        extract_into(&entries, dir.path()).unwrap();

        let after = std::fs::read(&stale_path).unwrap();
        assert_eq!(
            after, entries[0].bytes,
            "US-008: stale bytes must be overwritten by the current embed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_mode_is_0o755() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let entries = synthetic_entries();
        extract_into(&entries, dir.path()).unwrap();

        for entry in &entries {
            let p = dir.path().join(&entry.filename);
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o755,
                "US-008 AC: {} must be mode 0o755 on Unix, got 0o{:o}",
                p.display(),
                mode & 0o777
            );
        }
    }

    #[test]
    fn bins_embed_contains_both_staged_binaries() {
        // Proves the build.rs → rust-embed path populates the archive
        // with both expected keys. `embedded_bytes` wraps `Bins::get`;
        // a `None` here means either build.rs did not run or the
        // nested cargo build silently skipped one of the binaries.
        let suffix = exe_suffix();
        for src in ["paneflow-shim", "paneflow-ai-hook"] {
            let name = format!("{src}{suffix}");
            let bytes = embedded_bytes(&name).unwrap_or_else(|e| {
                panic!("US-008: Bins must contain `bin/{TARGET_TRIPLE}/{name}`: {e}")
            });
            assert!(
                !bytes.is_empty(),
                "US-008: embedded {name} must be non-empty"
            );
        }
    }

    #[test]
    fn ensure_binaries_extracted_produces_three_files() {
        // End-to-end smoke: calls the public entry point against the
        // real cache dir and asserts all three expected filenames land.
        // The cache dir is per-user and persistent, so this test is
        // deliberately idempotent — safe to run repeatedly. Skip when
        // `dirs::cache_dir()` is unresolvable (ephemeral CI containers
        // with no `$HOME` set) so the test becomes a no-op rather than
        // a false failure in those environments.
        if dirs::cache_dir().is_none() {
            eprintln!("skip: dirs::cache_dir() unresolvable in this environment");
            return;
        }
        let dir = ensure_binaries_extracted().unwrap();
        let suffix = exe_suffix();
        for name in [
            format!("claude{suffix}"),
            format!("codex{suffix}"),
            format!("paneflow-ai-hook{suffix}"),
        ] {
            let p = dir.join(&name);
            assert!(
                p.is_file(),
                "US-008: ensure_binaries_extracted must produce {}",
                p.display()
            );
        }
    }

    #[test]
    fn rejects_non_basename_filenames() {
        let dir = tempfile::TempDir::new().unwrap();
        let bad_cases: &[&str] = &["..", ".", "", "nested/evil", "..\\evil"];
        for bad in bad_cases {
            let entries = [Entry {
                filename: (*bad).to_string(),
                bytes: b"x",
            }];
            let err = extract_into(&entries, dir.path())
                .err()
                .unwrap_or_else(|| panic!("US-008: {bad:?} must be rejected"));
            assert!(
                err.to_string().contains("non-basename"),
                "US-008: rejection for {bad:?} must mention 'non-basename'; got {err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn returns_err_when_parent_is_readonly() {
        // AC "Unhappy path: extraction failure (permission denied, disk
        // full) → ensure_binaries_extracted returns Err". We simulate
        // permission-denied by extracting into a sub-path of a dir that
        // we chmod to 0o555 (no write).
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let ro_parent = dir.path().join("ro");
        std::fs::create_dir(&ro_parent).unwrap();
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o555)).unwrap();

        // Target sub-dir inside the read-only parent — create_dir_all
        // should fail.
        let target = ro_parent.join("bin");
        let entries = synthetic_entries();
        let res = extract_into(&entries, &target);

        // Restore perms so TempDir drop can clean up.
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            res.is_err(),
            "US-008 AC: extraction into a read-only parent must return Err"
        );
    }
}
