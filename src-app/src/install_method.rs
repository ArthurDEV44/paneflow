//! Runtime detection of how PaneFlow was installed.
//!
//! The in-app updater needs to pick different strategies depending on whether
//! the running binary came from a distro package, an AppImage, a user-local
//! tar.gz install, or an unknown location. We determine this from the binary
//! path alone — no config, no env var (except `$APPIMAGE`, which the AppImage
//! runtime already sets for us).
//!
//! Detection runs at startup. The caller canonicalises `current_exe()` before
//! classifying, so a symlink like `~/.local/bin/paneflow ->
//! ~/.local/paneflow.app/bin/paneflow` resolves to the real path and is
//! correctly identified as `TarGz`.
//!
//! Every public API in this module is consumed by the updater work in
//! US-009/010/011/012. Until those stories land, much of it is only
//! reachable through the unit tests — hence the crate-level dead-code
//! suppression.

#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Package manager used for system-wide installs. Advisory only — the updater
/// uses this to render the correct "update via apt/dnf" hint in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    /// `/usr/bin/paneflow` exists but neither `/etc/debian_version` nor
    /// `/etc/fedora-release` are present (e.g., `eopkg` on Solus, `xbps` on
    /// Void). The UI falls back to a generic "via your package manager" hint.
    Other,
}

/// How the running binary was installed on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    /// `/usr/bin/paneflow` or `/usr/local/bin/paneflow` — the apt/dnf managed
    /// binary. In-app updates are disabled; user is pointed at the system
    /// package manager.
    SystemPackage { manager: PackageManager },

    /// Launched from a mounted AppImage (`/tmp/.mount_*/...`). Update flow
    /// delegates to `appimageupdatetool` on the source `.AppImage` file.
    AppImage {
        mount_point: PathBuf,
        source_path: PathBuf,
    },

    /// Installed by the tar.gz installer under `$HOME/.local/paneflow.app/`.
    /// Update flow downloads a new tarball and atomically swaps the app dir.
    TarGz { app_dir: PathBuf },

    /// Binary location doesn't match any known layout (legacy `.run` install,
    /// manual copy, dev build). Updater disables in-app updates.
    Unknown,
}

/// Probe the filesystem and environment to classify the running binary.
pub fn detect() -> InstallMethod {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallMethod::Unknown,
    };
    // Canonicalise resolves symlinks and `..` segments. If it fails (unlikely),
    // fall back to the raw exe path.
    let canonical = std::fs::canonicalize(&exe).unwrap_or(exe);

    classify(
        &canonical,
        std::env::var_os("HOME"),
        std::env::var_os("APPIMAGE"),
    )
}

/// Pure classifier — no I/O beyond the `/etc/*-release` probe for package
/// manager inference, which is only reached on the SystemPackage arm. All
/// other inputs are parameters so callers (and tests) control them.
fn classify(canonical: &Path, home: Option<OsString>, appimage: Option<OsString>) -> InstallMethod {
    // 1. System package (apt/dnf).
    if canonical == Path::new("/usr/bin/paneflow")
        || canonical == Path::new("/usr/local/bin/paneflow")
    {
        return InstallMethod::SystemPackage {
            manager: detect_package_manager(),
        };
    }

    // 2. AppImage — mounted under /tmp/.mount_XXXXXX/. The path prefix alone
    //    is enough; $APPIMAGE is only used to locate the source .AppImage file
    //    for the updater. Works even when the user launched the AppImage in a
    //    non-standard way that didn't set $APPIMAGE.
    if let Some(mount_point) = appimage_mount_point(canonical) {
        let source_path = appimage.map(PathBuf::from).unwrap_or_default();
        return InstallMethod::AppImage {
            mount_point,
            source_path,
        };
    }

    // 3. Tar.gz install under $HOME/.local/paneflow.app/.
    if let Some(home_path) = home.map(PathBuf::from) {
        let app_dir = home_path.join(".local").join("paneflow.app");
        if canonical.starts_with(&app_dir) {
            return InstallMethod::TarGz { app_dir };
        }
    }

    InstallMethod::Unknown
}

/// Infer the system package manager from distro-identifier files.
///
/// Only reached when the binary is at `/usr/bin` or `/usr/local/bin` — i.e.
/// we already know a system package put it there. Returns `Other` when
/// neither Debian nor Fedora markers are found so the UI can degrade to a
/// generic hint instead of pretending `apt` is available.
fn detect_package_manager() -> PackageManager {
    if Path::new("/etc/debian_version").exists() {
        PackageManager::Apt
    } else if Path::new("/etc/fedora-release").exists() {
        PackageManager::Dnf
    } else {
        PackageManager::Other
    }
}

/// Return the `/tmp/.mount_XXXXXX/` directory if `path` lives inside a
/// mounted AppImage, else `None`. Matches the AppImage runtime's naming
/// convention.
fn appimage_mount_point(path: &Path) -> Option<PathBuf> {
    let mut comps = path.components();
    // `/` (root)
    if !matches!(comps.next()?, Component::RootDir) {
        return None;
    }
    // `tmp`
    if comps.next()?.as_os_str() != "tmp" {
        return None;
    }
    // `.mount_XXXXXX`
    let mount = comps.next()?;
    let mount_str = mount.as_os_str().to_str()?;
    if !mount_str.starts_with(".mount_") {
        return None;
    }
    // Require at least one component after the mount dir — `current_exe()`
    // always returns a file path, so a bare mount dir would be impossible,
    // but the guard makes the classifier resistant to malformed inputs.
    comps.next()?;

    Some(Path::new("/tmp").join(mount_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_package_usr_bin() {
        let r = classify(Path::new("/usr/bin/paneflow"), None, None);
        assert!(matches!(r, InstallMethod::SystemPackage { .. }));
    }

    #[test]
    fn system_package_usr_local_bin() {
        let r = classify(Path::new("/usr/local/bin/paneflow"), None, None);
        assert!(matches!(r, InstallMethod::SystemPackage { .. }));
    }

    #[test]
    fn appimage_with_env() {
        let r = classify(
            Path::new("/tmp/.mount_abc123/usr/bin/paneflow"),
            None,
            Some(OsString::from("/home/u/Downloads/paneflow.AppImage")),
        );
        match r {
            InstallMethod::AppImage {
                mount_point,
                source_path,
            } => {
                assert_eq!(mount_point, Path::new("/tmp/.mount_abc123"));
                assert_eq!(
                    source_path,
                    Path::new("/home/u/Downloads/paneflow.AppImage")
                );
            }
            other => panic!("expected AppImage, got {other:?}"),
        }
    }

    #[test]
    fn appimage_without_env_still_detected() {
        let r = classify(Path::new("/tmp/.mount_abc123/usr/bin/paneflow"), None, None);
        match r {
            InstallMethod::AppImage {
                mount_point,
                source_path,
            } => {
                assert_eq!(mount_point, Path::new("/tmp/.mount_abc123"));
                assert_eq!(source_path, PathBuf::new());
            }
            other => panic!("expected AppImage, got {other:?}"),
        }
    }

    #[test]
    fn tar_gz_under_home_app_dir() {
        let r = classify(
            Path::new("/home/u/.local/paneflow.app/bin/paneflow"),
            Some(OsString::from("/home/u")),
            None,
        );
        match r {
            InstallMethod::TarGz { app_dir } => {
                assert_eq!(app_dir, Path::new("/home/u/.local/paneflow.app"));
            }
            other => panic!("expected TarGz, got {other:?}"),
        }
    }

    #[test]
    fn unknown_for_legacy_run_install() {
        let r = classify(
            Path::new("/home/u/.local/bin/paneflow"),
            Some(OsString::from("/home/u")),
            None,
        );
        assert_eq!(r, InstallMethod::Unknown);
    }

    #[test]
    fn unknown_for_random_path() {
        let r = classify(
            Path::new("/opt/random/paneflow"),
            Some(OsString::from("/home/u")),
            None,
        );
        assert_eq!(r, InstallMethod::Unknown);
    }

    #[test]
    fn appimage_mount_parsing() {
        assert_eq!(
            appimage_mount_point(Path::new("/tmp/.mount_abc/usr/bin/paneflow")),
            Some(PathBuf::from("/tmp/.mount_abc"))
        );
        // Non-`/tmp` root
        assert!(appimage_mount_point(Path::new("/var/.mount_abc/paneflow")).is_none());
        // Non-`.mount_` prefix
        assert!(appimage_mount_point(Path::new("/tmp/foo/paneflow")).is_none());
        // Bare mount dir (no binary beneath)
        assert!(appimage_mount_point(Path::new("/tmp/.mount_x")).is_none());
    }

    /// Proves that `canonicalize` resolves a symlink chain mimicking the
    /// tar.gz install layout. Detection in `detect()` relies on this so the
    /// symlink at `~/.local/bin/paneflow` doesn't get misclassified.
    #[test]
    fn canonicalize_resolves_tar_gz_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let app_dir = tmp.path().join(".local/paneflow.app/bin");
        std::fs::create_dir_all(&app_dir).unwrap();
        let real_bin = app_dir.join("paneflow");
        std::fs::write(&real_bin, b"").unwrap();

        let bin_dir = tmp.path().join(".local/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let sym = bin_dir.join("paneflow");
        std::os::unix::fs::symlink(&real_bin, &sym).unwrap();

        let canonical = std::fs::canonicalize(&sym).unwrap();
        let r = classify(&canonical, Some(OsString::from(tmp.path())), None);
        match r {
            InstallMethod::TarGz { app_dir } => {
                assert_eq!(app_dir, tmp.path().join(".local/paneflow.app"));
            }
            other => panic!("expected TarGz, got {other:?}"),
        }
    }
}
