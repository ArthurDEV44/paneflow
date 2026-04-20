//! Monospace font family enumeration.
//!
//! Per-OS strategy:
//!
//! - **Linux / BSDs** → `fc-list :spacing=mono family` (fontconfig). Widely
//!   available, fast, and returns the deduplicated family list we need.
//! - **macOS** (US-012) → Core Text via the `core-text = "21"` crate. The
//!   previous shared `fc-list` branch worked only when Homebrew's
//!   fontconfig was installed — a fresh macOS lacks it entirely, leaving
//!   the settings font picker empty. Core Text is macOS-native, has no
//!   install requirement, and returns the same family strings the OS
//!   already knows about (SF Mono, Menlo, Monaco, Courier, etc.).
//! - **Windows** → empty list + warning. DirectWrite enumeration is
//!   tracked in the Windows port PRD.
//!
//! All three branches share the contract: `Ok`-shaped return even on
//! failure. An empty list + `log::warn!` keeps the settings picker
//! renderable; panicking here would cascade into the whole settings
//! window failing to open.

#[cfg(windows)]
pub fn load_mono_fonts() -> Vec<String> {
    log::warn!("Windows font enumeration not yet wired — returning empty list");
    Vec::new()
}

/// macOS implementation (US-012).
///
/// Calls `CTFontCollectionCreateFromAvailableFonts` under the hood via
/// `core_text::font_collection::create_for_all_families`, iterates the
/// returned descriptors, instantiates a zero-sized `CTFont` per descriptor
/// (cheap — no glyph tables loaded at size 0), checks the symbolic
/// `kCTFontMonoSpaceTrait` bit (`0x00000400`) via `SymbolicTraitAccessors::is_monospace`,
/// and collects unique family names into a `BTreeSet` for stable ordering.
///
/// **Failure mode** (US-012 AC7): if Core Text returns no descriptors
/// (sandbox restriction, framework not loaded, OS bug) we return
/// `Vec::new()` and log a warning — the picker shows empty rather than
/// panicking. Mirrors the Linux `fc-list` branch's fallback semantics.
///
/// **Performance**: Core Text caches the font collection internally, so
/// subsequent calls return in well under the PRD's 200 ms warm-cache
/// budget. Cold-start cost on a system with a few hundred fonts is
/// typically ~30 ms on Apple Silicon — comfortably inside budget.
#[cfg(target_os = "macos")]
pub fn load_mono_fonts() -> Vec<String> {
    use std::collections::BTreeSet;

    use core_text::font as ct_font;
    use core_text::font_collection;
    use core_text::font_descriptor::SymbolicTraitAccessors;

    let collection = font_collection::create_for_all_families();
    let Some(descriptors) = collection.get_descriptors() else {
        log::warn!("Core Text font enumeration failed: no descriptors returned");
        return Vec::new();
    };

    let mut families: BTreeSet<String> = BTreeSet::new();
    for desc in descriptors.iter() {
        // `new_from_descriptor(&CTFontDescriptor, f64)` at size 0.0 is
        // the documented cheap-instantiation idiom — Core Text lazy-loads
        // tables only when the font is actually rendered. Dereferencing
        // `desc` (a `core_foundation::ItemRef`) yields `&CTFontDescriptor`
        // which matches the constructor's signature.
        let font = ct_font::new_from_descriptor(&desc, 0.0);
        if font.symbolic_traits().is_monospace() {
            families.insert(desc.family_name());
        }
    }

    families.into_iter().collect()
}

/// Linux / FreeBSD / OpenBSD / NetBSD / other unixes.
///
/// Keeps the `fc-list` path that was shared with macOS pre-US-012. Any
/// unix target that isn't macOS lands here; if fontconfig isn't present
/// the spawn fails cleanly and the picker renders empty (same
/// fallback-on-failure invariant as the macOS and Windows branches).
#[cfg(all(not(target_os = "macos"), not(windows)))]
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

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    /// Smoke test: Core Text on any real macOS install ships at least
    /// one monospace family (Menlo is bundled back to 10.6). An empty
    /// result means either Core Text is broken or the filter is wrong
    /// — either way, a regression the CI `macos-check` leg catches.
    #[test]
    fn core_text_returns_at_least_one_monospace_family() {
        let families = load_mono_fonts();
        assert!(
            !families.is_empty(),
            "expected at least one monospace family from Core Text, got none"
        );
    }

    /// The canonical macOS monospace fonts that ship with every release
    /// back to 10.6 — if none of these are present, the enumeration is
    /// almost certainly broken (or the filter is mis-classifying).
    #[test]
    fn core_text_includes_at_least_one_canonical_mono_family() {
        let families = load_mono_fonts();
        let canonical = ["Menlo", "Monaco", "Courier", "Courier New", "SF Mono"];
        let hit = canonical
            .iter()
            .find(|name| families.iter().any(|f| f == *name));
        assert!(
            hit.is_some(),
            "expected at least one of {:?} in enumerated families {:?}",
            canonical,
            families
        );
    }
}
