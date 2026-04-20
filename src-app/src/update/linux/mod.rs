//! Linux-specific self-update runners.
//!
//! - [`appimage`] — zsync delta update via `appimageupdatetool`.
//! - [`targz`] — atomic directory swap under `$HOME/.local/paneflow.app/`.

pub mod appimage;
pub mod targz;
