//! Font resolution + cell measurement for the terminal renderer.
//!
//! Owns the font fallback stack, the installed-monospace-font registry
//! (Linux), and the cached font config read from `paneflow.json`. Exposes
//! `measure_cell` — the sole entry point used by the renderer to turn the
//! current font + size into per-cell pixel dimensions.
//!
//! Extracted from `terminal_element.rs` per US-008 of the src-app refactor PRD.

#[cfg(target_os = "linux")]
use std::collections::HashSet;
use std::sync::LazyLock;

use gpui::{
    App, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Pixels, SharedString, Window, px,
};

use super::CellDimensions;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_LINE_HEIGHT: f32 = 1.3;

const FONT_FALLBACK_EMOJI: &str = "Noto Color Emoji";
const FONT_FALLBACK_SYMBOLS: &str = "Symbols Nerd Font Mono";
const FONT_FALLBACK_SANS: &str = "Noto Sans";

static FONT_FALLBACKS: LazyLock<FontFallbacks> = LazyLock::new(|| {
    FontFallbacks::from_fonts(vec![
        FONT_FALLBACK_EMOJI.to_string(),
        FONT_FALLBACK_SYMBOLS.to_string(),
        FONT_FALLBACK_SANS.to_string(),
    ])
});

#[cfg(target_os = "linux")]
static INSTALLED_MONO_FONTS: LazyLock<HashSet<String>> =
    LazyLock::new(|| crate::fonts::load_mono_fonts().into_iter().collect());

// ---------------------------------------------------------------------------
// Font config cache — avoids load_config() on every base_font()/font_size() call
// ---------------------------------------------------------------------------

struct CachedFontConfig {
    family: String,
    size: f32,
    line_height: f32,
    last_check: std::time::Instant,
}

static FONT_CONFIG_CACHE: std::sync::Mutex<Option<CachedFontConfig>> = std::sync::Mutex::new(None);

pub(crate) fn default_font_family() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Menlo"
    }

    #[cfg(target_os = "windows")]
    {
        "Cascadia Mono"
    }

    #[cfg(target_os = "linux")]
    {
        [
            "Ubuntu Mono",
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
        ]
        .into_iter()
        .find(|family| INSTALLED_MONO_FONTS.contains(*family))
        .unwrap_or("Noto Sans Mono")
    }

    #[cfg(all(
        not(target_os = "macos"),
        not(target_os = "windows"),
        not(target_os = "linux")
    ))]
    {
        "Noto Sans Mono"
    }
}

pub fn resolve_font_family(configured: Option<&str>) -> String {
    if let Some(family) = configured
        .map(str::trim)
        .filter(|family| !family.is_empty())
    {
        #[cfg(target_os = "linux")]
        {
            if INSTALLED_MONO_FONTS.contains(family) {
                return family.to_string();
            }

            let fallback = default_font_family();
            log::warn!(
                "font_family '{family}' is not installed as a monospace font; using '{fallback}'"
            );
            return fallback.to_string();
        }

        #[cfg(not(target_os = "linux"))]
        {
            return family.to_string();
        }
    }

    default_font_family().to_string()
}

/// Read font config, cached for 500ms (same pattern as theme cache).
pub(super) fn cached_font_config() -> (String, f32, f32) {
    use std::time::{Duration, Instant};
    const CHECK_INTERVAL: Duration = Duration::from_millis(500);

    let mut cache = FONT_CONFIG_CACHE.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref c) = *cache
        && c.last_check.elapsed() < CHECK_INTERVAL
    {
        return (c.family.clone(), c.size, c.line_height);
    }

    let config = paneflow_config::loader::load_config();

    let family = resolve_font_family(config.font_family.as_deref());

    let size = config
        .font_size
        .map(|s| {
            if (8.0..=32.0).contains(&s) {
                s
            } else {
                log::warn!(
                    "font_size {s} out of range [8.0, 32.0]; using default {DEFAULT_FONT_SIZE}"
                );
                DEFAULT_FONT_SIZE
            }
        })
        .unwrap_or(DEFAULT_FONT_SIZE);

    let line_height = config
        .line_height
        .map(|lh| {
            if (1.0..=2.5).contains(&lh) {
                lh
            } else {
                log::warn!(
                    "line_height {lh} out of range [1.0, 2.5]; using default {DEFAULT_LINE_HEIGHT}"
                );
                DEFAULT_LINE_HEIGHT
            }
        })
        .unwrap_or(DEFAULT_LINE_HEIGHT);

    *cache = Some(CachedFontConfig {
        family: family.clone(),
        size,
        line_height,
        last_check: Instant::now(),
    });

    (family, size, line_height)
}

pub(super) fn base_font() -> Font {
    let (family, _, _) = cached_font_config();
    Font {
        family: SharedString::from(family),
        features: FontFeatures::disable_ligatures(),
        fallbacks: Some(FONT_FALLBACKS.clone()),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

pub(super) fn font_size() -> Pixels {
    let (_, size, _) = cached_font_config();
    px(size)
}

pub fn measure_cell(window: &mut Window, _cx: &mut App) -> CellDimensions {
    let font = base_font();
    let font_size = font_size();
    let font_id = window.text_system().resolve_font(&font);
    let cell_width = window
        .text_system()
        .advance(font_id, font_size, 'm')
        .unwrap()
        .width;
    let (_, size_f32, multiplier) = cached_font_config();
    let line_height = px(size_f32 * multiplier);
    CellDimensions {
        cell_width,
        line_height,
    }
}
