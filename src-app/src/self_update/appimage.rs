//! AppImage self-update via `appimageupdatetool` (US-010).
//!
//! Flow:
//!   1. Resolve `appimageupdatetool` — prefer `$PATH`, else download the
//!      community release to a cached temp location.
//!   2. Invoke it with `-O` (overwrite in place) against the running
//!      AppImage's source `.AppImage` file. zsync streams only the changed
//!      blocks, typically 10–30 % of file size.
//!   3. Return the unchanged source path — the file is updated in place.
//!      Caller passes it to `cx.set_restart_path()` for the GPUI launcher
//!      to exec the new version.
//!
//! The running AppImage has `UPDATE_INFORMATION="gh-releases-zsync|..."`
//! baked in by `scripts/bundle-appimage.sh`, so zsync metadata is always
//! present for releases ≥ v0.2.0. Older AppImages (pre-US-005) lack this
//! and will surface a dedicated "cannot self-update" error.
//!
//! `appimageupdatetool` is itself a Type-2 AppImage and normally needs
//! FUSE 2 at runtime. Ubuntu 24.04+ ships without `libfuse2`, and the
//! forced-install breaks `ubuntu-session`. We set `APPIMAGE_EXTRACT_AND_RUN=1`
//! on the child unconditionally — it works with OR without FUSE and side-
//! steps the whole detection problem.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::UpdateError;

/// Community fork of AppImageUpdate — the upstream `AppImage/AppImageUpdate`
/// repo was transferred. `/releases/latest/download/` redirects to whatever
/// the newest release is, so the URL is stable across versions.
const TOOL_URL_X86_64: &str = "https://github.com/AppImageCommunity/AppImageUpdate/releases/latest/download/appimageupdatetool-x86_64.AppImage";
const TOOL_URL_AARCH64: &str = "https://github.com/AppImageCommunity/AppImageUpdate/releases/latest/download/appimageupdatetool-aarch64.AppImage";

/// Run `appimageupdatetool -O <source_path>` and return the (unchanged)
/// source path on success.
///
/// On any failure — missing tool, download error, network outage, missing
/// embedded update-information, zsync integrity mismatch — returns a
/// human-readable error suitable for a toast.
pub fn run_update(source_path: &Path) -> Result<PathBuf> {
    if source_path.as_os_str().is_empty() {
        bail!(
            "This AppImage was launched without $APPIMAGE set; PaneFlow cannot locate the source file to update. Re-launch by double-clicking the .AppImage or running it directly from a shell."
        );
    }
    // `is_file` rather than `exists` so symlink-to-directory, dangling
    // symlinks, and non-regular files (sockets, fifos) all hit the same
    // clear error rather than bubbling up as opaque tool failures later.
    if !source_path.is_file() {
        bail!("AppImage source file not found: {}", source_path.display());
    }

    let tool = resolve_tool().context("resolve appimageupdatetool")?;
    invoke_tool(&tool, source_path).map(|()| source_path.to_path_buf())
}

/// Return a usable path to `appimageupdatetool`. Checks `$PATH` first; if
/// absent, downloads the community release to
/// `$XDG_CACHE_HOME/paneflow/appimageupdatetool-<arch>.AppImage` (or
/// `$HOME/.cache/paneflow/` fallback) and `chmod +x`'s it. Cached across
/// runs so a second update doesn't re-download.
///
/// Trust anchor: the cached binary is trusted on reuse. The download itself
/// relies on GitHub's HTTPS CDN + TLS cert validation (ureq default) as the
/// sole integrity check — there's no pinned SHA or GPG verification today.
/// Stronger supply-chain hardening is deferred to US-013 (structured errors)
/// and a future dedicated story.
///
/// Concurrent startup: two PaneFlow instances racing on the first update
/// will each download the tool and both rename into the same path. `rename`
/// is atomic on one filesystem so the final state is correct; the wasted
/// bandwidth is an accepted trade for not adding a lock file.
fn resolve_tool() -> Result<PathBuf> {
    if let Ok(path) = which::which("appimageupdatetool") {
        log::info!(
            "self-update/appimage: using appimageupdatetool from PATH: {}",
            path.display()
        );
        return Ok(path);
    }

    let arch = std::env::consts::ARCH;
    let cached = cache_path_for(arch)?;
    if cached.exists() {
        log::info!(
            "self-update/appimage: using cached appimageupdatetool: {}",
            cached.display()
        );
        return Ok(cached);
    }

    download_tool(arch, &cached)?;
    Ok(cached)
}

fn cache_path_for(arch: &str) -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .context("neither XDG_CACHE_HOME nor HOME is set")?;
    let dir = base.join("paneflow");
    std::fs::create_dir_all(&dir).with_context(|| format!("create cache dir {}", dir.display()))?;
    Ok(dir.join(format!("appimageupdatetool-{arch}.AppImage")))
}

fn download_tool(arch: &str, dest: &Path) -> Result<()> {
    let url = match arch {
        "x86_64" => TOOL_URL_X86_64,
        "aarch64" => TOOL_URL_AARCH64,
        other => bail!(
            "no appimageupdatetool release for arch '{other}'. Update PaneFlow manually from the releases page."
        ),
    };

    log::info!("self-update/appimage: downloading appimageupdatetool from {url}");

    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| "Could not download update tool. Try again when online.".to_string())?;

    if !response.status().is_success() {
        bail!(
            "Could not download update tool (HTTP {}). Try again later.",
            response.status()
        );
    }

    // Stream to a `.partial` sibling, then rename — leaves no half-written
    // tool around if we crash mid-download. `with_file_name` so the full
    // `.AppImage.partial` suffix is preserved (`with_extension` would drop
    // `.AppImage`).
    let partial_name = dest
        .file_name()
        .map(|n| {
            let mut s = n.to_os_string();
            s.push(".partial");
            s
        })
        .context("derive partial filename")?;
    let tmp = dest.with_file_name(partial_name);

    // 100 MB cap on the tool download — the real binary is ~10 MB. A
    // malicious mirror returning an unbounded stream would otherwise fill
    // the cache filesystem before we notice.
    const MAX_TOOL_BYTES: u64 = 100 * 1024 * 1024;
    {
        let reader = response.body_mut().as_reader();
        let mut reader = std::io::Read::take(reader, MAX_TOOL_BYTES + 1);
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        let written = std::io::copy(&mut reader, &mut file).context("stream download to disk")?;
        if written > MAX_TOOL_BYTES {
            let _ = std::fs::remove_file(&tmp);
            bail!(
                "Update tool download exceeded {} MiB — aborting. Try again later.",
                MAX_TOOL_BYTES / 1024 / 1024
            );
        }
        file.sync_all().ok();
    }

    // `0o700`: cached binary is a user-private cache, no need to expose it
    // to other users on shared hosts.
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&tmp)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(&tmp, perms)?;

    std::fs::rename(&tmp, dest)
        .with_context(|| format!("rename {} → {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn invoke_tool(tool: &Path, target: &Path) -> Result<()> {
    let output = Command::new(tool)
        // `APPIMAGE_EXTRACT_AND_RUN=1` avoids the FUSE 2 requirement on
        // Ubuntu 24.04+ where `libfuse2` is no longer shipped by default.
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        // `-O` overwrites the source file in place. Without it,
        // appimageupdatetool writes a sibling `_updated` file that our
        // restart path would miss.
        .arg("-O")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .with_context(|| format!("spawn {}", tool.display()))?;

    if output.status.success() {
        log::info!(
            "self-update/appimage: updated {} in place",
            target.display()
        );
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Some zsync2 errors go to stdout instead of stderr; classify the
    // concatenation so we don't miss them.
    let combined = format!("{stderr}\n{stdout}");
    let tag = classify_error(&combined);
    log::warn!(
        "self-update/appimage: tool exit={} tag={tag:?} stderr={stderr}",
        output.status
    );
    // Bail with a structured tag; the main-thread boundary downcasts to
    // pick the correct toast copy (US-013). No info is lost — the raw
    // stderr was just logged above.
    bail!(tag);
}

/// Map the free-form stderr/stdout of `appimageupdatetool` to an
/// [`UpdateError`] variant. The tool is noisy and its messages aren't
/// formally documented — we anchor on substrings observed in practice and
/// fall back to [`UpdateError::Other`] with a user-actionable sentence.
///
/// Kept as a pure function so it can be unit-tested without spawning.
fn classify_error(output: &str) -> UpdateError {
    let lower = output.to_ascii_lowercase();
    // FUSE 2 missing is the single most common "tool won't even start"
    // failure on modern Ubuntu — check it before generic network/exit codes
    // since it often surfaces as a shared-library load error rather than a
    // readable message.
    if lower.contains("libfuse.so.2")
        || lower.contains("libfuse2")
        || lower.contains("fuse: failed to exec fusermount")
    {
        return UpdateError::Fuse2Missing;
    }
    // "No update information" means this AppImage was built without the
    // `UPDATE_INFORMATION` key — a permanent condition for that binary, so
    // treat it as a generic `Other` with an actionable hint.
    if lower.contains("no update information")
        || lower.contains("update information not found")
        || lower.contains("no update_information")
    {
        return UpdateError::Other(
            "This AppImage cannot self-update. Download the latest version from the releases page."
                .to_string(),
        );
    }
    if lower.contains("could not resolve host")
        || lower.contains("could not connect")
        || lower.contains("failed to connect")
        || lower.contains("network is unreachable")
        || lower.contains("no such host")
    {
        return UpdateError::Network(output.to_string());
    }
    if lower.contains("checksum") || lower.contains("signature") || lower.contains("hash mismatch")
    {
        return UpdateError::IntegrityMismatch {
            expected: String::new(),
            got: String::new(),
        };
    }
    if lower.contains("no space left") || lower.contains("disk full") {
        return UpdateError::DiskFull {
            path: std::path::PathBuf::new(),
        };
    }
    UpdateError::Other(
        "Update failed. Try again later, or download the new AppImage manually from the releases page."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn empty_source_path_errors_without_spawning() {
        let r = run_update(Path::new(""));
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("$APPIMAGE"),
            "expected $APPIMAGE hint in error, got: {err}"
        );
    }

    #[test]
    fn nonexistent_source_path_errors() {
        let r = run_update(Path::new("/tmp/paneflow-does-not-exist-xyz.AppImage"));
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "expected 'not found' in error, got: {err}"
        );
    }

    #[test]
    fn classify_error_detects_missing_update_info() {
        match classify_error("zsync2 error: AppImage has no update information") {
            UpdateError::Other(msg) => assert!(msg.contains("cannot self-update"), "got: {msg}"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn classify_error_detects_network_variants() {
        for input in [
            "curl: (6) Could not resolve host: github.com",
            "Could not connect to server",
            "Failed to connect: timeout",
            "network is unreachable",
        ] {
            assert!(
                matches!(classify_error(input), UpdateError::Network(_)),
                "input {input:?} → {:?}",
                classify_error(input)
            );
        }
    }

    #[test]
    fn classify_error_detects_integrity_failure() {
        for input in [
            "checksum mismatch after download",
            "Signature verification failed",
        ] {
            assert!(
                matches!(classify_error(input), UpdateError::IntegrityMismatch { .. }),
                "input {input:?} → {:?}",
                classify_error(input)
            );
        }
    }

    #[test]
    fn classify_error_detects_disk_full() {
        assert!(matches!(
            classify_error("write failed: No space left on device"),
            UpdateError::DiskFull { .. }
        ));
    }

    #[test]
    fn classify_error_detects_fuse2_missing() {
        for input in [
            "error while loading shared libraries: libfuse.so.2",
            "fuse: failed to exec fusermount",
            "libfuse2 is required",
        ] {
            assert!(
                matches!(classify_error(input), UpdateError::Fuse2Missing),
                "input {input:?} → {:?}",
                classify_error(input)
            );
        }
    }

    #[test]
    fn classify_error_falls_back_generic() {
        match classify_error("some totally unexpected garbage") {
            UpdateError::Other(msg) => assert!(msg.contains("Update failed"), "got: {msg}"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn classify_error_is_case_insensitive() {
        assert!(matches!(
            classify_error("COULD NOT RESOLVE HOST: foo"),
            UpdateError::Network(_)
        ));
    }

    /// Simulate the "tool succeeded" path with `/bin/true` as the stub. The
    /// real `appimageupdatetool` would have mutated `target` in place; here
    /// we only verify the invoker correctly reports success.
    #[test]
    fn invoke_tool_succeeds_with_stub_true() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("fake.AppImage");
        std::fs::write(&target, b"x").unwrap();
        let r = invoke_tool(Path::new("/bin/true"), &target);
        assert!(r.is_ok(), "expected success, got: {r:?}");
    }

    /// Simulate the "tool failed with known stderr" path via a bash stub
    /// that emits a missing-update-information error. Exercises the full
    /// non-zero-exit → classify_error path end-to-end without the real tool.
    #[test]
    fn invoke_tool_propagates_missing_update_info() {
        let tmp = tempfile::TempDir::new().unwrap();
        let stub = tmp.path().join("fake-tool.sh");
        let mut f = std::fs::File::create(&stub).unwrap();
        writeln!(
            f,
            "#!/bin/sh\necho 'zsync2: AppImage has no update information' 1>&2\nexit 1"
        )
        .unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        let target = tmp.path().join("fake.AppImage");
        std::fs::write(&target, b"x").unwrap();
        let err = invoke_tool(&stub, &target).unwrap_err().to_string();
        assert!(err.contains("cannot self-update"), "got: {err}");
    }

    // Note: no dedicated test for `cache_path_for` — mutating process env
    // in a parallel test runner is a flake waiting to happen, and the fn
    // is trivially correct (just a `PathBuf::join`). The real-world
    // behavior is exercised transitively by `resolve_tool` when the user
    // lacks `appimageupdatetool` on PATH.
}
