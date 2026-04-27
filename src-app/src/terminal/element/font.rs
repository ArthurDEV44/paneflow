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
    /// US-008: render ligatures when `true`. Hot-reload comes for free
    /// via the surrounding 500 ms cache: editing `paneflow.json` is
    /// picked up on the next `cached_font_config()` call without any
    /// extra wiring.
    ligatures: bool,
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
pub(super) fn cached_font_config() -> (String, f32, f32, bool) {
    use std::time::{Duration, Instant};
    const CHECK_INTERVAL: Duration = Duration::from_millis(500);

    let mut cache = FONT_CONFIG_CACHE.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref c) = *cache
        && c.last_check.elapsed() < CHECK_INTERVAL
    {
        return (c.family.clone(), c.size, c.line_height, c.ligatures);
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

    // US-008: ligatures are off by default to preserve the historical
    // monospaced cell-stride behavior. Opt-in via `terminal.ligatures: true`
    // in `paneflow.json`. Both `terminal == None` and
    // `terminal.ligatures == None` keep ligatures disabled.
    let ligatures = config
        .terminal
        .as_ref()
        .and_then(|t| t.ligatures)
        .unwrap_or(false);

    *cache = Some(CachedFontConfig {
        family: family.clone(),
        size,
        line_height,
        ligatures,
        last_check: Instant::now(),
    });

    (family, size, line_height, ligatures)
}

pub(super) fn base_font() -> Font {
    let (family, _, _, ligatures) = cached_font_config();
    // US-008: when the user opts into ligatures, hand GPUI the font's
    // native feature set untouched. Default behavior (and explicit
    // `ligatures: false`) keeps the historical `disable_ligatures()`
    // call so a Paneflow upgrade never silently flips ligatures on.
    let features = if ligatures {
        FontFeatures::default()
    } else {
        FontFeatures::disable_ligatures()
    };
    Font {
        family: SharedString::from(family),
        features,
        fallbacks: Some(FONT_FALLBACKS.clone()),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

pub(super) fn font_size() -> Pixels {
    let (_, size, _, _) = cached_font_config();
    px(size)
}

pub fn measure_cell(window: &mut Window, _cx: &mut App) -> CellDimensions {
    let font = base_font();
    let font_size = font_size();
    let font_id = window.text_system().resolve_font(&font);

    // Raw advance width for 'm' in the resolved font. If the text system
    // can't measure (font load failed, glyph missing, etc.) fall back to
    // `font_size` rather than panic — a slightly-too-wide cell (~1.5–1.7×
    // a typical monospace 'm') is far better than a crashed renderer, and
    // the cached font config already validates the family + size before
    // we get here. Note: on this fallback path the PTY `SIGWINCH`
    // `ws_xpixel` value is also approximate, since it is derived from the
    // same `cell_width` (see resize handling in `mod.rs`).
    let cell_width_raw = window
        .text_system()
        .advance(font_id, font_size, 'm')
        .map(|advance| advance.width)
        .unwrap_or_else(|err| {
            log::warn!(
                "text_system().advance('m') failed in measure_cell: {err}; falling back to font_size={}px",
                font_size.as_f32()
            );
            font_size
        });

    let (_, size_f32, multiplier, _) = cached_font_config();
    let line_height_raw = px(size_f32 * multiplier);

    // US-002: snap raw font measurements to integer pixels via `.round()`
    // (WezTerm convention — minimizes layout-area drift on fractional
    // advances vs `floor`/`ceil`). Quantizing the cell stride at measure
    // time means every downstream coordinate `cell_width * col` is also
    // integer, eliminating the fractional residual that prevents adjacent
    // quads from sharing a pixel boundary. Trade-off: column count
    // `viewport / cell_width` may shift by ±1 on extreme aspect ratios.
    // Acceptable for pixel-perfect rendering (US-001 / US-003 / US-004).
    let cell_width = cell_width_raw.round();
    let line_height = line_height_raw.round();

    // PANEFLOW_PIXEL_PROBE: record both raw and snapped cell dimensions so a
    // future investigation can tell at a glance whether the snap was a
    // no-op (`raw == snapped`) or quantized a fractional residual. Origin
    // is logged separately from `paint()` via `record_origin`.
    #[cfg(debug_assertions)]
    super::pixel_probe::record_cell_dimensions(
        cell_width_raw,
        cell_width,
        line_height_raw,
        line_height,
        window.scale_factor(),
    );

    CellDimensions {
        cell_width,
        line_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_snap_no_op_for_integer_advance() {
        // When the font system already returns an integer advance (the case
        // observed in `debug_block_char_rendering.md`: cell_width=9.0), the
        // snap must be a no-op so US-002 introduces no regression on
        // already-aligned environments.
        let raw = px(9.0);
        assert_eq!(raw.round(), raw);

        let raw_lh = px(20.0);
        assert_eq!(raw_lh.round(), raw_lh);
    }

    #[test]
    fn round_snap_yields_integer_for_fractional_advance() {
        // 8.4 px advance is the canonical fractional case from the PRD
        // (DejaVu Sans Mono at 14 pt @ 1.0 DPI on Linux).
        let raw = px(8.4);
        let snapped = raw.round();
        assert_eq!(snapped, px(8.0));
        assert!(
            snapped.as_f32().fract().abs() < 1e-6,
            "snapped 8.4 should be integer, got {}",
            snapped.as_f32()
        );
    }

    #[test]
    fn round_snap_handles_half_away_from_zero() {
        // Rust's f32::round documents round-half-away-from-zero. Lock in
        // that behavior so a future `.round()` → `.round_ties_even()` swap
        // would surface here instead of as a silent layout shift.
        assert_eq!(px(8.5).round(), px(9.0));
        assert_eq!(px(8.6).round(), px(9.0));
        assert_eq!(px(8.49).round(), px(8.0));
    }

    #[test]
    fn round_snap_yields_integer_for_fractional_line_height() {
        // 14 pt × 1.3 multiplier = 18.2 px — matches the default config
        // (DEFAULT_FONT_SIZE × DEFAULT_LINE_HEIGHT in this file).
        let raw_lh = px(18.2);
        let snapped = raw_lh.round();
        assert_eq!(snapped, px(18.0));
        assert!(snapped.as_f32().fract().abs() < 1e-6);
    }
}
