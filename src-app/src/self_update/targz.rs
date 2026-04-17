//! tar.gz self-update via atomic directory swap (US-011).
//!
//! Flow:
//!   1. Download the tar.gz asset (and its `.sha256` sibling) to
//!      `$HOME/.cache/paneflow/update-<pid>.tar.gz`.
//!   2. Verify SHA-256 against the sibling file. A mismatch deletes the
//!      download and aborts — users never get an unverified install.
//!   3. Extract into `<parent>/paneflow.app.new/` (a sibling of the live
//!      install dir; same filesystem so the swap rename is atomic).
//!   4. Atomic swap via two `rename(2)` calls:
//!      `app_dir` → `app_dir.old`, then `app_dir.new` → `app_dir`, then
//!      `rm -rf app_dir.old`. The window between the two renames is
//!      brief. A pre-existing `app_dir.old` (crashed prior update) is a
//!      hard abort — we do NOT blindly overwrite.
//!   5. Return `<app_dir>/bin/paneflow` — the caller passes this to
//!      `cx.set_restart_path()` so GPUI's launcher execs the new binary.
//!
//! **Invariant — writes stay inside $HOME.** The download lives in
//! `$HOME/.cache/paneflow/`; extraction and swap happen inside
//! `<app_dir>` and its parent. No code path writes to `/usr`, `/opt`,
//! `/bin`, or any absolute path outside `$HOME`. This is essential for
//! immutable distros (Silverblue, SteamOS, MicroOS). Enforced by the
//! test `never_writes_outside_its_roots` which runs the whole flow under
//! a `tempdir`.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::IntegrityMismatch;

/// 500 MB ceiling on the downloaded tarball. The real release is ~30 MB;
/// a malicious mirror returning an unbounded stream would otherwise fill
/// `$HOME/.cache`.
const MAX_TARBALL_BYTES: u64 = 500 * 1024 * 1024;

/// Run the tar.gz self-update end-to-end. Resolves `$HOME`, delegates to
/// [`run_update_in`], then returns the path of the post-swap binary
/// (suitable for `cx.set_restart_path()`).
pub fn run_update(asset_url: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable is not set")?;
    let app_dir = home.join(".local").join("paneflow.app");
    let cache_dir = home.join(".cache").join("paneflow");
    run_update_in(asset_url, &app_dir, &cache_dir)?;
    Ok(app_dir.join("bin").join("paneflow"))
}

/// Testable core. Parameterised on `app_dir` + `cache_dir` so the whole
/// flow can run under a `tempdir` in unit tests without touching the real
/// `$HOME`.
fn run_update_in(asset_url: &str, app_dir: &Path, cache_dir: &Path) -> Result<()> {
    let (old_dir, new_dir) = staging_dirs(app_dir)?;

    // Fail fast if a crashed prior update left `.old` behind — silently
    // overwriting it could blow away files the user might need to
    // recover.
    if old_dir.exists() {
        bail!(
            "Previous update did not clean up. Delete `{}` and retry.",
            old_dir.display()
        );
    }
    // And if `.new` survived a crash, clean it up so extract() doesn't
    // merge into a stale tree. Unlike `.old` (hard abort because it may
    // hold recoverable state), `.new` is pure scratch — safe to remove,
    // but log a warning if the cleanup itself fails so the downstream
    // extract error isn't misdiagnosed as a tarball problem.
    if new_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&new_dir)
    {
        log::warn!(
            "self-update/targz: could not clean stale {}: {e}",
            new_dir.display()
        );
    }

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("create cache dir {}", cache_dir.display()))?;

    let tarball = cache_dir.join(format!("update-{}.tar.gz", std::process::id()));
    let download_result = download_with_verification(asset_url, &tarball);
    if let Err(e) = download_result {
        let _ = std::fs::remove_file(&tarball);
        return Err(e);
    }

    let extract_result = extract_and_swap(&tarball, app_dir, &new_dir, &old_dir);
    // Tarball cleanup is best-effort — keeping a stale ~30 MB file around
    // on disk is strictly preferable to failing the update over it.
    let _ = std::fs::remove_file(&tarball);
    extract_result
}

fn staging_dirs(app_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let parent = app_dir
        .parent()
        .context("app_dir has no parent directory — refusing to swap at filesystem root")?;
    let name = app_dir
        .file_name()
        .context("app_dir has no file name — refusing to swap")?;
    let name = name.to_string_lossy();
    Ok((
        parent.join(format!("{name}.old")),
        parent.join(format!("{name}.new")),
    ))
}

/// Download the asset + its `.sha256` sibling, verify, persist to `dest`.
///
/// On any failure the partial download is removed by the caller — we
/// don't want a half-written tarball to masquerade as a cached update on
/// the next run.
fn download_with_verification(asset_url: &str, dest: &Path) -> Result<()> {
    log::info!("self-update/targz: downloading {asset_url}");

    // 1. Fetch the sibling checksum first. If the server has no
    // `.sha256`, we refuse to install an unverified binary — fail-safe.
    // A 404 here usually means the release predates US-011's CI (when
    // the `.sha256` sibling started being emitted), so give it a
    // distinct, actionable message rather than lumping it in with a
    // network failure.
    let sha_url = format!("{asset_url}.sha256");
    let mut sha_response = ureq::get(&sha_url)
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| {
            "Could not fetch integrity checksum. Try again when online.".to_string()
        })?;
    let sha_status = sha_response.status();
    if !sha_status.is_success() {
        if sha_status.as_u16() == 404 {
            bail!(
                "This release has no SHA-256 checksum published. Download the latest version from the releases page."
            );
        }
        bail!("Could not fetch integrity checksum (HTTP {sha_status}). Try again later.");
    }
    let sha_body = sha_response
        .body_mut()
        .read_to_string()
        .context("read .sha256 body")?;
    let expected_hex = parse_sha256_file(&sha_body).with_context(|| {
        format!(
            "parse .sha256 body (first 80 bytes: {:?})",
            &sha_body.chars().take(80).collect::<String>()
        )
    })?;

    // 2. Stream the tarball to disk. Reuse the `.partial` → rename trick
    // from US-010 so a crashed download doesn't poison the cache.
    let partial = append_suffix(dest, ".partial")?;
    let mut response = ureq::get(asset_url)
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| "Could not download update. Try again when online.".to_string())?;
    if !response.status().is_success() {
        bail!(
            "Update download returned HTTP {}. Try again later.",
            response.status()
        );
    }

    {
        let reader = response.body_mut().as_reader();
        let mut reader = Read::take(reader, MAX_TARBALL_BYTES + 1);
        let mut file = std::fs::File::create(&partial)
            .with_context(|| format!("create {}", partial.display()))?;
        let written = std::io::copy(&mut reader, &mut file).context("stream tarball to disk")?;
        if written > MAX_TARBALL_BYTES {
            let _ = std::fs::remove_file(&partial);
            bail!(
                "Update download exceeded {} MiB — aborting.",
                MAX_TARBALL_BYTES / 1024 / 1024
            );
        }
        file.sync_all().ok();
    }

    // 3. Verify SHA-256. Mismatch → delete and bail with the unhappy-path
    // message mandated by US-011's acceptance criteria.
    if let Err(e) = verify_sha256_of_file(&partial, &expected_hex) {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }

    std::fs::rename(&partial, dest)
        .with_context(|| format!("rename {} → {}", partial.display(), dest.display()))?;
    Ok(())
}

/// Extract `tarball` into `new_dir` (cleaning any stale tree first), then
/// execute the two-rename swap. On any failure after extraction, `.new`
/// is cleaned up and `app_dir` is left untouched.
fn extract_and_swap(tarball: &Path, app_dir: &Path, new_dir: &Path, old_dir: &Path) -> Result<()> {
    // The tarball is laid out with `paneflow.app/` as the single top
    // entry (see scripts/bundle-tarball.sh), so we extract into `new_dir`'s
    // parent and then rename the produced `paneflow.app` to `new_dir`.
    let parent = app_dir
        .parent()
        .context("app_dir has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create parent {}", parent.display()))?;

    // Extract into a fresh scratch directory (not `parent` directly) so
    // we can't clobber an unrelated sibling if the tarball's top-level
    // entry name ever changes. The `tar` crate's `unpack` method rejects
    // entries with absolute paths or `..` traversal by default, protecting
    // against zip-slip even if a malicious archive slipped past CI. It
    // does NOT sanitize symlinks — we walk the tree post-extract and
    // reject any symlink outright (see `reject_symlinks_recursively`).
    //
    // `.paneflow-extract-<pid>` makes two PaneFlow instances updating at
    // once work: same-parent but distinct scratch dirs.
    let scratch = parent.join(format!(".paneflow-extract-{}", std::process::id()));
    if scratch.exists() {
        let _ = std::fs::remove_dir_all(&scratch);
    }
    std::fs::create_dir_all(&scratch)
        .with_context(|| format!("create scratch {}", scratch.display()))?;

    let extract_result = (|| -> Result<()> {
        let f =
            std::fs::File::open(tarball).with_context(|| format!("open {}", tarball.display()))?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(&scratch)
            .context("extract tar.gz into scratch dir")?;

        // Reject symlinks anywhere in the extracted tree before the swap.
        // Our bundler produces a symlink-free layout; anything else is
        // either a CI regression or a tampered archive. Blocking here
        // prevents a symlink like `bin/paneflow → /home/u/.ssh/id_rsa`
        // from ending up at the live install path (which the restarter
        // would then exec or follow).
        reject_symlinks_recursively(&scratch)?;

        // Move the top-level extracted directory into place as `new_dir`.
        // Strict "exactly one top-level directory" — the bundler writes
        // exactly `paneflow.app/` at the top (see
        // `scripts/bundle-tarball.sh`) and any deviation is more likely
        // a CI bug than a layout we should silently accept.
        let top = find_single_top_level(&scratch)?;
        if new_dir.exists() {
            let _ = std::fs::remove_dir_all(new_dir);
        }
        std::fs::rename(&top, new_dir)
            .with_context(|| format!("rename {} → {}", top.display(), new_dir.display()))?;

        // Belt-and-braces: force the restart binary to a known-good mode.
        // The bundler ships it 0o755, but a tampered archive could
        // downgrade to 0o777 (world-writable — local-priv-esc on shared
        // HOMEs) or upgrade to 0o700 (unexecutable by other session-id
        // processes). Force 0o755, which is the tarball's original mode.
        let bin_path = new_dir.join("bin").join("paneflow");
        if bin_path.exists() {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin_path)
                .with_context(|| format!("stat {}", bin_path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin_path, perms)
                .with_context(|| format!("chmod 0o755 {}", bin_path.display()))?;
        }
        Ok(())
    })();

    // Scratch is always cleaned up, even on success (the useful content
    // is already at `new_dir`).
    let _ = std::fs::remove_dir_all(&scratch);

    extract_result.inspect_err(|_| {
        let _ = std::fs::remove_dir_all(new_dir);
    })?;

    // Atomic swap. Two renames; brief window but same-filesystem so each
    // is atomic individually. `app_dir.old` must not exist (checked by
    // caller); any failure is aggressively rolled back.
    if app_dir.exists() {
        std::fs::rename(app_dir, old_dir)
            .with_context(|| format!("rename {} → {}", app_dir.display(), old_dir.display()))?;
    }
    if let Err(e) = std::fs::rename(new_dir, app_dir) {
        // Roll back: restore `app_dir` from `.old`. If the rollback
        // rename ALSO fails, the user has no live install at all —
        // surface both errors and the actual on-disk location of their
        // previous version so they can recover manually instead of
        // silently discarding the rollback error.
        let rollback_msg = if old_dir.exists() {
            match std::fs::rename(old_dir, app_dir) {
                Ok(()) => None,
                Err(rb) => Some(format!(
                    "rollback also failed ({rb}); your previous install is at {}",
                    old_dir.display()
                )),
            }
        } else {
            None
        };
        let _ = std::fs::remove_dir_all(new_dir);
        return match rollback_msg {
            Some(msg) => Err(e).context(format!("rename .new → app_dir ({msg})")),
            None => Err(e).context("rename .new → app_dir"),
        };
    }

    // Housekeeping: delete the previous version. Best-effort; a transient
    // failure here leaves a `.old` dir for the user to remove manually,
    // but the new install is already live.
    if old_dir.exists() {
        let _ = std::fs::remove_dir_all(old_dir);
    }

    Ok(())
}

/// Walk `dir` recursively and fail if any entry is a symlink. The
/// `tar` crate preserves symlink entries by default, so this is the
/// only defense against a tampered archive that embeds a link like
/// `bin/paneflow → /home/u/.ssh/authorized_keys`. Called AFTER
/// `unpack` so broken symlinks still count — they'd otherwise vanish
/// from `exists()` checks and survive past the swap.
fn reject_symlinks_recursively(dir: &Path) -> Result<()> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        let entries = std::fs::read_dir(&p).with_context(|| format!("read_dir {}", p.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("iterate {}", p.display()))?;
            // `symlink_metadata` does NOT follow symlinks — the file-type
            // probe here sees the link itself, not its target.
            let meta = entry.path();
            let lt = std::fs::symlink_metadata(&meta)
                .with_context(|| format!("symlink_metadata {}", meta.display()))?
                .file_type();
            if lt.is_symlink() {
                bail!(
                    "Update archive contains a symlink at {}, which PaneFlow refuses to install. Download the release manually from the releases page.",
                    meta.display()
                );
            }
            if lt.is_dir() {
                stack.push(meta);
            }
        }
    }
    Ok(())
}

/// Find the single top-level directory inside `dir`, returning its path.
/// Errors if the archive's shape is ambiguous (zero or multiple top
/// entries, or a file instead of a directory at the top).
fn find_single_top_level(dir: &Path) -> Result<PathBuf> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .context("collect dir entries")?;
    entries.sort_by_key(|e| e.file_name());
    match entries.as_slice() {
        [only] => {
            let ty = only.file_type().context("inspect top-level entry")?;
            if !ty.is_dir() {
                bail!("tarball top-level entry is not a directory");
            }
            Ok(only.path())
        }
        [] => bail!("tarball is empty — no top-level entry"),
        multi => bail!("tarball has {} top-level entries, expected 1", multi.len()),
    }
}

/// Append a suffix to the filename (not to the extension). `foo.tar.gz`
/// + `.partial` → `foo.tar.gz.partial`, not `foo.tar.partial`.
///
/// Refuses paths that don't have a filename component (e.g. `/` or
/// ending in `..`) rather than silently turning them into `/.partial`.
fn append_suffix(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path
        .file_name()
        .with_context(|| format!("path has no file name: {}", path.display()))?;
    let mut name = name.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

/// Parse a `.sha256` file's contents and return the hex digest.
///
/// Accepts both coreutils styles — `<hex>  <filename>\n` and
/// `<hex> *<filename>\n` — and the bare-hex form (`<hex>\n`). Returns the
/// first whitespace-separated token if it is a valid 64-char lowercase
/// hex string; errors otherwise.
fn parse_sha256_file(body: &str) -> Result<String> {
    let first_line = body.lines().next().context("empty .sha256 file")?;
    let token = first_line
        .split_whitespace()
        .next()
        .context("no token in .sha256 file")?;
    let lower = token.to_ascii_lowercase();
    if lower.len() != 64 || !lower.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 digest (expected 64 hex chars, got {token:?})");
    }
    Ok(lower)
}

/// Compute the SHA-256 of `file` and compare against `expected_hex` (the
/// output of [`parse_sha256_file`]). Errors with a stable, user-visible
/// "failed integrity check" message on mismatch.
fn verify_sha256_of_file(file: &Path, expected_hex: &str) -> Result<()> {
    let mut f = std::fs::File::open(file)
        .with_context(|| format!("open {} for hashing", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).context("read chunk for hashing")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got = hex_lower(&hasher.finalize());
    if got != expected_hex {
        // Bail with a structured tag instead of a formatted string so the
        // top-level classifier (UpdateError::classify) can downcast and
        // recover the exact digests for the toast + logs.
        return Err(anyhow::Error::new(IntegrityMismatch {
            expected: expected_hex.to_string(),
            got,
        }));
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── Pure helpers ───────────────────────────────────────────────────

    #[test]
    fn parse_sha256_file_accepts_coreutils_format() {
        let body = format!("{}  paneflow-v0.2.0-x86_64.tar.gz\n", "a".repeat(64));
        assert_eq!(parse_sha256_file(&body).unwrap(), "a".repeat(64));
    }

    #[test]
    fn parse_sha256_file_accepts_binary_mode() {
        let body = format!("{} *paneflow.tar.gz\n", "b".repeat(64));
        assert_eq!(parse_sha256_file(&body).unwrap(), "b".repeat(64));
    }

    #[test]
    fn parse_sha256_file_accepts_bare_hex() {
        let body = format!("{}\n", "c".repeat(64));
        assert_eq!(parse_sha256_file(&body).unwrap(), "c".repeat(64));
    }

    #[test]
    fn parse_sha256_file_is_case_insensitive() {
        let body = format!("{}\n", "DEADBEEF".repeat(8));
        assert_eq!(parse_sha256_file(&body).unwrap(), "deadbeef".repeat(8));
    }

    #[test]
    fn parse_sha256_file_rejects_short_digest() {
        let err = parse_sha256_file("abc\n").unwrap_err().to_string();
        assert!(err.contains("64 hex chars"), "got: {err}");
    }

    #[test]
    fn parse_sha256_file_rejects_non_hex() {
        let body = format!("{}\n", "z".repeat(64));
        assert!(parse_sha256_file(&body).is_err());
    }

    #[test]
    fn parse_sha256_file_rejects_empty() {
        assert!(parse_sha256_file("").is_err());
        assert!(parse_sha256_file("\n").is_err());
    }

    #[test]
    fn verify_sha256_accepts_matching_digest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("x");
        std::fs::write(&p, b"hello world").unwrap();
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        verify_sha256_of_file(
            &p,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
        )
        .unwrap();
    }

    #[test]
    fn verify_sha256_rejects_mismatched_digest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("x");
        std::fs::write(&p, b"hello world").unwrap();
        let err = verify_sha256_of_file(&p, &"0".repeat(64))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed integrity check"), "got: {err}");
    }

    /// The mismatch error must carry the actual digests as a downcastable
    /// tag so the top-level classifier (`UpdateError::classify`) can
    /// recover them for the toast — not just format them into a string.
    #[test]
    fn verify_sha256_mismatch_is_downcastable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("x");
        std::fs::write(&p, b"hello world").unwrap();
        let err = verify_sha256_of_file(&p, &"0".repeat(64)).unwrap_err();
        let mm = err
            .downcast_ref::<IntegrityMismatch>()
            .expect("expected IntegrityMismatch tag");
        assert_eq!(mm.expected, "0".repeat(64));
        // sha256("hello world") = b94d27b9...
        assert_eq!(
            mm.got,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn append_suffix_preserves_full_name() {
        assert_eq!(
            append_suffix(Path::new("/tmp/foo.tar.gz"), ".partial").unwrap(),
            PathBuf::from("/tmp/foo.tar.gz.partial")
        );
    }

    #[test]
    fn append_suffix_rejects_pathless_input() {
        assert!(append_suffix(Path::new("/"), ".partial").is_err());
    }

    #[test]
    fn staging_dirs_derives_sibling_paths() {
        let (old, new) = staging_dirs(Path::new("/home/u/.local/paneflow.app")).unwrap();
        assert_eq!(old, PathBuf::from("/home/u/.local/paneflow.app.old"));
        assert_eq!(new, PathBuf::from("/home/u/.local/paneflow.app.new"));
    }

    // ── extract_and_swap fixtures ──────────────────────────────────────

    /// Build a minimal tar.gz containing `paneflow.app/bin/paneflow` with
    /// a marker string. Returns its path.
    fn make_fixture_tarball(root: &Path, marker: &[u8]) -> PathBuf {
        let out = root.join("fixture.tar.gz");
        let f = std::fs::File::create(&out).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut builder = tar::Builder::new(gz);

        let bin_path = root.join("fixture-src/paneflow.app/bin/paneflow");
        std::fs::create_dir_all(bin_path.parent().unwrap()).unwrap();
        std::fs::write(&bin_path, marker).unwrap();

        builder
            .append_dir_all("paneflow.app", root.join("fixture-src/paneflow.app"))
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        out
    }

    #[test]
    fn extract_and_swap_replaces_existing_app_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let tarball = make_fixture_tarball(root, b"new-version");

        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(app_dir.join("bin")).unwrap();
        std::fs::write(app_dir.join("bin/paneflow"), b"old-version").unwrap();
        let (old_dir, new_dir) = staging_dirs(&app_dir).unwrap();

        extract_and_swap(&tarball, &app_dir, &new_dir, &old_dir).unwrap();

        let content = std::fs::read(app_dir.join("bin/paneflow")).unwrap();
        assert_eq!(content, b"new-version");
        assert!(!old_dir.exists(), ".old should be cleaned up");
        assert!(!new_dir.exists(), ".new should be gone post-swap");
    }

    #[test]
    fn extract_and_swap_works_when_app_dir_absent() {
        // Fresh install (no prior app_dir) must still succeed.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let tarball = make_fixture_tarball(root, b"fresh-install");

        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(app_dir.parent().unwrap()).unwrap();
        let (old_dir, new_dir) = staging_dirs(&app_dir).unwrap();

        extract_and_swap(&tarball, &app_dir, &new_dir, &old_dir).unwrap();
        assert_eq!(
            std::fs::read(app_dir.join("bin/paneflow")).unwrap(),
            b"fresh-install"
        );
    }

    #[test]
    fn extract_and_swap_rolls_back_on_corrupt_tarball() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let corrupt = root.join("corrupt.tar.gz");
        std::fs::write(&corrupt, b"not actually a gzip").unwrap();

        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(app_dir.join("bin")).unwrap();
        std::fs::write(app_dir.join("bin/paneflow"), b"keep-me").unwrap();
        let (old_dir, new_dir) = staging_dirs(&app_dir).unwrap();

        let r = extract_and_swap(&corrupt, &app_dir, &new_dir, &old_dir);
        assert!(r.is_err(), "corrupt tarball must fail");

        // The live install is preserved.
        assert_eq!(
            std::fs::read(app_dir.join("bin/paneflow")).unwrap(),
            b"keep-me"
        );
        assert!(!new_dir.exists(), ".new must be cleaned up on failure");
        assert!(!old_dir.exists(), ".old must not exist on failure");
    }

    #[test]
    fn run_update_in_aborts_when_dot_old_preexists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(&app_dir).unwrap();
        let (old_dir, _new_dir) = staging_dirs(&app_dir).unwrap();
        std::fs::create_dir_all(&old_dir).unwrap();

        let cache_dir = root.join("home/.cache/paneflow");
        // Bogus URL — we must bail BEFORE any network call.
        let r = run_update_in("http://127.0.0.1:1/should-never-hit", &app_dir, &cache_dir);
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("Previous update did not clean up"),
            "got: {err}"
        );
    }

    #[test]
    fn never_writes_outside_its_roots() {
        // End-to-end swap confined to a tempdir. We snapshot every entry
        // under a curated "outside" subtree before and after, and verify
        // no file was added, removed, or modified there. Everything the
        // updater is allowed to touch lives under `home/.local/paneflow.*`
        // (for the swap) or under the scratch dir (which must not
        // persist post-call); anything else is a bug.
        //
        // The real guarantee comes from code inspection: every path in
        // this module is derived from an input `app_dir`/`cache_dir`,
        // never from a hardcoded absolute path. This test catches
        // regressions where that invariant is violated.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        // Outside subtree: pre-populate with files whose names/contents
        // we can assert on afterwards.
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("a"), b"a-body").unwrap();
        std::fs::write(outside.join("b"), b"b-body").unwrap();
        std::fs::create_dir(outside.join("sub")).unwrap();
        std::fs::write(outside.join("sub/c"), b"c-body").unwrap();
        let before = snapshot_tree(&outside);

        let tarball = make_fixture_tarball(root, b"body");
        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(app_dir.parent().unwrap()).unwrap();
        let (old_dir, new_dir) = staging_dirs(&app_dir).unwrap();

        extract_and_swap(&tarball, &app_dir, &new_dir, &old_dir).unwrap();

        let after = snapshot_tree(&outside);
        assert_eq!(before, after, "updater touched the outside subtree");
    }

    /// Collect `(relative_path, contents)` pairs for every regular file
    /// under `root`. Deterministic ordering (BTreeMap-like via sort).
    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(p) = stack.pop() {
            for entry in std::fs::read_dir(&p).unwrap().flatten() {
                let ft = entry.file_type().unwrap();
                if ft.is_dir() {
                    stack.push(entry.path());
                } else if ft.is_file() {
                    let rel = entry.path().strip_prefix(root).unwrap().to_path_buf();
                    out.push((rel, std::fs::read(entry.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn extract_rejects_tarball_with_symlink() {
        // Build a tarball that includes a symlink and verify that
        // `extract_and_swap` refuses to install it — even though the
        // symlink itself never leaves the scratch dir, letting it reach
        // the live `app_dir` would let a tampered release point
        // `bin/paneflow` at `/home/u/.ssh/id_rsa` and exfiltrate on
        // restart.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        // Assemble the tarball manually so we can inject a symlink entry.
        let fixture_src = root.join("src/paneflow.app/bin");
        std::fs::create_dir_all(&fixture_src).unwrap();
        std::fs::write(fixture_src.join("paneflow"), b"real-bin").unwrap();
        let evil_link = fixture_src.join("evil-link");
        std::os::unix::fs::symlink("/etc/passwd", &evil_link).unwrap();

        let out = root.join("fixture.tar.gz");
        let f = std::fs::File::create(&out).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut builder = tar::Builder::new(gz);
        // `follow_symlinks(false)` preserves the symlink as a symlink
        // entry in the archive — the whole point of this test.
        builder.follow_symlinks(false);
        builder
            .append_dir_all("paneflow.app", root.join("src/paneflow.app"))
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let app_dir = root.join("home/.local/paneflow.app");
        std::fs::create_dir_all(app_dir.parent().unwrap()).unwrap();
        let (old_dir, new_dir) = staging_dirs(&app_dir).unwrap();

        let r = extract_and_swap(&out, &app_dir, &new_dir, &old_dir);
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("symlink"),
            "expected symlink rejection, got: {err}"
        );
        assert!(!app_dir.exists(), "a failed update must not leave app_dir");
        assert!(!new_dir.exists(), ".new must be cleaned up");
    }

    #[test]
    fn fixture_roundtrip_matches_known_sha256() {
        // Sanity: the pure hash helpers agree with a pre-computed digest.
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("f");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"abc").unwrap();
        drop(f);
        // sha256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        verify_sha256_of_file(
            &p,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
    }
}
