//! Terminal cell renderer using GPUI's Element trait.
//!
//! Renders terminal cells from alacritty_terminal as batched text runs with
//! full ANSI color support, cell attributes, and background quads.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Element, ElementId, Font, FontFallbacks, FontStyle,
    FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point,
    Rgba, SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window,
    fill, outline, px, relative,
};

use crate::terminal::{PtyNotifier, SpikeTermSize, ZedListener};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_LINE_HEIGHT: f32 = 1.3;

const FONT_FALLBACK_EMOJI: &str = "Noto Color Emoji";
const FONT_FALLBACK_SYMBOLS: &str = "Symbols Nerd Font Mono";
const FONT_FALLBACK_SANS: &str = "Noto Sans";

/// APCA minimum Lc (lightness contrast) threshold.
/// Lc 45 is "minimum for large fluent text" per ARC Bronze Simple Mode — matches Zed's default.
/// APCA is more accurate than WCAG 2.0 on dark backgrounds (polarity-aware, perceptually uniform).
const MIN_APCA_CONTRAST: f32 = 45.0;

static FONT_FALLBACKS: LazyLock<FontFallbacks> = LazyLock::new(|| {
    FontFallbacks::from_fonts(vec![
        FONT_FALLBACK_EMOJI.to_string(),
        FONT_FALLBACK_SYMBOLS.to_string(),
        FONT_FALLBACK_SANS.to_string(),
    ])
});

#[cfg(target_os = "linux")]
static INSTALLED_MONO_FONTS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    crate::config_writer::load_mono_fonts()
        .into_iter()
        .collect()
});

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

pub(crate) fn resolve_font_family(configured: Option<&str>) -> String {
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
fn cached_font_config() -> (String, f32, f32) {
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

// ---------------------------------------------------------------------------
// Minimum contrast (APCA — Accessible Perceptual Contrast Algorithm)
// ---------------------------------------------------------------------------

/// APCA constants (0.0.98G-4g W3 compatible).
/// https://github.com/Myndex/apca-w3
struct ApcaConstants {
    main_trc: f32,
    s_rco: f32,
    s_gco: f32,
    s_bco: f32,
    norm_bg: f32,
    norm_txt: f32,
    rev_txt: f32,
    rev_bg: f32,
    blk_thrs: f32,
    blk_clmp: f32,
    scale_bow: f32,
    scale_wob: f32,
    lo_bow_offset: f32,
    lo_wob_offset: f32,
    delta_y_min: f32,
    lo_clip: f32,
}

const APCA: ApcaConstants = ApcaConstants {
    main_trc: 2.4,
    s_rco: 0.2126729,
    s_gco: 0.7151522,
    s_bco: 0.0721750,
    norm_bg: 0.56,
    norm_txt: 0.57,
    rev_txt: 0.62,
    rev_bg: 0.65,
    blk_thrs: 0.022,
    blk_clmp: 1.414,
    scale_bow: 1.14,
    scale_wob: 1.14,
    lo_bow_offset: 0.027,
    lo_wob_offset: 0.027,
    delta_y_min: 0.0005,
    lo_clip: 0.1,
};

fn srgb_to_y(color: Hsla) -> f32 {
    let rgba = Rgba::from(color);
    let r_linear = rgba.r.powf(APCA.main_trc);
    let g_linear = rgba.g.powf(APCA.main_trc);
    let b_linear = rgba.b.powf(APCA.main_trc);
    APCA.s_rco * r_linear + APCA.s_gco * g_linear + APCA.s_bco * b_linear
}

fn apca_contrast(text: Hsla, bg: Hsla) -> f32 {
    let text_y = srgb_to_y(text);
    let bg_y = srgb_to_y(bg);

    let text_y = if text_y > APCA.blk_thrs {
        text_y
    } else {
        text_y + (APCA.blk_thrs - text_y).powf(APCA.blk_clmp)
    };
    let bg_y = if bg_y > APCA.blk_thrs {
        bg_y
    } else {
        bg_y + (APCA.blk_thrs - bg_y).powf(APCA.blk_clmp)
    };

    if (bg_y - text_y).abs() < APCA.delta_y_min {
        return 0.0;
    }

    let (sapc, offset) = if bg_y > text_y {
        let s = (bg_y.powf(APCA.norm_bg) - text_y.powf(APCA.norm_txt)) * APCA.scale_bow;
        (s, APCA.lo_bow_offset)
    } else {
        let s = (bg_y.powf(APCA.rev_bg) - text_y.powf(APCA.rev_txt)) * APCA.scale_wob;
        (s, -APCA.lo_wob_offset)
    };

    if sapc.abs() < APCA.lo_clip {
        0.0
    } else {
        (sapc - offset) * 100.0
    }
}

/// Adjust `fg` lightness using APCA so that perceptual contrast against `bg`
/// meets `min_lc`. Returns `fg` unchanged if contrast is already sufficient.
///
/// Three-stage fallback matching Zed's approach:
/// 1. Adjust lightness only (preserves hue + saturation)
/// 2. Reduce saturation + adjust lightness
/// 3. Fall back to black or white
fn ensure_minimum_contrast(fg: Hsla, bg: Hsla, min_lc: f32) -> Hsla {
    if min_lc <= 0.0 {
        return fg;
    }

    if apca_contrast(fg, bg).abs() >= min_lc {
        return fg;
    }

    // Stage 1: adjust lightness only
    let adjusted = adjust_lightness_for_apca(fg, bg, min_lc);
    if apca_contrast(adjusted, bg).abs() >= min_lc {
        return adjusted;
    }

    // Stage 2: reduce saturation + adjust lightness
    for &sat_mult in &[0.8, 0.6, 0.4, 0.2, 0.0] {
        let desat = Hsla { s: fg.s * sat_mult, ..fg };
        let adjusted = adjust_lightness_for_apca(desat, bg, min_lc);
        if apca_contrast(adjusted, bg).abs() >= min_lc {
            return adjusted;
        }
    }

    // Stage 3: black or white
    let black = Hsla { h: 0.0, s: 0.0, l: 0.0, a: fg.a };
    let white = Hsla { h: 0.0, s: 0.0, l: 1.0, a: fg.a };
    if apca_contrast(white, bg).abs() > apca_contrast(black, bg).abs() {
        white
    } else {
        black
    }
}

fn adjust_lightness_for_apca(fg: Hsla, bg: Hsla, min_lc: f32) -> Hsla {
    let bg_lum = srgb_to_y(bg);
    let should_darken = bg_lum > 0.5;

    let (mut lo, mut hi) = if should_darken {
        (0.0, fg.l)
    } else {
        (fg.l, 1.0)
    };
    let mut best_l = fg.l;

    for _ in 0..20 {
        let mid = (lo + hi) * 0.5;
        let test = Hsla { l: mid, ..fg };
        let contrast = apca_contrast(test, bg).abs();

        if contrast >= min_lc {
            best_l = mid;
            if should_darken { lo = mid; } else { hi = mid; }
        } else if should_darken {
            hi = mid;
        } else {
            lo = mid;
        }

        if (contrast - min_lc).abs() < 1.0 {
            best_l = mid;
            break;
        }
    }

    Hsla { l: best_l, ..fg }
}

/// Returns `true` for characters whose colors should be preserved exactly
/// (no contrast adjustment). Covers box-drawing, block elements, geometric
/// shapes, and Powerline separator symbols.
fn is_decorative_character(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2500..=0x257F   // Box Drawing (─ │ ┌ ┐ └ ┘ etc.)
        | 0x2580..=0x259F // Block Elements (▀ ▄ █ ░ ▒ ▓ etc.)
        | 0x25A0..=0x25FF // Geometric Shapes (■ ▶ ● etc.)
        | 0xE0B0..=0xE0B7 // Powerline: right/left arrows
        | 0xE0B8..=0xE0BF // Powerline: bottom/top triangles
        | 0xE0C0..=0xE0CA // Powerline: flame, pixel separators
        | 0xE0CC..=0xE0D1 // Powerline: waveform, hex (excludes 0xE0CB)
        | 0xE0D2..=0xE0D7 // Powerline: trapezoids, inverted triangles
    )
}

/// Returns `true` for Powerline separator glyphs and box-drawing characters
/// whose intentional color transitions at cell edges should not be extended
/// into the terminal padding (neverExtendBg heuristic, US-003).
fn is_powerline_or_boxdraw(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2500..=0x257F   // Box Drawing (─ │ ┌ ┐ └ ┘ etc.)
        | 0x2580..=0x259F // Block Elements (▀ ▄ █ ░ ▒ ▓ — half-block color transitions)
        | 0xE0B0..=0xE0B7 // Powerline: right/left arrows
        | 0xE0B8..=0xE0BF // Powerline: bottom/top triangles
        | 0xE0C0..=0xE0CA // Powerline: flame, pixel separators
        | 0xE0CC..=0xE0D1 // Powerline: waveform, hex (excludes 0xE0CB)
        | 0xE0D2..=0xE0D7 // Powerline: trapezoids, inverted triangles
    )
}

/// Merge vertically adjacent background rects that share the same column span
/// and color, reducing the number of paint_quad() calls. The input rects are
/// already horizontally merged (same-row, same-color, contiguous columns).
fn merge_background_regions(mut rects: Vec<LayoutRect>) -> Vec<LayoutRect> {
    if rects.len() <= 1 {
        return rects;
    }
    // Sort by (col, num_cols, color bits, line) so vertically adjacent candidates
    // are consecutive in the list.
    rects.sort_unstable_by(|a, b| {
        a.col
            .cmp(&b.col)
            .then(a.num_cols.cmp(&b.num_cols))
            .then(a.color.h.total_cmp(&b.color.h))
            .then(a.color.s.total_cmp(&b.color.s))
            .then(a.color.l.total_cmp(&b.color.l))
            .then(a.color.a.total_cmp(&b.color.a))
            .then(a.line.cmp(&b.line))
    });

    let mut merged: Vec<LayoutRect> = Vec::with_capacity(rects.len());
    let mut iter = rects.into_iter();
    let mut current = iter.next().unwrap();

    for next in iter {
        if next.col == current.col
            && next.num_cols == current.num_cols
            && next.color == current.color
            && next.line == current.line + current.num_lines as i32
        {
            current.num_lines += next.num_lines;
        } else {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);
    merged
}

// ---------------------------------------------------------------------------
// Layout types
// ---------------------------------------------------------------------------

pub struct CellDimensions {
    pub cell_width: Pixels,
    pub line_height: Pixels,
}

struct BatchedTextRun {
    text: String,
    font: Font,
    color: Hsla,
    underline: Option<UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
    line: i32,
    col_start: usize,
}

struct LayoutRect {
    line: i32,
    num_lines: usize,
    col: usize,
    num_cols: usize,
    color: Hsla,
}

/// A block/half-block character rendered as a filled quad instead of a font glyph.
/// This eliminates subpixel gaps between adjacent block elements in pixel art (logos, etc.).
struct BlockQuad {
    line: i32,
    col: usize,
    num_cols: usize, // 2 for wide chars
    color: Hsla,
    /// Fractional coverage of the cell: (x_start, y_start, width, height) in 0.0..1.0
    coverage: (f32, f32, f32, f32),
}

/// If `c` is a Unicode block element, return its fractional cell coverage (x, y, w, h).
/// Returns None for characters that should be rendered as normal glyphs.
fn block_char_coverage(c: char) -> Option<(f32, f32, f32, f32)> {
    match c {
        '▀' => Some((0.0, 0.0, 1.0, 0.5)),  // U+2580 Upper half
        '▁' => Some((0.0, 7.0/8.0, 1.0, 1.0/8.0)), // U+2581 Lower 1/8
        '▂' => Some((0.0, 6.0/8.0, 1.0, 2.0/8.0)), // U+2582 Lower 1/4
        '▃' => Some((0.0, 5.0/8.0, 1.0, 3.0/8.0)), // U+2583 Lower 3/8
        '▄' => Some((0.0, 0.5, 1.0, 0.5)),   // U+2584 Lower half
        '▅' => Some((0.0, 3.0/8.0, 1.0, 5.0/8.0)), // U+2585 Lower 5/8
        '▆' => Some((0.0, 2.0/8.0, 1.0, 6.0/8.0)), // U+2586 Lower 3/4
        '▇' => Some((0.0, 1.0/8.0, 1.0, 7.0/8.0)), // U+2587 Lower 7/8
        '█' => Some((0.0, 0.0, 1.0, 1.0)),   // U+2588 Full block
        '▉' => Some((0.0, 0.0, 7.0/8.0, 1.0)), // U+2589 Left 7/8
        '▊' => Some((0.0, 0.0, 6.0/8.0, 1.0)), // U+258A Left 3/4
        '▋' => Some((0.0, 0.0, 5.0/8.0, 1.0)), // U+258B Left 5/8
        '▌' => Some((0.0, 0.0, 0.5, 1.0)),   // U+258C Left half
        '▍' => Some((0.0, 0.0, 3.0/8.0, 1.0)), // U+258D Left 3/8
        '▎' => Some((0.0, 0.0, 2.0/8.0, 1.0)), // U+258E Left 1/4
        '▏' => Some((0.0, 0.0, 1.0/8.0, 1.0)), // U+258F Left 1/8
        '▐' => Some((0.5, 0.0, 0.5, 1.0)),   // U+2590 Right half
        _ => None,
    }
}

struct CursorInfo {
    line: i32,
    col: usize,
    shape: CursorShape,
    color: Hsla,
    wide: bool,
    /// Character under the cursor (None for whitespace or non-Block shapes).
    text: Option<char>,
    bold: bool,
    italic: bool,
}

/// A search match highlight to be painted by TerminalElement.
pub struct SearchHighlight {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
    pub is_active: bool,
}

/// Where a hyperlink was detected.
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum HyperlinkSource {
    /// Explicit OSC 8 escape sequence from the program.
    Osc8,
    /// Regex pattern match on terminal output.
    Regex,
}

/// A detected OSC 8 hyperlink zone spanning one or more cells.
/// Fields are populated here (US-014) and consumed by hover/click (US-015/US-016).
#[allow(dead_code)]
pub struct HyperlinkZone {
    pub uri: String,
    pub id: String,
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
    /// Whether this URL's scheme is in the openable allowlist.
    pub is_openable: bool,
    /// How this hyperlink was detected (OSC 8 takes priority over regex).
    pub source: HyperlinkSource,
}

/// URL regex pattern matching Zed's terminal_hyperlinks.rs.
/// Excludes C0/C1 control chars, whitespace, angle brackets, quotes, and other
/// non-URL characters. Box-drawing chars (U+2500-U+257F) are not valid URL
/// characters and won't match the allowed character class.
#[allow(dead_code)]
const URL_REGEX_PATTERN: &str = r#"(mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\x00-\x1f\x7f-\x9f<>"\s{}\^⟨⟩`']+"#;

/// Lazily compiled URL regex (compiled once, reused across all calls).
#[allow(dead_code)]
fn url_regex() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(URL_REGEX_PATTERN).expect("URL regex compilation failed"))
}

/// Detect URLs on a single terminal line via regex with char-to-column mapping.
/// `char_to_col` maps each character index in `line_text` to its grid column,
/// accounting for wide-char spacers that were skipped during text extraction.
#[allow(dead_code)]
pub fn detect_urls_on_line_mapped(
    line_text: &str,
    line: alacritty_terminal::index::Line,
    char_to_col: &[usize],
) -> Vec<HyperlinkZone> {
    let re = url_regex();
    re.find_iter(line_text)
        .filter_map(|m| {
            // Convert byte offsets to char indices for column lookup
            let char_start = line_text[..m.start()].chars().count();
            let char_end = line_text[..m.end()].chars().count().saturating_sub(1);
            let col_start = char_to_col.get(char_start)?;
            let col_end = char_to_col.get(char_end)?;
            let uri = m.as_str().to_string();
            let is_openable = is_url_scheme_openable(&uri);
            Some(HyperlinkZone {
                uri,
                id: String::new(),
                start: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_start),
                ),
                end: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_end),
                ),
                is_openable,
                source: HyperlinkSource::Regex,
            })
        })
        .collect()
}

/// Check if a URL scheme is in the allowlist for opening.
/// Allowed: http, https, mailto, file (with localhost/empty host validation).
pub fn is_url_scheme_openable(uri: &str) -> bool {
    if uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("mailto:") {
        return true;
    }
    if let Some(rest) = uri.strip_prefix("file://") {
        // file:// must have empty host or localhost
        return rest.starts_with('/')
            || rest.starts_with("localhost/")
            || rest.starts_with("localhost:");
    }
    false
}

pub struct LayoutState {
    batched_runs: Vec<BatchedTextRun>,
    rects: Vec<LayoutRect>,
    block_quads: Vec<BlockQuad>,
    selection_rects: Vec<LayoutRect>,
    search_rects: Vec<LayoutRect>,
    cursor: Option<CursorInfo>,
    dimensions: CellDimensions,
    background_color: Hsla,
    scrollbar_thumb: Hsla,
    exited: Option<i32>,
    /// Scroll position for scrollbar indicator (0 = at bottom)
    display_offset: usize,
    /// Total scrollback history size
    history_size: usize,
    /// Number of columns in the terminal grid (for padding extension)
    desired_cols: usize,
    /// Number of rows in the terminal grid (for padding extension)
    desired_rows: usize,
    /// Per-line (extend_left, extend_right) flags for neverExtendBg heuristic.
    /// Indexed by visible line number (0..desired_rows).
    extend_line_flags: Vec<(bool, bool)>,
    /// OSC 8 hyperlink zones detected during cell iteration.
    #[allow(dead_code)]
    hyperlinks: Vec<HyperlinkZone>,
    /// Theme color for hyperlink underline and tooltip text.
    link_text_color: Hsla,
    /// Cursor position bounds for IME popup positioning (pixel coordinates).
    ime_cursor_bounds: Option<Bounds<Pixels>>,
}

// ---------------------------------------------------------------------------
// Cell style — used for batching comparison
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
struct CellStyle {
    font: Font,
    fg: Hsla,
    bg: Hsla,
    underline: bool,
    undercurl: bool,
    strikethrough: bool,
}

// ---------------------------------------------------------------------------
// TerminalElement
// ---------------------------------------------------------------------------

/// Copy mode cursor state for rendering.
pub struct CopyModeCursorState {
    /// Grid-coordinate line of the copy cursor
    pub grid_line: i32,
    /// Column of the copy cursor
    pub col: usize,
}

pub struct TerminalElement {
    term: Arc<FairMutex<Term<ZedListener>>>,
    notifier: PtyNotifier,
    cursor_visible: bool,
    focused: bool,
    exited: Option<i32>,
    /// Shared origin — updated in paint() so mouse handlers know the element position.
    element_origin: Arc<Mutex<Point<Pixels>>>,
    /// Search match highlights to paint
    search_highlights: Vec<SearchHighlight>,
    /// Copy mode cursor position (grid coordinates), if copy mode is active
    copy_mode_cursor: Option<CopyModeCursorState>,
    /// Whether a bell flash is currently active (200ms visual pulse).
    bell_flash_active: bool,
    /// Ctrl+hovered hyperlink range for underline rendering (line, start_col, end_col).
    hovered_link_range: Option<(alacritty_terminal::index::Line, usize, usize)>,
    /// Full URI of the Ctrl+hovered link (for tooltip display).
    hovered_link_uri: Option<String>,
    /// IME preedit text to render at cursor position.
    ime_marked_text: String,
    /// Focus handle for IME input handler registration.
    focus_handle: gpui::FocusHandle,
    /// Terminal view entity for IME callbacks.
    terminal_view: gpui::Entity<crate::terminal::TerminalView>,
    /// Timestamp of the keystroke that triggered this render, for latency measurement.
    #[cfg(debug_assertions)]
    last_keystroke_at: Option<std::time::Instant>,
}

impl TerminalElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        term: Arc<FairMutex<Term<ZedListener>>>,
        notifier: PtyNotifier,
        cursor_visible: bool,
        focused: bool,
        exited: Option<i32>,
        element_origin: Arc<Mutex<Point<Pixels>>>,
        search_highlights: Vec<SearchHighlight>,
        copy_mode_cursor: Option<CopyModeCursorState>,
        bell_flash_active: bool,
        hovered_link_range: Option<(alacritty_terminal::index::Line, usize, usize)>,
        hovered_link_uri: Option<String>,
        ime_marked_text: String,
        focus_handle: gpui::FocusHandle,
        terminal_view: gpui::Entity<crate::terminal::TerminalView>,
        #[cfg(debug_assertions)] last_keystroke_at: Option<std::time::Instant>,
    ) -> Self {
        Self {
            term,
            notifier,
            cursor_visible,
            focused,
            exited,
            element_origin,
            search_highlights,
            copy_mode_cursor,
            bell_flash_active,
            hovered_link_range,
            hovered_link_uri,
            ime_marked_text,
            focus_handle,
            terminal_view,
            #[cfg(debug_assertions)]
            last_keystroke_at,
        }
    }

    fn base_font() -> Font {
        let (family, _, _) = cached_font_config();
        Font {
            family: SharedString::from(family),
            features: gpui::FontFeatures::disable_ligatures(),
            fallbacks: Some(FONT_FALLBACKS.clone()),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    fn font_size() -> Pixels {
        let (_, size, _) = cached_font_config();
        px(size)
    }

    pub fn measure_cell(window: &mut Window, _cx: &mut App) -> CellDimensions {
        let font = Self::base_font();
        let font_size = Self::font_size();
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

    fn build_layout(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutState {
        let dims = Self::measure_cell(window, cx);
        let theme = crate::theme::active_theme();
        let background_color = theme.background;
        let ansi_background = theme.ansi_background;

        // Compute desired terminal grid size from pixel bounds (accounting for left gutter)
        let gutter = dims.cell_width;
        let available_width = (bounds.size.width - gutter).max(px(0.0));
        let desired_cols = (available_width / dims.cell_width).floor().max(1.0) as usize;
        let desired_rows = (bounds.size.height / dims.line_height).floor() as usize;

        // Snapshot the grid, cursor, and selection under lock to minimize FairMutex hold time.
        let cursor_color = theme.cursor;
        let selection_color = theme.selection;

        let (cells, cursor_snapshot, selection_range, display_offset, history_size): (
            Vec<_>,
            Option<CursorInfo>,
            Option<SelectionRange>,
            usize,
            usize,
        ) = {
            let mut term = self.term.lock();
            // Resize the terminal grid if bounds have changed
            let current_cols = term.columns();
            let current_rows = term.screen_lines();
            if desired_cols > 0
                && desired_rows > 0
                && (current_cols != desired_cols || current_rows != desired_rows)
            {
                term.resize(SpikeTermSize {
                    columns: desired_cols,
                    screen_lines: desired_rows,
                });
                // Notify PTY EventLoop to send SIGWINCH to the child process
                let _ = self.notifier.0.send(Msg::Resize(WindowSize {
                    num_cols: desired_cols as u16,
                    num_lines: desired_rows as u16,
                    cell_width: dims.cell_width.as_f32() as u16,
                    cell_height: dims.line_height.as_f32() as u16,
                }));
            }
            let content = term.renderable_content();
            let sel_range = content.selection.as_ref().map(|sel| SelectionRange {
                start: sel.start,
                end: sel.end,
                is_block: sel.is_block,
            });
            let cursor =
                if matches!(content.cursor.shape, CursorShape::Hidden) || !self.cursor_visible {
                    None
                } else {
                    let shape = if !self.focused {
                        CursorShape::HollowBlock
                    } else {
                        content.cursor.shape
                    };
                    let cursor_cell = &term.grid()[content.cursor.point];
                    let wide = cursor_cell.flags.contains(CellFlags::WIDE_CHAR);
                    let cursor_char = cursor_cell.c;
                    let cursor_flags = cursor_cell.flags;
                    let text = if matches!(shape, CursorShape::Block)
                        && cursor_char != ' '
                        && cursor_char != '\0'
                    {
                        Some(cursor_char)
                    } else {
                        None
                    };
                    Some(CursorInfo {
                        line: content.cursor.point.line.0,
                        col: content.cursor.point.column.0,
                        shape,
                        color: cursor_color,
                        wide,
                        text,
                        bold: cursor_flags.contains(CellFlags::BOLD)
                            || cursor_flags.contains(CellFlags::BOLD_ITALIC),
                        italic: cursor_flags.contains(CellFlags::ITALIC)
                            || cursor_flags.contains(CellFlags::BOLD_ITALIC),
                    })
                };
            let disp_offset = content.display_offset;
            let hist_size = term.history_size();
            let cells: Vec<_> = content
                .display_iter
                .map(|ic| {
                    let zw = ic.cell.zerowidth().map(|chars| chars.to_vec());
                    let hyperlink = ic.cell.hyperlink();
                    (
                        ic.point,
                        ic.cell.c,
                        ic.cell.fg,
                        ic.cell.bg,
                        ic.cell.flags,
                        zw,
                        hyperlink,
                    )
                })
                .collect();
            (cells, cursor, sel_range, disp_offset, hist_size)
        };

        // Override cursor with copy mode cursor when active
        let cursor_snapshot = if let Some(ref cm) = self.copy_mode_cursor {
            let display_line = cm.grid_line + display_offset as i32;
            if display_line >= 0 && display_line < desired_rows as i32 {
                // Copy mode cursor: always a solid block with distinct color
                let copy_cursor_color = Hsla {
                    h: 0.5,
                    s: 0.8,
                    l: 0.65,
                    a: 0.9,
                }; // Bright cyan
                Some(CursorInfo {
                    line: display_line,
                    col: cm.col,
                    shape: CursorShape::Block,
                    color: copy_cursor_color,
                    wide: false,
                    text: None,
                    bold: false,
                    italic: false,
                })
            } else {
                None
            }
        } else {
            cursor_snapshot
        };

        let mut batch = BatchAccumulator::new();
        let mut rects: Vec<LayoutRect> = Vec::new();
        let mut block_quads: Vec<BlockQuad> = Vec::new();
        let mut current_rect: Option<LayoutRect> = None;
        let mut last_line: i32 = i32::MIN;
        let mut previous_cell_had_extras = false;

        // neverExtendBg heuristic (US-003): per-line flags for padding extension.
        // Tracks whether Powerline/box-drawing glyphs at edges suppress extension.
        let mut extend_line_flags: Vec<(bool, bool)> = vec![(true, true); desired_rows];
        let mut last_nondefault_is_powerline = false;

        // Viewport culling: compute visible row range from content mask.
        // Rows outside the visible clip rect are skipped during cell processing.
        let content_mask = window.content_mask();
        let visible_top = content_mask.bounds.origin.y;
        let visible_bottom = visible_top + content_mask.bounds.size.height;
        let first_visible_row = ((visible_top - bounds.origin.y) / dims.line_height)
            .floor()
            .max(0.0) as i32;
        let last_visible_row = ((visible_bottom - bounds.origin.y) / dims.line_height)
            .ceil()
            .max(0.0) as i32;

        // OSC 8 hyperlink zone accumulation
        // Merge key is (id, uri) — empty IDs are common, so URI must also match.
        // Zones are split at line boundaries for correct hit-test rectangles.
        let mut hyperlinks: Vec<HyperlinkZone> = Vec::new();

        for (point, c, cell_fg, cell_bg, flags, zw, hyperlink) in &cells {
            let point = *point;
            let flags = *flags;

            // Accumulate OSC 8 hyperlink zones (before culling so offscreen links are tracked)
            if let Some(hl) = hyperlink {
                let can_extend = hyperlinks.last().is_some_and(|zone| {
                    zone.id == hl.id() && zone.uri == hl.uri() && zone.end.line == point.line
                });
                if can_extend {
                    hyperlinks.last_mut().unwrap().end = point;
                } else {
                    let uri = hl.uri().to_string();
                    let is_openable = is_url_scheme_openable(&uri);
                    hyperlinks.push(HyperlinkZone {
                        uri,
                        id: hl.id().to_string(),
                        start: point,
                        end: point,
                        is_openable,
                        source: HyperlinkSource::Osc8,
                    });
                }
            }

            // Viewport culling: skip rendering for rows outside the visible content mask.
            // Hyperlink accumulation above is preserved so cross-line links work at boundaries.
            if point.line.0 < first_visible_row || point.line.0 >= last_visible_row {
                continue;
            }

            // Skip wide char spacers (trailing cell of CJK chars)
            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            // Line change → flush batch and rect, finalize previous line's right flag
            if point.line.0 != last_line {
                batch.flush();
                if let Some(rect) = current_rect.take() {
                    rects.push(rect);
                }
                // Finalize right-extension flag for the previous line
                if last_line >= 0
                    && (last_line as usize) < desired_rows
                    && last_nondefault_is_powerline
                {
                    extend_line_flags[last_line as usize].1 = false;
                }
                last_line = point.line.0;
                last_nondefault_is_powerline = false;
            }

            // Compute colors — INVERSE swap on raw ANSI tags, then tag-based
            // default-background skip (Zed parity: structural check, not HSLA compare).
            let (raw_fg, raw_bg) = if flags.contains(CellFlags::INVERSE) {
                (*cell_bg, *cell_fg)
            } else {
                (*cell_fg, *cell_bg)
            };
            let is_default_bg =
                matches!(raw_bg, AnsiColor::Named(NamedColor::Background));

            let mut fg = convert_color(raw_fg, &theme);
            let bg = convert_color(raw_bg, &theme);

            // neverExtendBg: track Powerline/box-drawing at edges (US-003)
            let line_idx = point.line.0;
            if line_idx >= 0 && (line_idx as usize) < desired_rows {
                // Left suppression: column 0 has a non-default bg AND a Powerline/box-drawing char
                if point.column.0 == 0 && !is_default_bg && is_powerline_or_boxdraw(*c) {
                    extend_line_flags[line_idx as usize].0 = false;
                }
                // Right suppression: track if current non-default-bg cell is Powerline
                if !is_default_bg {
                    last_nondefault_is_powerline = is_powerline_or_boxdraw(*c);
                }
            }

            // DIM/faint (SGR 2): reduce foreground opacity (applied after INVERSE)
            if flags.contains(CellFlags::DIM) {
                fg.a *= 0.7;
            }

            // Enforce minimum foreground/background contrast (skip decorative chars)
            if !is_decorative_character(*c) {
                fg = ensure_minimum_contrast(fg, bg, MIN_APCA_CONTRAST);
            }

            // Background rect — paint for ALL cells. Default-bg cells use
            // ansi_background (the theme's actual background) to contrast with the
            // slightly darker widget fill, creating visible depth for TUI content.
            let cell_cols = if flags.contains(CellFlags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let cell_bg_color = if is_default_bg {
                ansi_background
            } else {
                bg
            };
            match &mut current_rect {
                Some(rect)
                    if rect.line == point.line.0
                        && rect.color == cell_bg_color
                        && rect.col + rect.num_cols == point.column.0 =>
                {
                    rect.num_cols += cell_cols;
                }
                _ => {
                    if let Some(rect) = current_rect.take() {
                        rects.push(rect);
                    }
                    current_rect = Some(LayoutRect {
                        line: point.line.0,
                        num_lines: 1,
                        col: point.column.0,
                        num_cols: cell_cols,
                        color: cell_bg_color,
                    });
                }
            }

            // Skip space fillers following cells with zero-width extras (emoji sequences)
            let c = *c;
            if c == ' ' && previous_cell_had_extras {
                previous_cell_had_extras = false;
                continue;
            }

            // Track whether this cell has combining/zero-width characters
            let has_extras = matches!(zw, Some(chars) if !chars.is_empty());

            // Skip empty cells for text runs (space or NUL)
            if c == ' ' || c == '\0' {
                previous_cell_had_extras = has_extras;
                batch.flush();
                continue;
            }

            // Render block elements as filled quads instead of font glyphs
            // to eliminate subpixel gaps between adjacent cells (pixel art, logos).
            if let Some(coverage) = block_char_coverage(c) {
                batch.flush();
                block_quads.push(BlockQuad {
                    line: point.line.0,
                    col: point.column.0,
                    num_cols: cell_cols,
                    color: fg,
                    coverage,
                });
                previous_cell_had_extras = false;
                continue;
            }

            // Build cell style for batching comparison
            let mut font = Self::base_font();
            let is_underline = flags.contains(CellFlags::UNDERLINE)
                || flags.contains(CellFlags::DOUBLE_UNDERLINE)
                || flags.contains(CellFlags::UNDERCURL)
                || flags.contains(CellFlags::DOTTED_UNDERLINE)
                || flags.contains(CellFlags::DASHED_UNDERLINE);
            let is_undercurl = flags.contains(CellFlags::UNDERCURL);
            let is_strikethrough = flags.contains(CellFlags::STRIKEOUT);

            if flags.contains(CellFlags::BOLD) || flags.contains(CellFlags::BOLD_ITALIC) {
                font.weight = FontWeight::BOLD;
            }
            if flags.contains(CellFlags::ITALIC) || flags.contains(CellFlags::BOLD_ITALIC) {
                font.style = FontStyle::Italic;
            }

            let style = CellStyle {
                font: font.clone(),
                fg,
                bg,
                underline: is_underline,
                undercurl: is_undercurl,
                strikethrough: is_strikethrough,
            };

            // Check if we can append to current batch
            if batch.can_append(&style, point.line.0, point.column.0) {
                batch.append(c, cell_cols);
            } else {
                batch.flush();
                batch.start(
                    c,
                    cell_cols,
                    style,
                    font,
                    fg,
                    is_underline,
                    is_undercurl,
                    is_strikethrough,
                    point.line.0,
                    point.column.0,
                );
            }

            // Append zero-width combining characters (diacriticals, ZWJ, variation selectors)
            if let Some(chars) = zw {
                batch.append_zerowidth(chars);
            }
            previous_cell_had_extras = has_extras;
        }

        // Flush remaining
        batch.flush();
        if let Some(rect) = current_rect {
            rects.push(rect);
        }
        // Vertical merge: coalesce same-column-span, same-color, adjacent-line rects
        let rects = merge_background_regions(rects);
        // Finalize right-extension flag for the last line
        if last_line >= 0 && (last_line as usize) < desired_rows && last_nondefault_is_powerline {
            extend_line_flags[last_line as usize].1 = false;
        }

        // Build selection highlight rects from the SelectionRange
        let mut selection_rects = Vec::new();
        if let Some(sel) = &selection_range {
            let start = sel.start;
            let end = sel.end;
            let num_cols = desired_cols.max(1);

            if start.line == end.line {
                // Single-line selection
                selection_rects.push(LayoutRect {
                    line: start.line.0,
                    num_lines: 1,
                    col: start.column.0,
                    num_cols: end.column.0.saturating_sub(start.column.0) + 1,
                    color: selection_color,
                });
            } else {
                // Multi-line: first line from start.col to end of line
                selection_rects.push(LayoutRect {
                    line: start.line.0,
                    num_lines: 1,
                    col: start.column.0,
                    num_cols: num_cols.saturating_sub(start.column.0),
                    color: selection_color,
                });
                // Middle full lines
                let mut line = start.line.0 + 1;
                while line < end.line.0 {
                    selection_rects.push(LayoutRect {
                        line,
                        num_lines: 1,
                        col: 0,
                        num_cols,
                        color: selection_color,
                    });
                    line += 1;
                }
                // Last line from col 0 to end.col
                selection_rects.push(LayoutRect {
                    line: end.line.0,
                    num_lines: 1,
                    col: 0,
                    num_cols: end.column.0 + 1,
                    color: selection_color,
                });
            }
        }

        // Build search match highlight rects
        let search_match_color = Hsla {
            h: 0.11,
            s: 0.9,
            l: 0.55,
            a: 0.45,
        }; // Amber for inactive matches
        let search_active_color = Hsla {
            h: 0.08,
            s: 1.0,
            l: 0.6,
            a: 0.7,
        }; // Brighter orange for active match

        let mut search_rects = Vec::new();
        for highlight in &self.search_highlights {
            // Convert grid coordinates to display-relative line numbers
            // display_offset is the number of scrollback lines visible above the viewport
            // Visible lines are: -(display_offset as i32) .. (screen_lines - 1 - display_offset as i32)
            // A match at grid line L maps to display line: L.0 + display_offset as i32
            let display_line = highlight.start.line.0 + display_offset as i32;

            // Only paint if the match is in the visible area
            if display_line >= 0 && display_line < desired_rows as i32 {
                let color = if highlight.is_active {
                    search_active_color
                } else {
                    search_match_color
                };

                // Single-line match (search matches are always single-line)
                let col_start = highlight.start.column.0;
                let col_end = highlight.end.column.0;
                search_rects.push(LayoutRect {
                    line: display_line,
                    num_lines: 1,
                    col: col_start,
                    num_cols: col_end.saturating_sub(col_start) + 1,
                    color,
                });
            }
        }

        // Compute IME cursor bounds for popup positioning
        let ime_cursor_bounds = cursor_snapshot.as_ref().map(|c| {
            let x = dims.cell_width * c.col as f32;
            let y = dims.line_height * c.line as f32;
            Bounds::new(
                Point { x, y },
                gpui::Size {
                    width: dims.cell_width,
                    height: dims.line_height,
                },
            )
        });

        LayoutState {
            batched_runs: batch.runs,
            rects,
            block_quads,
            selection_rects,
            search_rects,
            cursor: cursor_snapshot,
            dimensions: dims,
            background_color,
            scrollbar_thumb: theme.scrollbar_thumb,
            exited: self.exited,
            display_offset,
            history_size,
            desired_cols,
            desired_rows,
            extend_line_flags,
            hyperlinks,
            link_text_color: theme.link_text,
            ime_cursor_bounds,
        }
    }
}

struct BatchAccumulator {
    runs: Vec<BatchedTextRun>,
    text: String,
    style: Option<CellStyle>,
    font: Font,
    fg: Hsla,
    underline: bool,
    undercurl: bool,
    strikethrough: bool,
    line: i32,
    col_start: usize,
    col_end: usize, // next expected column (tracks wide chars correctly)
}

impl BatchAccumulator {
    fn new() -> Self {
        Self {
            runs: Vec::new(),
            text: String::new(),
            style: None,
            font: TerminalElement::base_font(),
            fg: Hsla::default(),
            underline: false,
            undercurl: false,
            strikethrough: false,
            line: 0,
            col_start: 0,
            col_end: 0,
        }
    }

    fn can_append(&self, style: &CellStyle, line: i32, col: usize) -> bool {
        match &self.style {
            Some(cs) => *cs == *style && self.line == line && col == self.col_end,
            None => false,
        }
    }

    fn append(&mut self, c: char, cell_cols: usize) {
        self.text.push(c);
        self.col_end += cell_cols;
    }

    fn append_zerowidth(&mut self, chars: &[char]) {
        debug_assert!(
            !self.text.is_empty(),
            "zero-width chars require a base character"
        );
        for &c in chars {
            self.text.push(c);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        c: char,
        cell_cols: usize,
        style: CellStyle,
        font: Font,
        fg: Hsla,
        underline: bool,
        undercurl: bool,
        strikethrough: bool,
        line: i32,
        col_start: usize,
    ) {
        self.text.push(c);
        self.style = Some(style);
        self.font = font;
        self.fg = fg;
        self.underline = underline;
        self.undercurl = undercurl;
        self.strikethrough = strikethrough;
        self.line = line;
        self.col_start = col_start;
        self.col_end = col_start + cell_cols;
    }

    fn flush(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.runs.push(BatchedTextRun {
            text: std::mem::take(&mut self.text),
            font: self.font.clone(),
            color: self.fg,
            underline: if self.underline {
                Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(self.fg),
                    wavy: self.undercurl,
                })
            } else {
                None
            },
            strikethrough: if self.strikethrough {
                Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(self.fg),
                })
            } else {
                None
            },
            line: self.line,
            col_start: self.col_start,
        });
        self.style = None;
    }
}

// ---------------------------------------------------------------------------
// Element trait implementation
// ---------------------------------------------------------------------------

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<LayoutState>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        Some(self.build_layout(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(debug_assertions)]
        let _paint_start = if crate::terminal::probe_enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        let Some(layout) = prepaint.take() else {
            return;
        };

        let cell_width = layout.dimensions.cell_width;
        // Offset origin by left gutter (1 cell width)
        let origin = Point {
            x: bounds.origin.x + cell_width,
            y: bounds.origin.y,
        };
        // Store gutter-adjusted origin for mouse → grid coordinate conversion
        *self.element_origin.lock().unwrap() = origin;
        let line_height = layout.dimensions.line_height;
        let font_size = Self::font_size();

        // Clip to element bounds
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // 1. Paint full terminal background
            window.paint_quad(fill(bounds, layout.background_color));

            // Bell flash: semi-transparent white overlay
            if self.bell_flash_active {
                window.paint_quad(fill(bounds, gpui::hsla(0., 0., 1., 0.12)));
            }

            // 2. Paint per-cell background rects (pixel-aligned to prevent gaps)
            //    Edge cells extend into gutter/padding to fill the full widget,
            //    matching Ghostty's EXTEND_LEFT/RIGHT/UP/DOWN (EP-001/US-001+US-002).
            let widget_left = bounds.origin.x;
            let widget_right = bounds.origin.x + bounds.size.width;
            let widget_top = bounds.origin.y;
            let widget_bottom = bounds.origin.y + bounds.size.height;
            let last_row = layout.desired_rows.saturating_sub(1) as i32;
            for rect in &layout.rects {
                // Zed-parity positioning: floor(x), ceil(width), raw y, single line_height.
                let mut x = (origin.x + cell_width * rect.col as f32).floor();
                let mut y = origin.y + line_height * rect.line as f32;
                let w = (cell_width * rect.num_cols as f32).ceil();
                let mut right = x + w;
                let mut bottom = y + line_height * rect.num_lines as f32;
                let last_rect_line = rect.line + rect.num_lines as i32 - 1;

                // Look up per-line extension flags (neverExtendBg, US-003)
                // For vertically merged rects, use the first line's flags for edges.
                let (extend_left, extend_right) =
                    if rect.line >= 0 && (rect.line as usize) < layout.extend_line_flags.len() {
                        layout.extend_line_flags[rect.line as usize]
                    } else {
                        (true, true)
                    };

                // Extend left edge into gutter for column-0 rects
                if rect.col == 0 && extend_left {
                    x = widget_left;
                }
                // Extend right edge to widget boundary for last-column rects
                if rect.col + rect.num_cols >= layout.desired_cols && extend_right {
                    right = widget_right;
                }
                // Vertical extension is unconditional — Powerline glyphs only
                // create horizontal edge artifacts, not vertical ones.
                if rect.line == 0 {
                    y = widget_top;
                }
                if last_rect_line == last_row {
                    bottom = widget_bottom;
                }

                let rect_bounds = Bounds::new(
                    Point { x, y },
                    gpui::Size {
                        width: (right - x).max(px(0.0)),
                        height: (bottom - y).max(px(0.0)),
                    },
                );
                window.paint_quad(fill(rect_bounds, rect.color));
            }

            // 2b. Paint selection highlight rects (pixel-aligned, rounded corners)
            let selection_corner_radius = line_height * 0.15;
            for rect in &layout.selection_rects {
                let x = (origin.x + cell_width * rect.col as f32).floor();
                let y = origin.y + line_height * rect.line as f32;
                let w = (cell_width * rect.num_cols as f32).ceil();
                let rect_bounds = Bounds::new(
                    Point { x, y },
                    gpui::Size {
                        width: w,
                        height: line_height,
                    },
                );
                window.paint_quad(
                    fill(rect_bounds, rect.color).corner_radii(selection_corner_radius),
                );
            }

            // 2c. Paint search match highlight rects
            for rect in &layout.search_rects {
                let x = (origin.x + cell_width * rect.col as f32).floor();
                let y = origin.y + line_height * rect.line as f32;
                let w = (cell_width * rect.num_cols as f32).ceil();
                let rect_bounds = Bounds::new(
                    Point { x, y },
                    gpui::Size {
                        width: w,
                        height: line_height,
                    },
                );
                window.paint_quad(fill(rect_bounds, rect.color));
            }

            // 2c. Paint block element quads (pixel-perfect, no font glyph gaps)
            for bq in &layout.block_quads {
                let cx_start = origin.x + cell_width * bq.col as f32;
                let cy_start = origin.y + line_height * bq.line as f32;
                let cw = cell_width * bq.num_cols as f32;
                let ch = line_height;
                let (fx, fy, fw, fh) = bq.coverage;
                let qx = (cx_start + cw * fx).floor();
                let qy = cy_start + ch * fy;
                let qw = (cw * fw).ceil();
                let qh = ch * fh;
                window.paint_quad(fill(
                    Bounds::new(
                        Point { x: qx, y: qy },
                        gpui::Size { width: qw, height: qh },
                    ),
                    bq.color,
                ));
            }

            // 3. Paint batched text runs
            for run in &layout.batched_runs {
                let x = origin.x + cell_width * run.col_start as f32;
                let y = origin.y + line_height * run.line as f32;
                let text_run = TextRun {
                    len: run.text.len(),
                    font: run.font.clone(),
                    color: run.color,
                    background_color: None,
                    underline: run.underline,
                    strikethrough: run.strikethrough,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(run.text.clone()),
                    font_size,
                    &[text_run],
                    Some(cell_width),
                );
                let _ = shaped.paint(
                    Point { x, y },
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // 3b. Paint hyperlink underline (Ctrl+hover)
            if let Some((link_line, col_start, col_end)) = self.hovered_link_range {
                let display_offset = layout.display_offset as i32;
                let screen_line = link_line.0 + display_offset;
                if screen_line >= 0 && (screen_line as usize) < layout.desired_rows {
                    let x_start = origin.x + cell_width * col_start as f32;
                    let x_end = origin.x + cell_width * (col_end + 1) as f32;
                    let y = origin.y + line_height * (screen_line + 1) as f32 - gpui::px(1.0);
                    let underline_bounds = Bounds::new(
                        Point { x: x_start, y },
                        gpui::Size {
                            width: x_end - x_start,
                            height: gpui::px(1.0),
                        },
                    );
                    window.paint_quad(fill(underline_bounds, layout.link_text_color));

                    // Paint URL tooltip near the underline
                    if let Some(ref uri) = self.hovered_link_uri {
                        let tooltip_font_size = gpui::px(11.0);
                        let tooltip_padding = gpui::px(4.0);
                        // Char-safe truncation to avoid panics on multibyte URIs
                        let display_uri: String = if uri.chars().count() > 80 {
                            let mut s: String = uri.chars().take(77).collect();
                            s.push_str("...");
                            s
                        } else {
                            uri.clone()
                        };
                        let display_len = display_uri.len(); // UTF-8 byte count for TextRun
                        let shaped = window.text_system().shape_line(
                            SharedString::from(display_uri),
                            tooltip_font_size,
                            &[gpui::TextRun {
                                len: display_len,
                                font: gpui::Font {
                                    family: "monospace".into(),
                                    ..Default::default()
                                },
                                color: layout.link_text_color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        let text_width = shaped.width;
                        let tooltip_height = tooltip_font_size + tooltip_padding * 2.0;
                        let tooltip_x = x_start;
                        // Flip tooltip above the link when near the bottom of the terminal
                        let tooltip_y = {
                            let below = y + gpui::px(3.0);
                            let bottom_edge = origin.y + line_height * layout.desired_rows as f32;
                            if below + tooltip_height > bottom_edge {
                                // Place above the link line
                                origin.y + line_height * screen_line as f32
                                    - tooltip_height
                                    - gpui::px(2.0)
                            } else {
                                below
                            }
                        };
                        let bg_bounds = Bounds::new(
                            Point {
                                x: tooltip_x - tooltip_padding,
                                y: tooltip_y,
                            },
                            gpui::Size {
                                width: text_width + tooltip_padding * 2.0,
                                height: tooltip_height,
                            },
                        );
                        // Semi-transparent overlay background for visibility
                        let mut tooltip_bg = layout.background_color;
                        tooltip_bg.a = 0.92;
                        window.paint_quad(fill(bg_bounds, tooltip_bg));
                        let _ = shaped.paint(
                            Point {
                                x: tooltip_x,
                                y: tooltip_y + tooltip_padding,
                            },
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }

            // 4. Paint cursor
            if let Some(cursor) = &layout.cursor {
                let cx_ = origin.x + cell_width * cursor.col as f32;
                let cy = origin.y + line_height * cursor.line as f32;
                let mut cw = if cursor.wide {
                    cell_width * 2.0
                } else {
                    cell_width
                };
                let ch = line_height;
                let color = cursor.color;

                match cursor.shape {
                    CursorShape::Block => {
                        // Shape the cursor character first so we can size the
                        // cursor quad to fit wide/emoji glyphs.
                        let shaped = cursor.text.map(|ch| {
                            let mut cursor_font = Self::base_font();
                            if cursor.bold {
                                cursor_font.weight = FontWeight::BOLD;
                            }
                            if cursor.italic {
                                cursor_font.style = FontStyle::Italic;
                            }
                            let text = ch.to_string();
                            let len = text.len();
                            window.text_system().shape_line(
                                SharedString::from(text),
                                Self::font_size(),
                                &[TextRun {
                                    len,
                                    font: cursor_font,
                                    color: layout.background_color,
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            )
                        });

                        // Widen cursor to fit glyphs that exceed cell_width * 2
                        if cursor.wide
                            && let Some(ref shaped) = shaped
                        {
                            cw = cw.max(shaped.width());
                        }

                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: cw,
                                height: ch,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));

                        // Paint the character on top of the cursor quad
                        if let Some(shaped) = shaped {
                            let _ = shaped.paint(
                                Point { x: cx_, y: cy },
                                line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    CursorShape::Beam => {
                        let beam_width = px(2.0);
                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: beam_width,
                                height: ch,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));
                    }
                    CursorShape::Underline => {
                        let underline_height = px(2.0);
                        let cursor_bounds = Bounds::new(
                            Point {
                                x: cx_,
                                y: cy + ch - underline_height,
                            },
                            gpui::Size {
                                width: cw,
                                height: underline_height,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));
                    }
                    CursorShape::HollowBlock => {
                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: cw,
                                height: ch,
                            },
                        );
                        window.paint_quad(
                            outline(cursor_bounds, color, BorderStyle::Solid)
                                .border_widths(1.5)
                                .corner_radii(px(2.0)),
                        );
                    }
                    CursorShape::Hidden => {} // Already filtered in build_layout
                }
            }

            // 5. Paint scrollbar indicator (thin overlay, right edge)
            if layout.display_offset > 0 && layout.history_size > 0 {
                let scrollbar_width = px(4.0);
                let visible_rows = (bounds.size.height / line_height).floor() as usize;
                let total_lines = layout.history_size + visible_rows;
                let visible_ratio = visible_rows as f32 / total_lines as f32;
                let thumb_height = (bounds.size.height * visible_ratio).max(px(16.0));
                let scroll_ratio = layout.display_offset as f32 / layout.history_size as f32;
                // display_offset=max → scrolled to top → thumb at top
                let thumb_y = bounds.size.height
                    - thumb_height
                    - (bounds.size.height - thumb_height) * scroll_ratio;
                let scrollbar_color = layout.scrollbar_thumb;
                let scrollbar_bounds = Bounds::new(
                    Point {
                        x: origin.x + bounds.size.width - scrollbar_width,
                        y: origin.y + thumb_y,
                    },
                    gpui::Size {
                        width: scrollbar_width,
                        height: thumb_height,
                    },
                );
                window.paint_quad(fill(scrollbar_bounds, scrollbar_color));
            }

            // 6. Register IME input handler and paint preedit text
            if self.focused {
                let cursor_bounds = layout.ime_cursor_bounds.map(|b| {
                    Bounds::new(
                        Point {
                            x: b.origin.x + origin.x,
                            y: b.origin.y + origin.y,
                        },
                        b.size,
                    )
                });
                let handler = TerminalInputHandler {
                    terminal_view: self.terminal_view.clone(),
                    term: self.term.clone(),
                    cursor_bounds,
                };
                window.handle_input(&self.focus_handle, handler, cx);

                // Paint preedit overlay
                if !self.ime_marked_text.is_empty()
                    && let Some(cb) = cursor_bounds
                {
                    let ime_font = Self::base_font();
                    let ime_run = TextRun {
                        len: self.ime_marked_text.len(),
                        font: ime_font,
                        color: layout.background_color,
                        background_color: None,
                        underline: Some(gpui::UnderlineStyle {
                            color: None,
                            thickness: px(1.0),
                            wavy: false,
                        }),
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(
                        SharedString::from(self.ime_marked_text.clone()),
                        font_size,
                        &[ime_run],
                        Some(cell_width),
                    );
                    // Background erase behind preedit
                    let preedit_width = shaped.width();
                    let preedit_bg = Bounds::new(
                        cb.origin,
                        gpui::Size {
                            width: preedit_width,
                            height: line_height,
                        },
                    );
                    window.paint_quad(fill(preedit_bg, layout.background_color));
                    // Paint preedit text
                    let _ = shaped.paint(cb.origin, line_height, TextAlign::Left, None, window, cx);
                }
            }

            // 7. Paint exit overlay if process has exited
            if let Some(code) = layout.exited {
                let msg = format!("[Process exited with code {code}]");
                let exit_fg = rgb_to_hsla(0x6c, 0x70, 0x86); // Overlay6
                let run = TextRun {
                    len: msg.len(),
                    font: Self::base_font(),
                    color: exit_fg,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(msg),
                    font_size,
                    &[run],
                    None,
                );
                // Center the message in the terminal bounds
                let text_width = shaped.width();
                let x = origin.x + (bounds.size.width - text_width) * 0.5;
                let y = origin.y + (bounds.size.height - line_height) * 0.5;
                let _ = shaped.paint(
                    Point { x, y },
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });

        #[cfg(debug_assertions)]
        if let Some(paint_start) = _paint_start {
            let paint_elapsed = paint_start.elapsed();
            let paint_ms = paint_elapsed.as_secs_f64() * 1000.0;

            // Phase 2: paint() duration
            if paint_ms > 1.0 {
                log::warn!("[latency] paint: {paint_ms:.2}ms");
            }

            // Phase 3: total keystroke → pixel with per-phase breakdown
            if let Some(keystroke_at) = self.last_keystroke_at {
                let total_elapsed = keystroke_at.elapsed();
                let total_ms = total_elapsed.as_secs_f64() * 1000.0;
                let pty_to_paint_ms = total_ms - paint_ms;
                if total_ms > 8.0 {
                    log::warn!(
                        "[latency] keystroke→pixel: {total_ms:.2}ms \
                         (pty_write→paint_start: {pty_to_paint_ms:.2}ms, \
                         paint: {paint_ms:.2}ms)"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IntoElement implementation
// ---------------------------------------------------------------------------

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

fn convert_color(color: AnsiColor, theme: &crate::theme::TerminalTheme) -> Hsla {
    match color {
        AnsiColor::Named(name) => named_color(name, theme),
        AnsiColor::Spec(rgb) if rgb.r == 0 && rgb.g == 0 && rgb.b == 0 => theme.black,
        AnsiColor::Spec(rgb) => rgb_to_hsla(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) => indexed_color(i, theme),
    }
}

fn named_color(name: NamedColor, theme: &crate::theme::TerminalTheme) -> Hsla {
    match name {
        NamedColor::Black => theme.black,
        NamedColor::Red => theme.red,
        NamedColor::Green => theme.green,
        NamedColor::Yellow => theme.yellow,
        NamedColor::Blue => theme.blue,
        NamedColor::Magenta => theme.magenta,
        NamedColor::Cyan => theme.cyan,
        NamedColor::White => theme.white,
        NamedColor::BrightBlack => theme.bright_black,
        NamedColor::BrightRed => theme.bright_red,
        NamedColor::BrightGreen => theme.bright_green,
        NamedColor::BrightYellow => theme.bright_yellow,
        NamedColor::BrightBlue => theme.bright_blue,
        NamedColor::BrightMagenta => theme.bright_magenta,
        NamedColor::BrightCyan => theme.bright_cyan,
        NamedColor::BrightWhite => theme.bright_white,
        NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
        NamedColor::Background => theme.ansi_background,
        NamedColor::DimBlack => theme.dim_black,
        NamedColor::DimRed => theme.dim_red,
        NamedColor::DimGreen => theme.dim_green,
        NamedColor::DimYellow => theme.dim_yellow,
        NamedColor::DimBlue => theme.dim_blue,
        NamedColor::DimMagenta => theme.dim_magenta,
        NamedColor::DimCyan => theme.dim_cyan,
        NamedColor::DimWhite => theme.dim_white,
        NamedColor::DimForeground => theme.dim_foreground,
        NamedColor::Cursor => theme.cursor,
    }
}

/// Convert the xterm-256color indexed palette to HSLA.
fn indexed_color(i: u8, theme: &crate::theme::TerminalTheme) -> Hsla {
    if i < 16 {
        // Standard 16 colors — map to named
        return named_color(
            match i {
                0 => NamedColor::Black,
                1 => NamedColor::Red,
                2 => NamedColor::Green,
                3 => NamedColor::Yellow,
                4 => NamedColor::Blue,
                5 => NamedColor::Magenta,
                6 => NamedColor::Cyan,
                7 => NamedColor::White,
                8 => NamedColor::BrightBlack,
                9 => NamedColor::BrightRed,
                10 => NamedColor::BrightGreen,
                11 => NamedColor::BrightYellow,
                12 => NamedColor::BrightBlue,
                13 => NamedColor::BrightMagenta,
                14 => NamedColor::BrightCyan,
                15 => NamedColor::BrightWhite,
                _ => unreachable!(),
            },
            theme,
        );
    }

    if i < 232 {
        // 6x6x6 color cube (indices 16-231)
        let idx = i - 16;
        let r_idx = idx / 36;
        let g_idx = (idx % 36) / 6;
        let b_idx = idx % 6;
        let r = if r_idx == 0 { 0 } else { 55 + 40 * r_idx };
        let g = if g_idx == 0 { 0 } else { 55 + 40 * g_idx };
        let b = if b_idx == 0 { 0 } else { 55 + 40 * b_idx };
        return rgb_to_hsla(r, g, b);
    }

    // Grayscale ramp (indices 232-255)
    let gray = 8 + 10 * (i - 232);
    rgb_to_hsla(gray, gray, gray)
}

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    Hsla::from(Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    })
}

// ---------------------------------------------------------------------------
// IME InputHandler (US-017)
// ---------------------------------------------------------------------------

struct TerminalInputHandler {
    terminal_view: gpui::Entity<crate::terminal::TerminalView>,
    term: Arc<FairMutex<Term<ZedListener>>>,
    cursor_bounds: Option<Bounds<Pixels>>,
}

impl gpui::InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<gpui::UTF16Selection> {
        // Disable IME on ALT_SCREEN (TUI apps handle their own input)
        let mode = *self.term.lock().mode();
        if mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN) {
            return None;
        }
        Some(gpui::UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<std::ops::Range<usize>> {
        self.terminal_view.read(cx).marked_text_range()
    }

    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        // Commit: clear preedit and write text to PTY
        self.terminal_view.update(cx, |view, cx| {
            view.clear_marked_text(cx);
            view.commit_text(text, cx);
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        // Preedit: update marked text for rendering
        self.terminal_view.update(cx, |view, cx| {
            view.set_marked_text(new_text.to_string(), cx);
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        // Cancel composition
        self.terminal_view.update(cx, |view, cx| {
            view.clear_marked_text(cx);
        });
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        None
    }
}
