//! Monospace font family enumeration.
//!
//! On Linux/macOS, spawns `fc-list` (fontconfig) to list installed monospace
//! families. On Windows, returns an empty list and logs a warning — native
//! enumeration via DirectWrite is tracked in the Windows port PRD.

#[cfg(windows)]
pub fn load_mono_fonts() -> Vec<String> {
    log::warn!("Windows font enumeration not yet wired — returning empty list");
    Vec::new()
}

#[cfg(not(windows))]
pub fn load_mono_fonts() -> Vec<String> {
    use std::collections::BTreeSet;

    let output = match std::process::Command::new("fc-list")
        .args([":spacing=mono", "family"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("fonts: fc-list failed: {e}");
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut families = BTreeSet::new();

    for line in stdout.lines() {
        // fc-list may output "Family1,Family2" on a single line
        for part in line.split(',') {
            let name = part.trim();
            if !name.is_empty() {
                families.insert(name.to_string());
            }
        }
    }

    families.into_iter().collect()
}
