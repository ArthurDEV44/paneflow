//! macOS DMG update runner — stub until a real implementation lands.

use anyhow::{Result, bail};
use std::path::PathBuf;

/// Placeholder for the DMG update flow. Replace with a mount/copy/install
/// pipeline once the macOS signed DMG pipeline ships.
#[allow(dead_code)]
pub fn run_dmg_update(_asset_url: &str) -> Result<PathBuf> {
    bail!("macOS DMG self-update not yet implemented");
}
