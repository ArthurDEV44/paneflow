//! Background update checker — queries GitHub Releases API at startup,
//! deposits the result into a shared slot for the main thread to pick up.
//!
//! US-009 adds arch-+-format asset matching so users only ever see an asset
//! that matches both their CPU architecture and their install method (never
//! a .deb handed to a Fedora user).

use semver::Version;

use crate::install_method::{self, InstallMethod, PackageManager};

const GITHUB_API: &str = "https://api.github.com/repos/ArthurDEV44/paneflow/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Release-asset format the update checker advertises to the UI.
///
/// Filename convention: `paneflow-<version>-<arch>[<target-qualifier>].<format-suffix>`.
/// Linux formats carry no qualifier (e.g. `paneflow-v0.2.0-x86_64.deb`),
/// while macOS `Dmg` uses the Rust target-triple tail `-apple-darwin`
/// (e.g. `paneflow-0.2.0-aarch64-apple-darwin.dmg`) and Windows `Msi`
/// uses the `-pc-windows-msvc` tail (e.g.
/// `paneflow-0.2.0-x86_64-pc-windows-msvc.msi`). See
/// [`AssetFormat::filename_suffix`] and [`AssetFormat::target_qualifier`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetFormat {
    Deb,
    Rpm,
    AppImage,
    TarGz,
    Dmg,
    /// Windows MSI installer (US-011 — prd-windows-port.md). Produced by
    /// `cargo-wix` (see US-013), signed via Azure Trusted Signing in
    /// `release.yml` (US-016). Paired with [`InstallMethod::WindowsMsi`].
    Msi,
}

impl AssetFormat {
    /// Canonical filename suffix the CI emits for this format. Matching is
    /// performed case-insensitively so a release with `.DEB` still works.
    fn filename_suffix(&self) -> &'static str {
        match self {
            AssetFormat::Deb => ".deb",
            AssetFormat::Rpm => ".rpm",
            AssetFormat::AppImage => ".AppImage",
            AssetFormat::TarGz => ".tar.gz",
            AssetFormat::Dmg => ".dmg",
            AssetFormat::Msi => ".msi",
        }
    }

    /// Target-triple qualifier inserted between the arch and the suffix.
    ///
    /// Linux formats emit bare `<arch><suffix>` (historical convention,
    /// preserved for regression safety). macOS `.dmg` files carry the
    /// `-apple-darwin` tail and Windows `.msi` files carry the
    /// `-pc-windows-msvc` tail because GitHub Releases host artifacts for
    /// all platforms side by side — a bare `-x86_64.msi` would collide
    /// visually with `-x86_64.deb` in the releases listing.
    fn target_qualifier(&self) -> &'static str {
        match self {
            AssetFormat::Dmg => "-apple-darwin",
            AssetFormat::Msi => "-pc-windows-msvc",
            _ => "",
        }
    }

    /// Pick the right asset format for a given install method.
    ///
    /// `Unknown` falls back to `.tar.gz` because that's the only format that
    /// works without root and without a specific package manager — the safe
    /// default for dev builds and legacy `.run` migrations.
    fn from_install_method(method: &InstallMethod) -> Self {
        match method {
            InstallMethod::SystemPackage {
                manager: PackageManager::Apt,
            } => AssetFormat::Deb,
            InstallMethod::SystemPackage {
                manager: PackageManager::Dnf,
            } => AssetFormat::Rpm,
            // A system install on a non-apt/dnf distro is effectively a dead
            // end for the in-app updater (the click handler short-circuits to
            // the hint toast), so any format works. TarGz is the neutral
            // fallback mirroring `InstallMethod::Unknown`.
            InstallMethod::SystemPackage {
                manager: PackageManager::Other,
            } => AssetFormat::TarGz,
            InstallMethod::AppImage { .. } => AssetFormat::AppImage,
            InstallMethod::TarGz { .. } => AssetFormat::TarGz,
            InstallMethod::AppBundle { .. } => AssetFormat::Dmg,
            // US-011 — Windows MSI installs take the signed `.msi` asset
            // for `x86_64-pc-windows-msvc`. Paired with `InstallMethod::WindowsMsi`
            // detected in US-010; the MSI is produced + signed by the
            // release pipeline in US-013/US-015/US-016.
            InstallMethod::WindowsMsi { .. } => AssetFormat::Msi,
            InstallMethod::Unknown => AssetFormat::TarGz,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    Checking,
    Available {
        version: String,
        /// GitHub release HTML page — always populated. The title bar opens
        /// this in a browser as a fallback when `asset_url` is `None`.
        url: String,
        /// Direct download URL for the arch-+-format-matched asset. `None`
        /// when the release has no asset matching the current host+method.
        asset_url: Option<String>,
        /// Format of the picked asset. Drives UI messaging in US-010/011/012
        /// ("Update via apt" vs "Download new AppImage"). `None` when
        /// `asset_url` is also `None`.
        asset_format: Option<AssetFormat>,
    },
    UpToDate,
    Failed,
}

pub type SharedUpdateSlot = std::sync::Arc<std::sync::Mutex<Option<UpdateStatus>>>;

/// Spawn a detached thread that checks GitHub for a newer release.
/// The result is deposited into the returned shared slot.
pub fn spawn_check() -> SharedUpdateSlot {
    let slot: SharedUpdateSlot =
        std::sync::Arc::new(std::sync::Mutex::new(Some(UpdateStatus::Checking)));
    let writer = std::sync::Arc::clone(&slot);
    std::thread::spawn(move || {
        let status = check_github_release();
        *writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
    });
    slot
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
pub(crate) struct GitHubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

/// Pick the release asset that matches both the host architecture and the
/// install method's expected format.
///
/// Matching is strict: a Fedora (`Dnf`) user is never handed a `.deb`; an
/// AppImage user is never handed a `.tar.gz`. When the release is missing
/// the expected format, the function returns `None` and the UI falls back
/// to opening the release page in a browser.
///
/// # Filename convention
/// Expects assets named `paneflow-<version>-<arch>[<qualifier>].<format-suffix>`:
/// Linux: `paneflow-v0.2.0-x86_64.deb` (no qualifier).
/// macOS: `paneflow-0.2.0-aarch64-apple-darwin.dmg` (target-triple qualifier).
/// Sibling files like `paneflow-v0.2.0-x86_64.AppImage.zsync` are naturally
/// rejected because their suffix is `.zsync`, not `.AppImage`.
pub fn pick_asset<'a>(
    assets: &'a [GitHubAsset],
    arch: &str,
    method: InstallMethod,
) -> Option<&'a GitHubAsset> {
    let format = AssetFormat::from_install_method(&method);
    let expected = format!(
        "-{arch}{}{}",
        format.target_qualifier(),
        format.filename_suffix()
    )
    .to_ascii_lowercase();
    assets
        .iter()
        .find(|a| a.name.to_ascii_lowercase().ends_with(&expected))
}

fn check_github_release() -> UpdateStatus {
    let response = ureq::get(GITHUB_API)
        .header("User-Agent", &format!("paneflow/{CURRENT_VERSION}"))
        .header("Accept", "application/vnd.github.v3+json")
        .call();

    let mut response = match response {
        Ok(r) => r,
        Err(e) => {
            log::warn!("update check failed: {e}");
            return UpdateStatus::Failed;
        }
    };

    let release: GitHubRelease = match response.body_mut().read_json() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("update check: failed to parse response: {e}");
            return UpdateStatus::Failed;
        }
    };

    let remote_tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);

    let remote = match Version::parse(remote_tag) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "update check: invalid remote version '{}': {e}",
                release.tag_name
            );
            return UpdateStatus::Failed;
        }
    };
    let local = match Version::parse(CURRENT_VERSION) {
        Ok(v) => v,
        Err(_) => return UpdateStatus::Failed,
    };

    if remote > local {
        let method = install_method::detect();
        let picked = pick_asset(&release.assets, std::env::consts::ARCH, method.clone());
        let (asset_url, asset_format) = match picked {
            Some(asset) => (
                Some(asset.browser_download_url.clone()),
                Some(AssetFormat::from_install_method(&method)),
            ),
            None => (None, None),
        };
        log::info!(
            "update available: v{remote} (current: v{local}) — asset_format: {asset_format:?}"
        );
        UpdateStatus::Available {
            version: remote.to_string(),
            url: release.html_url,
            asset_url,
            asset_format,
        }
    } else {
        log::info!("up to date (v{local})");
        UpdateStatus::UpToDate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_asset(name: &str) -> GitHubAsset {
        GitHubAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
        }
    }

    fn apt() -> InstallMethod {
        InstallMethod::SystemPackage {
            manager: PackageManager::Apt,
        }
    }
    fn dnf() -> InstallMethod {
        InstallMethod::SystemPackage {
            manager: PackageManager::Dnf,
        }
    }
    fn tar_gz() -> InstallMethod {
        InstallMethod::TarGz {
            app_dir: PathBuf::from("/home/u/.local/paneflow.app"),
        }
    }
    fn app_image() -> InstallMethod {
        InstallMethod::AppImage {
            mount_point: PathBuf::from("/tmp/.mount_x"),
            source_path: PathBuf::from("/home/u/Downloads/paneflow.AppImage"),
        }
    }
    fn app_bundle() -> InstallMethod {
        InstallMethod::AppBundle {
            bundle_path: PathBuf::from("/Applications/PaneFlow.app"),
        }
    }
    fn windows_msi() -> InstallMethod {
        // Forward-slash install path intentional — see US-010's test
        // header for why `Path::starts_with` needs forward slashes in
        // Linux CI. Not consumed by `pick_asset` anyway (only the variant
        // discriminant matters here).
        InstallMethod::WindowsMsi {
            install_path: PathBuf::from("C:/Program Files/PaneFlow"),
        }
    }

    #[test]
    fn apt_picks_deb() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.deb"),
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
            make_asset("paneflow-v0.2.0-x86_64.AppImage"),
        ];
        let r = pick_asset(&assets, "x86_64", apt());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.deb")
        );
    }

    #[test]
    fn dnf_picks_rpm() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.rpm"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
        ];
        let r = pick_asset(&assets, "x86_64", dnf());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.rpm")
        );
    }

    #[test]
    fn appimage_method_picks_appimage() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.AppImage"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
        ];
        let r = pick_asset(&assets, "x86_64", app_image());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.AppImage")
        );
    }

    #[test]
    fn tar_gz_method_picks_tar_gz() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
        ];
        let r = pick_asset(&assets, "x86_64", tar_gz());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.tar.gz")
        );
    }

    #[test]
    fn tar_gz_method_picks_tar_gz_aarch64() {
        // US-019 AC5 regression test. A multi-arch release carries both
        // x86_64 and aarch64 assets; an aarch64 host using the TarGz
        // install method must receive the aarch64 tar.gz, never the
        // x86_64 one and never an arch-mismatched .deb.
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
            make_asset("paneflow-v0.2.0-aarch64.tar.gz"),
            make_asset("paneflow-v0.2.0-aarch64.deb"),
        ];
        let r = pick_asset(&assets, "aarch64", tar_gz());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-aarch64.tar.gz")
        );
    }

    #[test]
    fn unknown_method_falls_back_to_tar_gz() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
            make_asset("paneflow-v0.2.0-x86_64.AppImage"),
        ];
        let r = pick_asset(&assets, "x86_64", InstallMethod::Unknown);
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.tar.gz")
        );
    }

    #[test]
    fn fedora_never_handed_deb_fallback() {
        // Release has .deb + .tar.gz but NO .rpm. Fedora user must get
        // `None`, not a cross-format `.deb`.
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.deb"),
            make_asset("paneflow-v0.2.0-x86_64.tar.gz"),
        ];
        let r = pick_asset(&assets, "x86_64", dnf());
        assert!(r.is_none(), "Fedora user must NOT receive a .deb");
    }

    #[test]
    fn multi_arch_release_picks_correct_arch() {
        let assets = vec![
            make_asset("paneflow-v0.2.0-aarch64.deb"),
            make_asset("paneflow-v0.2.0-x86_64.deb"),
        ];
        let x = pick_asset(&assets, "x86_64", apt());
        assert_eq!(
            x.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.deb")
        );
        let a = pick_asset(&assets, "aarch64", apt());
        assert_eq!(
            a.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-aarch64.deb")
        );
    }

    #[test]
    fn match_is_case_insensitive() {
        let assets = vec![make_asset("PaneFlow-v0.2.0-X86_64.DEB")];
        let r = pick_asset(&assets, "x86_64", apt());
        assert!(r.is_some(), "case-insensitive match failed");
    }

    #[test]
    fn returns_none_when_no_matching_asset() {
        let assets = vec![
            make_asset("README.md"),
            make_asset("paneflow-v0.2.0-x86_64.AppImage.zsync"),
        ];
        let r = pick_asset(&assets, "x86_64", tar_gz());
        assert!(r.is_none());
    }

    #[test]
    fn zsync_sidecar_never_picked_for_appimage() {
        // The CI produces both paneflow-*.AppImage and its .AppImage.zsync
        // sidecar. The matcher must prefer the runnable .AppImage, never the
        // .zsync metadata file.
        let assets = vec![
            make_asset("paneflow-v0.2.0-x86_64.AppImage.zsync"),
            make_asset("paneflow-v0.2.0-x86_64.AppImage"),
        ];
        let r = pick_asset(&assets, "x86_64", app_image());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-v0.2.0-x86_64.AppImage")
        );
    }

    #[test]
    fn format_from_install_method_mapping() {
        assert_eq!(AssetFormat::from_install_method(&apt()), AssetFormat::Deb);
        assert_eq!(AssetFormat::from_install_method(&dnf()), AssetFormat::Rpm);
        assert_eq!(
            AssetFormat::from_install_method(&tar_gz()),
            AssetFormat::TarGz
        );
        assert_eq!(
            AssetFormat::from_install_method(&app_image()),
            AssetFormat::AppImage
        );
        assert_eq!(
            AssetFormat::from_install_method(&InstallMethod::Unknown),
            AssetFormat::TarGz
        );
        // US-008 AC6: AppBundle pairs with Dmg.
        assert_eq!(
            AssetFormat::from_install_method(&app_bundle()),
            AssetFormat::Dmg
        );
    }

    // -- US-008 ---------------------------------------------------------

    #[test]
    fn app_bundle_picks_dmg_aarch64() {
        // AC2: aarch64 macOS host picks the aarch64-apple-darwin.dmg.
        let assets = vec![
            make_asset("paneflow-0.2.0-aarch64-apple-darwin.dmg"),
            make_asset("paneflow-0.2.0-x86_64-apple-darwin.dmg"),
            make_asset("paneflow-0.2.0-aarch64.tar.gz"),
        ];
        let r = pick_asset(&assets, "aarch64", app_bundle());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-0.2.0-aarch64-apple-darwin.dmg")
        );
    }

    #[test]
    fn app_bundle_picks_dmg_x86_64() {
        // AC3: x86_64 macOS host picks the x86_64-apple-darwin.dmg.
        let assets = vec![
            make_asset("paneflow-0.2.0-aarch64-apple-darwin.dmg"),
            make_asset("paneflow-0.2.0-x86_64-apple-darwin.dmg"),
            make_asset("paneflow-0.2.0-x86_64.deb"),
        ];
        let r = pick_asset(&assets, "x86_64", app_bundle());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-0.2.0-x86_64-apple-darwin.dmg")
        );
    }

    #[test]
    fn app_bundle_returns_none_when_release_has_no_dmg() {
        // AC4: Linux-only hotfix release — macOS user gets None, not a .deb.
        let assets = vec![
            make_asset("paneflow-0.2.0-x86_64.deb"),
            make_asset("paneflow-0.2.0-aarch64.tar.gz"),
            make_asset("paneflow-0.2.0-x86_64.AppImage"),
        ];
        let r = pick_asset(&assets, "aarch64", app_bundle());
        assert!(
            r.is_none(),
            "AppBundle user must NOT be handed a Linux asset"
        );
    }

    #[test]
    fn linux_never_picks_dmg() {
        // AC5 regression: an apt user on aarch64 must not accidentally match
        // a `.dmg` just because its filename starts with `-aarch64`.
        let assets = vec![
            make_asset("paneflow-0.2.0-aarch64-apple-darwin.dmg"),
            make_asset("paneflow-0.2.0-aarch64.deb"),
        ];
        let r = pick_asset(&assets, "aarch64", apt());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-0.2.0-aarch64.deb")
        );
    }

    #[test]
    fn dmg_match_is_case_insensitive() {
        // AC1: filename matching stays case-insensitive for Dmg too.
        let assets = vec![make_asset("PaneFlow-0.2.0-AArch64-Apple-Darwin.DMG")];
        let r = pick_asset(&assets, "aarch64", app_bundle());
        assert!(r.is_some(), "case-insensitive .dmg match failed");
    }

    #[test]
    fn dmg_arch_mismatch_returns_none() {
        // x86_64 host on a release that only shipped an aarch64 .dmg.
        let assets = vec![make_asset("paneflow-0.2.0-aarch64-apple-darwin.dmg")];
        let r = pick_asset(&assets, "x86_64", app_bundle());
        assert!(r.is_none());
    }

    // -- US-011 — Windows MSI asset matching. -----------------------------

    #[test]
    fn windows_msi_picks_msi_x86_64() {
        // AC2: x86_64 Windows host picks the x86_64-pc-windows-msvc.msi.
        let assets = vec![
            make_asset("paneflow-0.2.0-x86_64-pc-windows-msvc.msi"),
            make_asset("paneflow-0.2.0-x86_64.deb"),
            make_asset("paneflow-0.2.0-x86_64-apple-darwin.dmg"),
        ];
        let r = pick_asset(&assets, "x86_64", windows_msi());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-0.2.0-x86_64-pc-windows-msvc.msi")
        );
    }

    #[test]
    fn windows_msi_returns_none_when_release_has_no_msi() {
        // AC3: Linux-only hotfix — Windows user gets None, update prompt
        // silently defers, no Linux asset is ever handed to the MSI flow.
        let assets = vec![
            make_asset("paneflow-0.2.0-x86_64.deb"),
            make_asset("paneflow-0.2.0-x86_64.tar.gz"),
            make_asset("paneflow-0.2.0-x86_64.AppImage"),
        ];
        let r = pick_asset(&assets, "x86_64", windows_msi());
        assert!(
            r.is_none(),
            "WindowsMsi user must NOT be handed a Linux/macOS asset"
        );
    }

    #[test]
    fn linux_never_picks_msi() {
        // AC5 regression: an apt user on x86_64 must not accidentally match
        // a `.msi` just because its filename starts with `-x86_64`.
        let assets = vec![
            make_asset("paneflow-0.2.0-x86_64-pc-windows-msvc.msi"),
            make_asset("paneflow-0.2.0-x86_64.deb"),
        ];
        let r = pick_asset(&assets, "x86_64", apt());
        assert_eq!(
            r.map(|a| a.name.as_str()),
            Some("paneflow-0.2.0-x86_64.deb")
        );
    }

    #[test]
    fn msi_match_is_case_insensitive() {
        // Mirrors `dmg_match_is_case_insensitive`: filename matching stays
        // case-insensitive for Msi.
        let assets = vec![make_asset("PaneFlow-0.2.0-X86_64-PC-Windows-Msvc.MSI")];
        let r = pick_asset(&assets, "x86_64", windows_msi());
        assert!(r.is_some(), "case-insensitive .msi match failed");
    }
}
