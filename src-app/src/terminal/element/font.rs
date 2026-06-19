//! Font resolution + cell measurement for the terminal renderer.
//!
//! Owns the embedded-font primary contract, the installed-monospace-font
//! registry (cross-platform), and the cached font config read from
//! `paneflow.json`. Exposes `measure_cell` - the sole entry point used by
//! the renderer to turn the current font + size into per-cell pixel
//! dimensions.
//!
//! Extracted from `terminal_element.rs` per US-008 of the src-app refactor PRD.

#[cfg(target_os = "macos")]
use std::collections::HashSet;
#[cfg(target_os = "macos")]
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

/// Embedded monospace family - the default terminal/code font on Linux and
/// macOS, and the cross-platform fallback available on every platform. Files:
/// `assets/fonts/IBMPlexMono-{Regular,SemiBold,Italic,SemiBoldItalic}.ttf`,
/// registered with GPUI at startup (`main.rs` → `Assets::load_fonts` →
/// `cx.text_system().add_fonts`).
///
/// Picking an *embedded* family as the primary instead of a system family
/// (Menlo / Cascadia Mono / DejaVu) sidesteps the failure mode behind commit
/// c3e2331: Core Text inside a signed .app bundle can return valid glyph_ids
/// for a system family while rasterizing them as empty bitmaps, and GPUI's
/// per-`Font` fallback chain only walks on missing-glyph not on empty-raster -
/// so the system primary "renders" zero glyphs and nothing falls through. With
/// an embedded family as primary, GPUI's text system owns the font tables
/// end-to-end and rasterization always works. (Same embedded-primary strategy
/// Zed uses with `.ZedMono` → "Lilex"; Lilex also ships as an opt-in ligature
/// alternate - see `resolve_font_family` - but is no longer the default.)
pub(crate) const EMBEDDED_MONO_FAMILY: &str = "IBM Plex Mono";

/// Windows Terminal's default font. PaneFlow follows the platform convention
/// on Windows while retaining the embedded mono as GPUI's fallback.
#[cfg(target_os = "windows")]
const WINDOWS_DEFAULT_FONT_FAMILY: &str = "Cascadia Mono";

/// Embedded UI/sans family. Files:
/// `assets/fonts/IBMPlexSans-{Regular,SemiBold,Italic,SemiBoldItalic}.ttf`.
/// Mirrors Zed's `.ZedSans` → "IBM Plex Sans" alias. Currently used
/// only in the Paneflow font fallback chain (so missing UI glyphs
/// like e.g. extended Latin diacritics fall through to a known-good
/// embedded font); a future split where the sidebar/tabs use sans
/// while the terminal stays mono would set `.PaneflowSans` as the
/// `ui_font_family` config root.
pub(crate) const EMBEDDED_SANS_FAMILY: &str = "IBM Plex Sans";

/// Paneflow-side virtual font aliases. Mirror Zed's `.ZedMono` /
/// `.ZedSans` pattern from `crates/gpui/src/text_system.rs:1167-1173`,
/// but expanded at the Paneflow boundary (in `resolve_font_family`)
/// before the family name reaches GPUI - GPUI's pinned rev does not
/// know about Paneflow-specific aliases.
///
/// Users can write either the alias (`".PaneflowMono"`) or the
/// concrete name (`"Lilex"`) in `paneflow.json`; both resolve to the
/// same embedded TTF. Defaulting to the alias (rather than the
/// concrete name) lets a future swap of the underlying file -
/// e.g. Lilex → IosevkaTerm - happen with a single edit to this
/// constant table instead of a config migration for every user.
pub(crate) const PANEFLOW_MONO_ALIAS: &str = ".PaneflowMono";
pub(crate) const PANEFLOW_SANS_ALIAS: &str = ".PaneflowSans";

/// Resolve a Paneflow-virtual alias to its concrete embedded family.
/// Returns the input unchanged when it isn't an alias. Pure function,
/// no I/O, used by `resolve_font_family` to expand aliases at the
/// Paneflow boundary before the family name reaches GPUI.
fn expand_paneflow_alias(name: &str) -> &str {
    match name {
        PANEFLOW_MONO_ALIAS => EMBEDDED_MONO_FAMILY,
        PANEFLOW_SANS_ALIAS => EMBEDDED_SANS_FAMILY,
        other => other,
    }
}

// Per-`Font` `fallbacks: Some(...)` was REMOVED on purpose. Paneflow
// previously attached a hardcoded chain (Noto Color Emoji, Symbols
// Nerd Font Mono, embedded sans, embedded mono) that, on macOS, was
// the trigger for the v0.2.12 "boxes drawn, no glyphs" bug:
// `apply_features_and_fallbacks` (gpui_macos/src/open_type.rs:30-73)
// rebuilds every CTFont with a Core Text cascade list assembled from
// `CTFontDescriptorCreateWithNameAndSize` for each fallback name.
// Two entries in the old chain - Noto Color Emoji and Symbols Nerd
// Font Mono - are NOT installed on a fresh macOS, and the resulting
// cascade list, while accepted by Core Text without erroring, ended
// up suppressing rasterization of the primary face. Icons rendered
// (different code path, walking GPUI's internal `fallback_font_stack`
// at gpui/src/text_system.rs:71-83) but text glyphs did not.
//
// Zed's terminal uses `fallbacks: None` by default
// (zed/crates/terminal_view/src/terminal_element.rs:908-912). It only
// wraps `Some(...)` when the user explicitly configures
// `terminal.font_fallbacks` in their settings. Paneflow mirrors that
// pattern: `base_font` emits `Some(FontFallbacks)` ONLY when the user sets
// the top-level `font_fallbacks` array in `paneflow.json` (e.g. a Nerd
// Font for Starship / oh-my-posh / Terminal-Icons glyphs that no Windows
// system font carries), and `None` otherwise - never a hardcoded chain.
//
// Glyph fallback for codepoints Lilex doesn't cover (emoji, CJK,
// symbols) still works: GPUI walks its built-in `fallback_font_stack`
// - which already ships `.ZedMono` (resolves to Lilex, which we
// embed), `.ZedSans` (resolves to IBM Plex Sans, which we embed),
// then OS-canonical sans like Helvetica / Segoe UI / Arial. That
// chain is global, not per-`Font`, so it does NOT pollute the
// per-Font CTFont cascade list.

/// Registry of installed monospace families (Core Text), used ONLY on macOS to
/// validate a configured `font_family` against the documented c3e2331
/// empty-raster failure mode. Populated lazily on first access.
///
/// macOS-only by design: on Linux the equivalent `fc-list :spacing=mono`
/// validation wrongly rejected real monospace fonts that fontconfig didn't tag
/// (patched Nerd Fonts) and forked `fc-list` on the first terminal layout; on
/// Windows the registry was always empty (no enumeration). `resolve_font_family`
/// therefore trusts the configured family on those platforms.
#[cfg(target_os = "macos")]
static INSTALLED_MONO_FONTS: LazyLock<HashSet<String>> =
    LazyLock::new(|| crate::fonts::load_mono_fonts().into_iter().collect());

// ---------------------------------------------------------------------------
// Font config cache - avoids load_config() on every base_font()/font_size() call
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
    /// Sanitized user `font_fallbacks` (trimmed, empties dropped, `None`
    /// when absent/all-empty). Hot-reloaded via the same 500 ms cache as
    /// `family`/`size`/`ligatures`.
    fallbacks: Option<Vec<String>>,
    last_check: std::time::Instant,
}

/// Normalize a configured `font_fallbacks` list before it reaches GPUI:
/// trim each entry, drop empties, and collapse an absent / all-empty list to
/// `None` so [`base_font`] emits `fallbacks: None` (GPUI's built-in stack
/// only) rather than an empty `FontFallbacks`. Pure - unit-tested.
fn sanitize_font_fallbacks(configured: Option<&Vec<String>>) -> Option<Vec<String>> {
    let list: Vec<String> = configured?
        .iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect();
    (!list.is_empty()).then_some(list)
}

static FONT_CONFIG_CACHE: std::sync::Mutex<Option<CachedFontConfig>> = std::sync::Mutex::new(None);

/// The default monospace family PaneFlow uses out of the box.
///
/// Windows uses the system-provided `Cascadia Mono`, matching Windows
/// Terminal. Linux and macOS use the embedded [`EMBEDDED_MONO_FAMILY`].
/// Keeping macOS on an embedded primary avoids the Core Text empty-raster
/// failure documented by commit c3e2331.
///
/// Users can still override with any system font via
/// `paneflow.json#font_family` - `resolve_font_family` validates the
/// override against the installed-mono registry (when populated) and
/// degrades back to this default with a warning otherwise.
pub(crate) fn default_font_family() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        WINDOWS_DEFAULT_FONT_FAMILY
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Registered with GPUI's text system at boot via
        // `Assets::load_fonts`, bypassing OS font-enumeration failures.
        EMBEDDED_MONO_FAMILY
    }
}

pub fn resolve_font_family(configured: Option<&str>) -> String {
    let candidate = configured
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .map(expand_paneflow_alias)
        .unwrap_or(default_font_family());

    // Embedded families are always resolvable: Assets::load_fonts
    // registers them directly with GPUI's text system at boot,
    // bypassing the OS font enumeration registry. Short-circuit before
    // the INSTALLED_MONO_FONTS lookup, which only sees system fonts.
    // "Lilex" is also embedded (alternate mono kept available for
    // users who prefer ligatures); recognised here even though it is
    // no longer the default.
    if candidate == EMBEDDED_MONO_FAMILY
        || candidate == EMBEDDED_SANS_FAMILY
        || candidate == "Lilex"
    {
        return candidate.to_string();
    }

    // The installed-monospace validation guards a macOS-specific Core Text
    // failure mode (a system family that resolves but rasterizes empty - commit
    // c3e2331), so it is gated to macOS. On Linux it wrongly rejected real
    // monospace fonts fontconfig didn't tag `:spacing=mono` (patched Nerd
    // Fonts) AND ran `fc-list` on the first terminal layout; on Windows the
    // registry was always empty. Elsewhere we trust the configured family -
    // GPUI's text system resolves it, and an unresolvable name already falls
    // through to the embedded fallback stack.
    #[cfg(target_os = "macos")]
    if !INSTALLED_MONO_FONTS.is_empty() && !INSTALLED_MONO_FONTS.contains(candidate) {
        let fallback = default_font_family();
        log::warn!(
            "font_family '{candidate}' is not an installed monospace family; using embedded '{fallback}'"
        );
        return fallback.to_string();
    }

    candidate.to_string()
}

/// Read font config, cached for 500ms (same pattern as theme cache).
pub(super) fn cached_font_config() -> (String, f32, f32, bool, Option<Vec<String>>) {
    use std::time::{Duration, Instant};
    const CHECK_INTERVAL: Duration = Duration::from_millis(500);

    let mut cache = FONT_CONFIG_CACHE.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref c) = *cache
        && c.last_check.elapsed() < CHECK_INTERVAL
    {
        return (
            c.family.clone(),
            c.size,
            c.line_height,
            c.ligatures,
            c.fallbacks.clone(),
        );
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

    // User-configured fallback families (Nerd Font for icon glyphs, …),
    // sanitized to `None` when absent/all-empty so `base_font` keeps GPUI's
    // built-in stack in that case.
    let fallbacks = sanitize_font_fallbacks(config.font_fallbacks.as_ref());

    // Diagnostic: log the effective resolved family the first time we
    // populate the cache, and on every subsequent change. This makes it
    // possible to confirm from `RUST_LOG=info` whether the embedded
    // fallback was selected (e.g. on a macOS install where Core Text
    // failed to surface `Menlo` from inside a signed .app bundle) without
    // adding a hot-path log on every render.
    let family_changed = cache.as_ref().is_none_or(|prev| prev.family != family);
    if family_changed {
        log::info!(
            "font: resolved family='{family}' size={size}px line_height={line_height} ligatures={ligatures}"
        );
    }

    *cache = Some(CachedFontConfig {
        family: family.clone(),
        size,
        line_height,
        ligatures,
        fallbacks: fallbacks.clone(),
        last_check: Instant::now(),
    });

    (family, size, line_height, ligatures, fallbacks)
}

pub(super) fn base_font() -> Font {
    let (family, _, _, ligatures, fallbacks) = cached_font_config();
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
        // `None` matches Zed's terminal Font default
        // (zed/crates/terminal_view/src/terminal_element.rs:908-912) and is
        // kept unless the user opts in via the top-level `font_fallbacks`
        // array (already sanitized to non-empty-or-`None` by
        // `cached_font_config`). See the long-form rationale on the removed
        // `FONT_FALLBACKS` static above for why we never hardcode a chain.
        fallbacks: fallbacks.map(FontFallbacks::from_fonts),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// EP-006 US-019: bounds shared by the global config validation, the
/// per-pane zoom steps, and the session-restore ingress.
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 32.0;

/// EP-006 US-019: validate a `font_size` read back from session.json
/// (UNTRUSTED-adjacent: local-only but validated anyway, US-057/EP-010
/// invariant). NaN/±inf are DROPPED (`None` - they would poison the cell
/// geometry); finite out-of-range values are clamped. Pure - unit-tested.
pub fn sanitize_font_override(raw: f32) -> Option<f32> {
    if !raw.is_finite() {
        return None;
    }
    Some(raw.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE))
}

/// Effective terminal font size. `size_override` is EP-006 US-019's
/// per-pane zoom: `Some(px)` wins over the global config; `None` falls
/// back to the cached global (config + 500 ms cache). The override is
/// already clamped to [8.0, 32.0] at every write site (action handler +
/// session ingress), so no re-validation here.
pub(super) fn font_size(size_override: Option<f32>) -> Pixels {
    if let Some(s) = size_override {
        return px(s);
    }
    let (_, size, _, _, _) = cached_font_config();
    px(size)
}

/// EP-006 US-019: the global (non-overridden) font size - the zoom
/// handlers' baseline for a pane that has no override yet.
pub fn global_font_size() -> f32 {
    let (_, size, _, _, _) = cached_font_config();
    size
}

pub fn measure_cell(
    window: &mut Window,
    _cx: &mut App,
    size_override: Option<f32>,
) -> CellDimensions {
    let font = base_font();
    let font_size = font_size(size_override);
    let font_id = window.text_system().resolve_font(&font);

    // DIAGNOSTIC A - fires once per process. Surfaces whether GPUI's
    // `resolve_font` actually loaded the requested family ("Lilex") or
    // silently fell back to the `fallback_font_stack`
    // (gpui/src/text_system.rs:148-160). The Paneflow log line
    // `font: resolved family='Lilex'` reflects only what Paneflow
    // requested as input - it is NOT proof that GPUI returned a
    // FontId pointing at Lilex. If `get_font_for_id` returns a
    // different family, GPUI silently fell through to a system font
    // that may not rasterize correctly inside a signed .app on
    // macOS. Tied to the v0.2.12 "boxes drawn, no glyphs" bug.
    {
        use std::sync::Once;
        static LOG_ONCE: Once = Once::new();
        LOG_ONCE.call_once(|| {
            let resolved = window.text_system().get_font_for_id(font_id);
            match resolved {
                Some(actual) if actual.family == font.family => {
                    log::info!(
                        "font diagnostic: PRIMARY MATCH requested='{}' resolved='{}'",
                        font.family,
                        actual.family,
                    );
                }
                Some(actual) => {
                    log::warn!(
                        "font diagnostic: SILENT FALLBACK requested='{}' resolved='{}' \
                         (GPUI walked fallback_font_stack - primary `font_id` failed)",
                        font.family,
                        actual.family,
                    );
                }
                None => {
                    log::warn!(
                        "font diagnostic: get_font_for_id returned None for resolved \
                         id of requested='{}' (cache mapping anomaly)",
                        font.family,
                    );
                }
            }
        });
    }

    // Raw advance width for 'm' in the resolved font. If the text system
    // can't measure (font load failed, glyph missing, etc.) fall back to
    // `font_size` rather than panic - a slightly-too-wide cell (~1.5-1.7×
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

    // Line height scales with the EFFECTIVE size (override or global) so a
    // zoomed pane keeps its configured line-height ratio.
    let (_, global_size, multiplier, _, _) = cached_font_config();
    let size_f32 = size_override.unwrap_or(global_size);
    let line_height_raw = px(size_f32 * multiplier);

    // US-002: snap raw font measurements to integer pixels via `.round()`
    // (WezTerm convention - minimizes layout-area drift on fractional
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

    // EP-006 US-019: session.json ingress for the per-pane zoom - NaN/inf
    // dropped, finite values clamped to [8.0, 32.0] (PRD AC + test).
    #[test]
    fn sanitize_font_override_drops_non_finite_and_clamps() {
        assert_eq!(sanitize_font_override(f32::NAN), None);
        assert_eq!(sanitize_font_override(f32::INFINITY), None);
        assert_eq!(sanitize_font_override(f32::NEG_INFINITY), None);
        assert_eq!(sanitize_font_override(0.0), Some(MIN_FONT_SIZE));
        assert_eq!(sanitize_font_override(-5.0), Some(MIN_FONT_SIZE));
        assert_eq!(sanitize_font_override(1000.0), Some(MAX_FONT_SIZE));
        assert_eq!(sanitize_font_override(14.0), Some(14.0));
        assert_eq!(sanitize_font_override(8.0), Some(8.0));
        assert_eq!(sanitize_font_override(32.0), Some(32.0));
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
        // 14 pt × 1.3 multiplier = 18.2 px - matches the default config
        // (DEFAULT_FONT_SIZE × DEFAULT_LINE_HEIGHT in this file).
        let raw_lh = px(18.2);
        let snapped = raw_lh.round();
        assert_eq!(snapped, px(18.0));
        assert!(snapped.as_f32().fract().abs() < 1e-6);
    }

    // ─── Paneflow virtual-alias resolution ────────────────────────────
    // Lock in the contract that `.PaneflowMono` and `.PaneflowSans`
    // resolve to the embedded family names BEFORE leaving Paneflow.
    // GPUI's pinned rev does not know these aliases - a regression
    // here would surface as "embedded font registered but never
    // selected because GPUI sees the literal alias string".

    #[test]
    fn expand_paneflow_alias_resolves_mono_alias() {
        assert_eq!(expand_paneflow_alias(".PaneflowMono"), EMBEDDED_MONO_FAMILY);
        assert_eq!(expand_paneflow_alias(".PaneflowMono"), "IBM Plex Mono");
    }

    #[test]
    fn expand_paneflow_alias_resolves_sans_alias() {
        assert_eq!(expand_paneflow_alias(".PaneflowSans"), EMBEDDED_SANS_FAMILY);
        assert_eq!(expand_paneflow_alias(".PaneflowSans"), "IBM Plex Sans");
    }

    #[test]
    fn expand_paneflow_alias_passes_concrete_names_through() {
        // System fonts and any non-alias string round-trip unchanged.
        // Critical for `resolve_font_family` correctness: the alias
        // expansion must not eat user-configured system fonts.
        assert_eq!(expand_paneflow_alias("Menlo"), "Menlo");
        assert_eq!(expand_paneflow_alias("Cascadia Mono"), "Cascadia Mono");
        assert_eq!(expand_paneflow_alias("Lilex"), "Lilex");
        assert_eq!(expand_paneflow_alias(""), "");
        // Case-sensitive: `.paneflowmono` is not `.PaneflowMono`.
        assert_eq!(expand_paneflow_alias(".paneflowmono"), ".paneflowmono");
    }

    #[test]
    fn resolve_font_family_default_returns_platform_default() {
        assert_eq!(resolve_font_family(None), default_font_family());
        assert_eq!(resolve_font_family(Some("")), default_font_family());
        assert_eq!(resolve_font_family(Some("   ")), default_font_family());
    }

    #[test]
    fn resolve_font_family_expands_paneflow_aliases() {
        // Both aliases must resolve through to their embedded targets
        // - the value GPUI's `text_system().resolve_font` will look
        // up against the registered TTFs.
        assert_eq!(resolve_font_family(Some(".PaneflowMono")), "IBM Plex Mono");
        assert_eq!(resolve_font_family(Some(".PaneflowSans")), "IBM Plex Sans");
    }

    #[test]
    fn resolve_font_family_short_circuits_embedded_concrete_names() {
        // Users who write `"Lilex"` or `"IBM Plex Sans"` in
        // paneflow.json get the embedded font even on platforms whose
        // INSTALLED_MONO_FONTS registry doesn't list them (Windows
        // pre-DirectWrite, container without fontconfig). The short
        // circuit before the registry lookup is what makes that work.
        assert_eq!(resolve_font_family(Some("Lilex")), "Lilex");
        assert_eq!(resolve_font_family(Some("IBM Plex Sans")), "IBM Plex Sans");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn default_font_family_is_embedded_mono_on_linux_and_macos() {
        assert_eq!(default_font_family(), EMBEDDED_MONO_FAMILY);
        assert_eq!(default_font_family(), "IBM Plex Mono");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn default_font_family_matches_windows_terminal() {
        assert_eq!(default_font_family(), WINDOWS_DEFAULT_FONT_FAMILY);
        assert_eq!(default_font_family(), "Cascadia Mono");
    }

    // ─── font_fallbacks sanitization ─────────────────────────────────
    // The wiring that lets a user keep IBM Plex Mono primary while adding
    // a Nerd Font fallback for Starship / oh-my-posh icons. The sanitizer
    // must collapse absent/all-empty lists to `None` so `base_font` emits
    // `fallbacks: None` (GPUI's built-in stack) rather than an empty
    // `FontFallbacks`, and must trim + drop blank entries.

    #[test]
    fn sanitize_font_fallbacks_absent_is_none() {
        assert_eq!(sanitize_font_fallbacks(None), None);
    }

    #[test]
    fn sanitize_font_fallbacks_empty_list_is_none() {
        assert_eq!(sanitize_font_fallbacks(Some(&vec![])), None);
    }

    #[test]
    fn sanitize_font_fallbacks_all_blank_is_none() {
        let cfg = vec!["".to_string(), "   ".to_string(), "\t".to_string()];
        assert_eq!(sanitize_font_fallbacks(Some(&cfg)), None);
    }

    #[test]
    fn sanitize_font_fallbacks_trims_and_drops_blanks() {
        let cfg = vec![
            "  FiraCode Nerd Font Mono  ".to_string(),
            "".to_string(),
            "Segoe UI Emoji".to_string(),
        ];
        assert_eq!(
            sanitize_font_fallbacks(Some(&cfg)),
            Some(vec![
                "FiraCode Nerd Font Mono".to_string(),
                "Segoe UI Emoji".to_string(),
            ])
        );
    }

    #[test]
    fn sanitize_font_fallbacks_preserves_order() {
        // Fallback order is significant - GPUI consults entries in order,
        // so the sanitizer must never reorder or dedupe.
        let cfg = vec!["B".to_string(), "A".to_string(), "B".to_string()];
        assert_eq!(
            sanitize_font_fallbacks(Some(&cfg)),
            Some(vec!["B".to_string(), "A".to_string(), "B".to_string()])
        );
    }
}
