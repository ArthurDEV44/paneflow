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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::super::error::{IntegrityMismatch, UpdateError};

/// Upper bound on any single HTTP call in the update flow (US-001). 30
/// seconds is long enough for a cold-start tethered connection, short
/// enough that a zombie background thread never forms on a half-open TCP.
const UPDATE_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

// ─── US-005: pinned-tag appimageupdatetool with SHA-256 verification ──────
//
// Rationale: `releases/latest/download/` silently redirects to whichever
// build the upstream project ships last, so a CDN regression or a
// channel-hijack can put a new, unsigned binary on the wire without any
// PaneFlow release change. Pinning to a dated tag + verifying the SHA-256
// of the downloaded bytes against constants baked into this source file
// means tampering fails closed (IntegrityMismatch, delete on disk, no
// spawn) rather than silently executing.
//
// To bump: (1) pick new dated tag from https://github.com/AppImageCommunity/AppImageUpdate/releases,
// (2) download both arch binaries, (3) sha256sum each, (4) paste hex bytes
// here, (5) advance tag in URL.
//
// The two `[u8; 32]` arrays are the raw SHA-256 digests of the assets at
// tag `2.0.0-alpha-1-20251018`. Verified 2026-04-20 against the GitHub
// release mirror.

const TOOL_URL_X86_64: &str = "https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/appimageupdatetool-x86_64.AppImage";
const TOOL_URL_AARCH64: &str = "https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/appimageupdatetool-aarch64.AppImage";

// sha256 d976cdac667b03dee8cb23fb95ef74b042c406c5cbab3ff294d2b16efeaff84f
const APPIMAGEUPDATETOOL_SHA256_X86_64: [u8; 32] = [
    0xd9, 0x76, 0xcd, 0xac, 0x66, 0x7b, 0x03, 0xde, 0xe8, 0xcb, 0x23, 0xfb, 0x95, 0xef, 0x74, 0xb0,
    0x42, 0xc4, 0x06, 0xc5, 0xcb, 0xab, 0x3f, 0xf2, 0x94, 0xd2, 0xb1, 0x6e, 0xfe, 0xaf, 0xf8, 0x4f,
];

// sha256 7aaf89dd4cf66ebd940d416c67e1c240c57a139cee38d9c0ed3bb9387bc435b0
const APPIMAGEUPDATETOOL_SHA256_AARCH64: [u8; 32] = [
    0x7a, 0xaf, 0x89, 0xdd, 0x4c, 0xf6, 0x6e, 0xbd, 0x94, 0x0d, 0x41, 0x6c, 0x67, 0xe1, 0xc2, 0x40,
    0xc5, 0x7a, 0x13, 0x9c, 0xee, 0x38, 0xd9, 0xc0, 0xed, 0x3b, 0xb9, 0x38, 0x7b, 0xc4, 0x35, 0xb0,
];

/// Resolve `(url, expected_digest)` for the running arch. Returns an error
/// for unsupported architectures — the caller surfaces it as a toast.
fn tool_asset_for(arch: &str) -> Result<(&'static str, &'static [u8; 32])> {
    match arch {
        "x86_64" => Ok((TOOL_URL_X86_64, &APPIMAGEUPDATETOOL_SHA256_X86_64)),
        "aarch64" => Ok((TOOL_URL_AARCH64, &APPIMAGEUPDATETOOL_SHA256_AARCH64)),
        other => bail!(
            "no appimageupdatetool release for arch '{other}'. Update PaneFlow manually from the releases page."
        ),
    }
}

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
/// absent, downloads the pinned-tag community release (US-005) to
/// `$XDG_CACHE_HOME/paneflow/appimageupdatetool-<arch>.AppImage` (or
/// `$HOME/.cache/paneflow/` fallback), verifies its SHA-256 against the
/// hardcoded digest, and `chmod +x`'s it.
///
/// Trust anchor (US-005): the downloaded bytes are compared byte-for-byte
/// against `APPIMAGEUPDATETOOL_SHA256_<ARCH>` before the file is renamed
/// into the cache. A cached binary is re-verified on each resolve so that
/// (a) tampering between runs fails closed, and (b) a constants bump
/// invalidates stale caches from the previous pinned tag — no manual
/// `rm ~/.cache/paneflow/appimageupdatetool-*` step needed.
///
/// A tool found on `$PATH` is trusted without hashing — a distro-installed
/// `appimageupdatetool` is under the user's package manager, not the
/// update flow's threat model.
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
    let (url, expected) = tool_asset_for(arch)?;
    let cached = cache_path_for(arch)?;
    if cached.exists() {
        match verify_sha256_of_file(&cached, expected) {
            Ok(()) => {
                log::info!(
                    "self-update/appimage: using cached appimageupdatetool: {}",
                    cached.display()
                );
                return Ok(cached);
            }
            Err(e) => {
                // Stale cache from a prior pinned tag OR on-disk tampering.
                // Either way, discard and re-download — the constants in
                // source are the trust root. Don't surface the mismatch as
                // an error yet; the fresh download will either succeed
                // (constants match the upstream binary) or fail with a
                // real IntegrityMismatch the user needs to see.
                log::warn!(
                    "self-update/appimage: cached tool digest mismatch, re-downloading: {e:#}"
                );
                let _ = std::fs::remove_file(&cached);
            }
        }
    }

    download_tool(url, expected, &cached)?;
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

fn download_tool(url: &str, expected: &[u8; 32], dest: &Path) -> Result<()> {
    log::info!("self-update/appimage: downloading appimageupdatetool from {url}");

    let mut response = ureq::get(url)
        .config()
        .timeout_global(Some(UPDATE_HTTP_TIMEOUT))
        .build()
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| "Could not download update tool. Try again when online.".to_string())?;

    // US-005 AC8: a 404 on the pinned asset means the upstream tag or asset
    // was removed. Surface it as the dedicated ReleaseAssetMissing variant
    // so the toast copy names the exact asset (not a generic "try again
    // later" that suggests the user is at fault).
    if response.status().as_u16() == 404 {
        return Err(anyhow::Error::new(UpdateError::ReleaseAssetMissing {
            url: url.to_string(),
        }));
    }
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
    // Stream the body in an inner block so `file` drops before any
    // `remove_file` runs. On Windows, `DeleteFile` fails while a handle is
    // open (ERROR_SHARING_VIOLATION) — keeping this scope tight is a
    // cross-platform requirement. US-001 AC7.
    let stream_result = {
        let reader = response.body_mut().as_reader();
        let mut reader = Read::take(reader, MAX_TOOL_BYTES + 1);
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        let written = std::io::copy(&mut reader, &mut file).context("stream download to disk");
        file.sync_all().ok();
        written
    };
    let written = match stream_result {
        Ok(n) => n,
        Err(e) => {
            // AC6: partial file never survives an I/O failure — the next
            // run must re-download from scratch rather than trust a
            // truncated binary.
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if written > MAX_TOOL_BYTES {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "Update tool download exceeded {} MiB — aborting. Try again later.",
            MAX_TOOL_BYTES / 1024 / 1024
        );
    }

    // US-005 AC2/AC3: verify the downloaded bytes against the hardcoded
    // digest BEFORE rename. Mismatch → delete the partial and bail with a
    // typed IntegrityMismatch so the classifier surfaces the "corrupt or
    // tampered" toast. `expected` is the raw 32-byte digest, so we compare
    // against the hasher's output directly (no hex round-trip).
    if let Err(e) = verify_sha256_of_file(&tmp, expected) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // `0o700`: cached binary is a user-private cache, no need to expose it
    // to other users on shared hosts. Also satisfies the "chmod +x" leg of
    // US-005 AC4 — 0o700 includes the owner execute bit.
    //
    // AppImage is a Linux-only format, so this function never executes on
    // Windows; the cfg guard exists purely so the module still compiles on
    // `x86_64-pc-windows-msvc` for the shared dep closure (the module
    // stays unconditionally declared in mod.rs).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&tmp, perms)?;
    }

    std::fs::rename(&tmp, dest)
        .with_context(|| format!("rename {} → {}", tmp.display(), dest.display()))?;
    Ok(())
}

/// Compute the SHA-256 of `file` and compare against `expected` byte-for-byte.
/// Mismatch returns a typed [`IntegrityMismatch`] (anyhow-wrapped) so the
/// top-level classifier routes to the "corrupt or tampered" toast and
/// preserves both digests for logs — per US-005 AC2/AC3.
fn verify_sha256_of_file(file: &Path, expected: &[u8; 32]) -> Result<()> {
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
    let digest = hasher.finalize();
    if digest.as_slice() != expected.as_slice() {
        return Err(anyhow::Error::new(IntegrityMismatch {
            expected: hex_lower(expected),
            got: hex_lower(digest.as_slice()),
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

// US-005 — tests module gated to Unix because the fixtures use
// `std::os::unix::fs::PermissionsExt` and the behaviors under test
// (AppImage bundle permissions) are Linux-only by definition.
#[cfg(all(test, unix))]
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

    // ─── US-005: pinned-tag + digest verification ────────────────────────

    /// The dated release tag is the trust anchor — kept as a single source
    /// of truth here so a bump procedure updates one const and the two URL
    /// assertions pick it up automatically.
    const PINNED_TAG: &str = "2.0.0-alpha-1-20251018";

    #[test]
    fn tool_asset_for_x86_64_points_at_pinned_tag() {
        let (url, digest) = tool_asset_for("x86_64").unwrap();
        assert!(
            url.contains(PINNED_TAG),
            "x86_64 URL should embed the pinned tag, got: {url}"
        );
        assert!(
            !url.contains("/latest/"),
            "x86_64 URL must not use the floating 'latest' redirect: {url}"
        );
        assert_eq!(digest, &APPIMAGEUPDATETOOL_SHA256_X86_64);
    }

    #[test]
    fn tool_asset_for_aarch64_points_at_pinned_tag() {
        let (url, digest) = tool_asset_for("aarch64").unwrap();
        assert!(
            url.contains(PINNED_TAG),
            "aarch64 URL should embed the pinned tag, got: {url}"
        );
        assert!(
            !url.contains("/latest/"),
            "aarch64 URL must not use the floating 'latest' redirect: {url}"
        );
        assert_eq!(digest, &APPIMAGEUPDATETOOL_SHA256_AARCH64);
    }

    #[test]
    fn tool_asset_for_unknown_arch_errors() {
        let err = tool_asset_for("riscv64").unwrap_err().to_string();
        assert!(err.contains("riscv64"), "got: {err}");
        assert!(err.contains("manually"), "got: {err}");
    }

    /// AC7: when the bytes on disk do not match the hardcoded digest,
    /// `verify_sha256_of_file` returns an `IntegrityMismatch` carrying both
    /// digests (so the classifier's typed downcast surfaces them in logs).
    #[test]
    fn verify_sha256_rejects_mismatched_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("tampered.AppImage");
        std::fs::write(&path, b"not the real tool bytes").unwrap();

        let err = verify_sha256_of_file(&path, &APPIMAGEUPDATETOOL_SHA256_X86_64).unwrap_err();
        let mm = err
            .downcast_ref::<IntegrityMismatch>()
            .expect("mismatch error should be an IntegrityMismatch");
        assert_eq!(
            mm.expected,
            hex_lower(&APPIMAGEUPDATETOOL_SHA256_X86_64),
            "expected digest should be the hex of the pinned constant"
        );
        assert_ne!(mm.got, mm.expected, "got digest must differ from expected");
        assert_eq!(
            mm.got.len(),
            64,
            "got digest must be a full 64-char sha256 hex, got: {:?}",
            mm.got
        );
    }

    /// AC7 (continued): the mismatch error classifies as `IntegrityMismatch`
    /// at the main-thread boundary — this is what drives the "corrupt or
    /// tampered" toast.
    #[test]
    fn verify_sha256_mismatch_classifies_as_integrity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("tampered.AppImage");
        std::fs::write(&path, b"x").unwrap();
        let err = verify_sha256_of_file(&path, &APPIMAGEUPDATETOOL_SHA256_X86_64).unwrap_err();
        assert!(matches!(
            UpdateError::classify(&err),
            UpdateError::IntegrityMismatch { .. }
        ));
    }

    /// AC7 (continued): simulate the download-time flow — file is created,
    /// digest fails, the caller (download_tool) deletes the file. After the
    /// `remove_file` step the tampered file must NOT be present on disk.
    #[test]
    fn digest_mismatch_deletes_file_on_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("appimageupdatetool.AppImage.partial");
        std::fs::write(&path, b"tampered").unwrap();
        assert!(path.exists());

        if verify_sha256_of_file(&path, &APPIMAGEUPDATETOOL_SHA256_X86_64).is_err() {
            std::fs::remove_file(&path).unwrap();
        }
        assert!(
            !path.exists(),
            "mismatched file must be removed from disk after verification failure"
        );
    }

    /// Round-trip test for the bump-procedure comment: the x86_64 constant's
    /// hex encoding is the one documented next to the declaration. If a
    /// future bump updates the hex comment but forgets the byte array (or
    /// vice versa), this assertion catches the divergence.
    #[test]
    fn pinned_digest_hex_matches_byte_array() {
        assert_eq!(
            hex_lower(&APPIMAGEUPDATETOOL_SHA256_X86_64),
            "d976cdac667b03dee8cb23fb95ef74b042c406c5cbab3ff294d2b16efeaff84f"
        );
        assert_eq!(
            hex_lower(&APPIMAGEUPDATETOOL_SHA256_AARCH64),
            "7aaf89dd4cf66ebd940d416c67e1c240c57a139cee38d9c0ed3bb9387bc435b0"
        );
    }
}
