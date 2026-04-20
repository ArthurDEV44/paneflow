//! Windows MSI update runner — stub until a real implementation lands.

use anyhow::{Result, bail};
use std::path::PathBuf;

/// Placeholder for the MSI update flow. Replace with a download + `msiexec
/// /i /quiet` pipeline once the Windows signed MSI pipeline ships.
#[allow(dead_code)]
pub fn run_msi_update(_asset_url: &str) -> Result<PathBuf> {
    bail!("Windows MSI self-update not yet implemented");
}
