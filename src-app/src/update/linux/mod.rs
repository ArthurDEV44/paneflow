//! Linux-specific self-update runners.
//!
//! - [`appimage`] — zsync delta update via `appimageupdatetool`.
//! - [`targz`] — atomic directory swap under `$HOME/.local/paneflow.app/`.
//! - [`system_package`] — pkexec-elevated `dnf`/`apt-get install` for users
//!   on the signed rpm/deb repo. Linux-only (uses
//!   `std::os::unix::process::ExitStatusExt`); gated at the declaration
//!   so macOS / Windows builds skip the whole module.

pub mod appimage;
pub mod targz;

#[cfg(target_os = "linux")]
pub mod system_package;
