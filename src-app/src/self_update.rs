//! In-app self-update flow.
//!
//! When the user clicks the "update available" pill in the title bar, we:
//!   1. Download the Linux `.run` installer from GitHub Releases to a temp path.
//!   2. `chmod +x` the file and spawn it — the installer writes the new binary
//!      to `~/.local/bin/paneflow` non-interactively.
//!   3. Call GPUI's `cx.restart()` with the installed binary path. GPUI's
//!      Linux platform spawns a detached `bash` launcher that waits for the
//!      current PID to exit (via `kill -0` polling) and then execs the new
//!      binary. This is the "launcher pattern" — safe for Wayland + GPU apps
//!      because the current process runs its Drops cleanly before the new one
//!      opens a fresh compositor/GPU connection.
//!
//! State lives in `PaneFlowApp::self_update_status`. The title bar reads it
//! each render to flip the pill label between `available / Downloading… /
//! Installing…`. Errors are reported via a toast.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

/// Rendering-facing state of the self-update flow.
#[derive(Clone, Debug, Default)]
pub enum SelfUpdateStatus {
    /// No update operation in flight — the title bar shows `v{x} available`.
    #[default]
    Idle,
    Downloading,
    Installing,
    /// The error is preserved so future UI can surface it in a tooltip; the
    /// toast already displays it on entry.
    Errored(#[allow(dead_code)] Arc<anyhow::Error>),
}

impl SelfUpdateStatus {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            SelfUpdateStatus::Downloading | SelfUpdateStatus::Installing
        )
    }
}

/// Download the `.run` asset to a temp path and return it.
///
/// Streamed straight to disk (no full-file buffering) so large installers
/// don't spike memory. Blocking ureq is fine here — the caller runs us in
/// `smol::unblock` / `cx.background_spawn`.
pub fn download_installer(asset_url: &str) -> Result<PathBuf> {
    let target = std::env::temp_dir().join(format!("paneflow-update-{}.run", std::process::id()));

    let mut response = ureq::get(asset_url)
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("HTTP request failed for {asset_url}"))?;

    if !response.status().is_success() {
        anyhow::bail!("download returned HTTP {}", response.status());
    }

    let mut reader = response.body_mut().as_reader();
    let mut file =
        std::fs::File::create(&target).with_context(|| format!("create {}", target.display()))?;
    std::io::copy(&mut reader, &mut file).context("stream body to disk")?;
    file.sync_all().ok();

    // chmod +x — the installer is a bash self-extracting script, it needs
    // execute permission to run.
    let mut perms = std::fs::metadata(&target)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&target, perms)?;

    log::info!("self-update: downloaded {}", target.display());
    Ok(target)
}

/// Run the `.run` installer and wait for it to finish.
///
/// The installer is non-interactive: it extracts its payload and copies the
/// new binary to `~/.local/bin/paneflow`, then exits. No stdin is forwarded.
pub fn run_installer(path: &std::path::Path) -> Result<()> {
    let output = std::process::Command::new(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .with_context(|| format!("spawn {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "installer exited with {} — stderr: {}",
            output.status,
            stderr.trim()
        );
    }

    // Best-effort cleanup of the downloaded file.
    let _ = std::fs::remove_file(path);

    log::info!("self-update: installer completed successfully");
    Ok(())
}

/// Resolve the expected install location of the paneflow binary. The
/// installer writes here; we pass this path to `cx.set_restart_path()` so
/// GPUI's relaunch script execs the freshly installed binary.
pub fn installed_binary_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME environment variable is not set")?;
    Ok(PathBuf::from(home).join(".local/bin/paneflow"))
}
