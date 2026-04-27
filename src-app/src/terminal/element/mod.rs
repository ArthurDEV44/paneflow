//! Terminal cell renderer using GPUI's Element trait.
//!
//! Renders terminal cells from alacritty_terminal as batched text runs with
//! full ANSI color support, cell attributes, and background quads.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Line as GridLine, Point as AlacPoint};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, StrikethroughStyle, Style,
    UnderlineStyle, Window, px, relative,
};

use crate::terminal::types::{
    CopyModeCursorState, HyperlinkSource, HyperlinkZone, SearchHighlight,
};
use crate::terminal::{PtyNotifier, SpikeTermSize, ZedListener};

mod color;
mod font;
mod geometry;
mod hyperlink;
mod paint;
#[cfg(debug_assertions)]
pub(super) mod pixel_probe;

use color::{convert_color, rgb_to_hsla};
use font::{base_font, font_size};
pub use font::{measure_cell, resolve_font_family};
use geometry::CellGeometry;
pub use hyperlink::{detect_urls_on_line_mapped, is_url_scheme_openable};

// US-007: re-export APCA primitives so theme code (and theme tests) can
// derive and verify a contrast-validated `selection_foreground` color
// without duplicating the algorithm. `ensure_minimum_contrast` is also
// used locally by `build_layout` (cell-vs-bg pass); `apca_contrast` is
// referenced only by theme tests but must be re-exported through this
// module to honour `pub(crate)` visibility.
#[allow(unused_imports)] // re-exported for theme tests; not used inside this module
pub(crate) use color::apca_contrast;
pub(crate) use color::ensure_minimum_contrast;

/// APCA minimum Lc (lightness contrast) threshold.
/// Lc 45 is "minimum for large fluent text" per ARC Bronze Simple Mode — matches Zed's default.
/// APCA is more accurate than WCAG 2.0 on dark backgrounds (polarity-aware, perceptually uniform).
pub(crate) const MIN_APCA_CONTRAST: f32 = 45.0;

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

/// US-007: returns `true` if a cell at `point` (viewport coordinates) lies
/// inside the active `SelectionRange` (whose `start`/`end` are in scrollback
/// coordinates and require `display_offset` correction). Mirrors the
/// `selection_rects` generation block below — first/last/middle line ranges
/// for linear selections, axis-aligned rectangle for block selections.
///
/// Used inside the cell loop to override the cell's `fg` with the theme's
/// `selection_foreground`, guaranteeing readable text under the selection
/// quad on themes whose `selection` background is close in luminance to
/// common ANSI colors.
fn is_cell_in_selection(point: AlacPoint, sel: &SelectionRange, display_offset: usize) -> bool {
    let start_line = sel.start.line.0 + display_offset as i32;
    let end_line = sel.end.line.0 + display_offset as i32;
    let start_col = sel.start.column.0;
    let end_col = sel.end.column.0;

    let cell_line = point.line.0;
    let cell_col = point.column.0;

    if sel.is_block {
        let (l_min, l_max) = if start_line <= end_line {
            (start_line, end_line)
        } else {
            (end_line, start_line)
        };
        let (c_min, c_max) = if start_col <= end_col {
            (start_col, end_col)
        } else {
            (end_col, start_col)
        };
        return cell_line >= l_min && cell_line <= l_max && cell_col >= c_min && cell_col <= c_max;
    }

    // Linear selection: normalize so (s_line, s_col) is reading-order start.
    let ((s_line, s_col), (e_line, e_col)) =
        if start_line < end_line || (start_line == end_line && start_col <= end_col) {
            ((start_line, start_col), (end_line, end_col))
        } else {
            ((end_line, end_col), (start_line, start_col))
        };
    if cell_line < s_line || cell_line > e_line {
        false
    } else if s_line == e_line {
        cell_col >= s_col && cell_col <= e_col
    } else if cell_line == s_line {
        cell_col >= s_col
    } else if cell_line == e_line {
        cell_col <= e_col
    } else {
        true
    }
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

/// If `c` is a Unicode block element, return its fractional cell coverage as
/// a slice of `(x, y, w, h)` rects (origin at the cell's top-left, in 0..1).
/// Returns `None` for characters that should be rendered as normal glyphs.
///
/// Most block-element codepoints are a single rectangle, but the multi-quadrant
/// chars (`▙ ▚ ▛ ▜ ▞ ▟`) need 2 rects each — that's why this returns a slice.
/// Each emitted rect becomes one [`BlockQuad`] at the call site, all sharing
/// the same outer cell boundaries through [`paint::background::cell_x_boundaries`].
///
/// US-005 fallback: extension beyond the original `U+2580..U+2590` range, after
/// the pixel probe revealed Claude Code's banner robot uses single + multi
/// quadrant blocks (`U+2596..U+259F`) and the upper one-eighth block (`U+2594`).
/// Without this extension these codepoints fall back to font glyphs that don't
/// fully fill the cell, producing the visible vertical gaps documented in the
/// `debug_block_char_rendering.md` memory.
fn block_char_coverages(c: char) -> Option<&'static [(f32, f32, f32, f32)]> {
    match c {
        // U+2580..U+2590 — half / eighth blocks (one rect each)
        '▀' => Some(&[(0.0, 0.0, 1.0, 0.5)]), // U+2580 Upper half
        '▁' => Some(&[(0.0, 7.0 / 8.0, 1.0, 1.0 / 8.0)]), // U+2581 Lower 1/8
        '▂' => Some(&[(0.0, 6.0 / 8.0, 1.0, 2.0 / 8.0)]), // U+2582 Lower 1/4
        '▃' => Some(&[(0.0, 5.0 / 8.0, 1.0, 3.0 / 8.0)]), // U+2583 Lower 3/8
        '▄' => Some(&[(0.0, 0.5, 1.0, 0.5)]), // U+2584 Lower half
        '▅' => Some(&[(0.0, 3.0 / 8.0, 1.0, 5.0 / 8.0)]), // U+2585 Lower 5/8
        '▆' => Some(&[(0.0, 2.0 / 8.0, 1.0, 6.0 / 8.0)]), // U+2586 Lower 3/4
        '▇' => Some(&[(0.0, 1.0 / 8.0, 1.0, 7.0 / 8.0)]), // U+2587 Lower 7/8
        '█' => Some(&[(0.0, 0.0, 1.0, 1.0)]), // U+2588 Full block
        '▉' => Some(&[(0.0, 0.0, 7.0 / 8.0, 1.0)]), // U+2589 Left 7/8
        '▊' => Some(&[(0.0, 0.0, 6.0 / 8.0, 1.0)]), // U+258A Left 3/4
        '▋' => Some(&[(0.0, 0.0, 5.0 / 8.0, 1.0)]), // U+258B Left 5/8
        '▌' => Some(&[(0.0, 0.0, 0.5, 1.0)]), // U+258C Left half
        '▍' => Some(&[(0.0, 0.0, 3.0 / 8.0, 1.0)]), // U+258D Left 3/8
        '▎' => Some(&[(0.0, 0.0, 2.0 / 8.0, 1.0)]), // U+258E Left 1/4
        '▏' => Some(&[(0.0, 0.0, 1.0 / 8.0, 1.0)]), // U+258F Left 1/8
        '▐' => Some(&[(0.5, 0.0, 0.5, 1.0)]), // U+2590 Right half

        // ─── US-005 fallback extension ────────────────────────────────────
        // U+2594 — Upper 1/8 (the lone "upper edge" block, complement of ▁)
        '▔' => Some(&[(0.0, 0.0, 1.0, 1.0 / 8.0)]),

        // U+2596..U+259D — single quadrants
        '▖' => Some(&[(0.0, 0.5, 0.5, 0.5)]), // U+2596 Quadrant lower left
        '▗' => Some(&[(0.5, 0.5, 0.5, 0.5)]), // U+2597 Quadrant lower right
        '▘' => Some(&[(0.0, 0.0, 0.5, 0.5)]), // U+2598 Quadrant upper left
        '▝' => Some(&[(0.5, 0.0, 0.5, 0.5)]), // U+259D Quadrant upper right

        // U+2599..U+259F — multi-quadrants (2 rects each, each rect already
        // shares its outer edges with the surrounding cell's boundary array
        // via `paint_block_quads` → no inter-rect gaps possible).
        '▙' => Some(&[
            // U+2599 Quadrant upper-left + entire lower half
            (0.0, 0.0, 0.5, 0.5),
            (0.0, 0.5, 1.0, 0.5),
        ]),
        '▚' => Some(&[
            // U+259A Diagonal upper-left + lower-right
            (0.0, 0.0, 0.5, 0.5),
            (0.5, 0.5, 0.5, 0.5),
        ]),
        '▛' => Some(&[
            // U+259B Entire upper half + lower-left
            (0.0, 0.0, 1.0, 0.5),
            (0.0, 0.5, 0.5, 0.5),
        ]),
        '▜' => Some(&[
            // U+259C Entire upper half + lower-right
            (0.0, 0.0, 1.0, 0.5),
            (0.5, 0.5, 0.5, 0.5),
        ]),
        '▞' => Some(&[
            // U+259E Diagonal upper-right + lower-left
            (0.5, 0.0, 0.5, 0.5),
            (0.0, 0.5, 0.5, 0.5),
        ]),
        '▟' => Some(&[
            // U+259F Quadrant upper-right + entire lower half
            (0.5, 0.0, 0.5, 0.5),
            (0.0, 0.5, 1.0, 0.5),
        ]),
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

pub struct LayoutState {
    batched_runs: Vec<BatchedTextRun>,
    rects: Vec<LayoutRect>,
    block_quads: Vec<BlockQuad>,
    selection_rects: Vec<LayoutRect>,
    search_rects: Vec<LayoutRect>,
    cursor: Option<CursorInfo>,
    /// Selection anchor cursor in copy mode — rendered as a distinct amber hollow
    /// block so the user can see where the selection started (tmux-style).
    anchor_cursor: Option<CursorInfo>,
    dimensions: CellDimensions,
    background_color: Hsla,
    scrollbar_thumb: Hsla,
    exited: Option<i32>,
    /// Scroll position for scrollbar indicator (0 = at bottom)
    display_offset: usize,
    /// Total scrollback history size
    history_size: usize,
    /// Number of columns in the terminal grid
    desired_cols: usize,
    /// Number of rows in the terminal grid
    desired_rows: usize,
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
    /// Gate for clearing pre-resize shell startup content on first render.
    needs_initial_clear: Arc<std::sync::atomic::AtomicBool>,
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
        needs_initial_clear: Arc<std::sync::atomic::AtomicBool>,
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
            needs_initial_clear,
            #[cfg(debug_assertions)]
            last_keystroke_at,
        }
    }

    fn build_layout(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutState {
        let dims = measure_cell(window, cx);
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
            // On the very first resize, clear any shell startup content that
            // landed in the grid before we knew the actual window dimensions.
            // The shell receives SIGWINCH and redraws its prompt at the
            // correct width, so nothing visible is lost.
            if self
                .needs_initial_clear
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                term.grid_mut().reset();
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
            // Transform grid-line coords (where scrollback rows are negative) into
            // viewport-line coords so the rest of the paint pipeline — culling,
            // Y positioning, hyperlink zones, batching — all speak the same
            // coordinate system as the cursor and search-highlight code.
            let disp_offset_i = disp_offset as i32;
            let cells: Vec<_> = content
                .display_iter
                .map(|ic| {
                    let zw = ic.cell.zerowidth().map(|chars| chars.to_vec());
                    let hyperlink = ic.cell.hyperlink();
                    let viewport_point =
                        AlacPoint::new(GridLine(ic.point.line.0 + disp_offset_i), ic.point.column);
                    (
                        viewport_point,
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

        // Override cursor with copy mode cursor when active, and surface the
        // selection anchor as a distinct secondary marker (tmux-style).
        let (cursor_snapshot, anchor_cursor) = if let Some(ref cm) = self.copy_mode_cursor {
            let display_line = cm.grid_line + display_offset as i32;
            let copy_cursor_color = Hsla {
                h: 0.5,
                s: 0.8,
                l: 0.65,
                a: 0.9,
            }; // Bright cyan — the moving cursor (current position)
            let anchor_color = Hsla {
                h: 0.12,
                s: 0.95,
                l: 0.6,
                a: 0.95,
            }; // Amber — the anchor (selection start)

            let main = if display_line >= 0 && display_line < desired_rows as i32 {
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
            };

            let anchor = cm.anchor_grid_line.and_then(|anchor_line| {
                let display_anchor = anchor_line + display_offset as i32;
                if display_anchor >= 0 && display_anchor < desired_rows as i32 {
                    Some(CursorInfo {
                        line: display_anchor,
                        col: cm.anchor_col,
                        shape: CursorShape::HollowBlock,
                        color: anchor_color,
                        wide: false,
                        text: None,
                        bold: false,
                        italic: false,
                    })
                } else {
                    None
                }
            });

            (main, anchor)
        } else if let Some(sel) = &selection_range {
            // Mouse selection (no copy mode): mark both endpoints with distinct
            // hollow blocks so the user can see the selection bounds precisely
            // before copying. Keep the normal shell cursor untouched.
            let anchor_color = Hsla {
                h: 0.12,
                s: 0.95,
                l: 0.6,
                a: 0.95,
            }; // Amber — selection start
            let end_color = Hsla {
                h: 0.5,
                s: 0.8,
                l: 0.65,
                a: 0.9,
            }; // Cyan — selection end

            let start_line = sel.start.line.0 + display_offset as i32;
            let end_line = sel.end.line.0 + display_offset as i32;

            let anchor = if start_line >= 0 && start_line < desired_rows as i32 {
                Some(CursorInfo {
                    line: start_line,
                    col: sel.start.column.0,
                    shape: CursorShape::HollowBlock,
                    color: anchor_color,
                    wide: false,
                    text: None,
                    bold: false,
                    italic: false,
                })
            } else {
                None
            };

            // Overload cursor_snapshot with the selection end marker so both
            // ends are visible. The shell's real cursor is hidden by the
            // selection highlight anyway during a drag.
            let end_marker = if end_line >= 0 && end_line < desired_rows as i32 {
                Some(CursorInfo {
                    line: end_line,
                    col: sel.end.column.0,
                    shape: CursorShape::HollowBlock,
                    color: end_color,
                    wide: false,
                    text: None,
                    bold: false,
                    italic: false,
                })
            } else {
                None
            };

            (end_marker.or(cursor_snapshot), anchor)
        } else {
            (cursor_snapshot, None)
        };

        let mut batch = BatchAccumulator::new();
        let mut rects: Vec<LayoutRect> = Vec::new();
        let mut block_quads: Vec<BlockQuad> = Vec::new();
        let mut current_rect: Option<LayoutRect> = None;
        let mut last_line: i32 = i32::MIN;
        let mut previous_cell_had_extras = false;

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

            // Line change → flush batch and rect
            if point.line.0 != last_line {
                batch.flush();
                if let Some(rect) = current_rect.take() {
                    rects.push(rect);
                }
                last_line = point.line.0;
            }

            // Compute colors — INVERSE swap on raw ANSI tags, then tag-based
            // default-background skip (Zed parity: structural check, not HSLA compare).
            let (raw_fg, raw_bg) = if flags.contains(CellFlags::INVERSE) {
                (*cell_bg, *cell_fg)
            } else {
                (*cell_fg, *cell_bg)
            };
            let is_default_bg = matches!(raw_bg, AnsiColor::Named(NamedColor::Background));

            let mut fg = convert_color(raw_fg, &theme);
            let bg = convert_color(raw_bg, &theme);

            // DIM/faint (SGR 2): reduce foreground opacity (applied after INVERSE)
            if flags.contains(CellFlags::DIM) {
                fg.a *= 0.7;
            }

            // Enforce minimum foreground/background contrast (skip decorative chars)
            if !is_decorative_character(*c) {
                fg = ensure_minimum_contrast(fg, bg, MIN_APCA_CONTRAST);
            }

            // US-007: cells inside the selection rect get the precomputed
            // contrast-validated `selection_foreground` (computed at theme-
            // load time against `selection`). This replaces the cell-vs-
            // background contrast we just enforced — selected text needs
            // contrast against the selection quad painted ON TOP of the
            // cell background, not against the cell background itself.
            // Because `fg` is part of `CellStyle` and `BatchAccumulator::
            // can_append` compares CellStyle by equality, this override
            // also breaks batched runs at selection boundaries with no
            // explicit accumulator change.
            //
            // Decorative characters (box-drawing, Powerline separators,
            // block elements) are skipped: their color encodes visual
            // shape (e.g. Powerline arrows transitioning between segment
            // colors), and overriding `fg` to `selection_foreground`
            // would destroy that meaning. Same exclusion as the
            // cell-vs-bg `ensure_minimum_contrast` pass above.
            if let Some(sel) = &selection_range
                && !is_decorative_character(*c)
                && is_cell_in_selection(point, sel, display_offset)
            {
                fg = theme.selection_foreground;
            }

            // Background rect — paint for ALL cells. Default-bg cells use
            // ansi_background (the theme's actual background) to contrast with the
            // slightly darker widget fill, creating visible depth for TUI content.
            let cell_cols = if flags.contains(CellFlags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let cell_bg_color = if is_default_bg { ansi_background } else { bg };
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
            // to eliminate subpixel gaps between adjacent cells (pixel art,
            // Claude Code's banner robot, neofetch ASCII).
            //
            // Multi-quadrant chars (`▙ ▚ ▛ ▜ ▞ ▟`) emit two BlockQuad records
            // per cell — both share the cell's outer boundary array so adjacent
            // cells stay seamless regardless of how many sub-rects they each
            // produce.
            if let Some(coverages) = block_char_coverages(c) {
                batch.flush();
                for &coverage in coverages {
                    block_quads.push(BlockQuad {
                        line: point.line.0,
                        col: point.column.0,
                        num_cols: cell_cols,
                        color: fg,
                        coverage,
                    });
                }
                previous_cell_had_extras = false;
                continue;
            }

            // Build cell style for batching comparison
            let mut font = base_font();
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

        // Build selection highlight rects from the SelectionRange.
        // SelectionRange carries alacritty grid-line coords (scrollback = negative);
        // convert to viewport-line coords to match the cell coordinate system.
        let mut selection_rects = Vec::new();
        if let Some(sel) = &selection_range {
            let start_line = sel.start.line.0 + display_offset as i32;
            let end_line = sel.end.line.0 + display_offset as i32;
            let start_col = sel.start.column.0;
            let end_col = sel.end.column.0;
            let num_cols = desired_cols.max(1);

            if sel.is_block {
                // US-007: block (rectangular) selection — emit one rect per
                // visible line covering only the columns inside the block,
                // matching the rectangular semantics of `is_cell_in_selection`
                // so the bg quad and the fg override agree on which cells
                // are "in" the selection.
                let (l_min, l_max) = if start_line <= end_line {
                    (start_line, end_line)
                } else {
                    (end_line, start_line)
                };
                let (c_min, c_max) = if start_col <= end_col {
                    (start_col, end_col)
                } else {
                    (end_col, start_col)
                };
                let block_cols = c_max.saturating_sub(c_min) + 1;
                let mut line = l_min;
                while line <= l_max {
                    selection_rects.push(LayoutRect {
                        line,
                        num_lines: 1,
                        col: c_min,
                        num_cols: block_cols,
                        color: selection_color,
                    });
                    line += 1;
                }
            } else if start_line == end_line {
                // Single-line linear selection
                selection_rects.push(LayoutRect {
                    line: start_line,
                    num_lines: 1,
                    col: start_col,
                    num_cols: end_col.saturating_sub(start_col) + 1,
                    color: selection_color,
                });
            } else {
                // Multi-line linear: first line from start.col to end of line
                selection_rects.push(LayoutRect {
                    line: start_line,
                    num_lines: 1,
                    col: start_col,
                    num_cols: num_cols.saturating_sub(start_col),
                    color: selection_color,
                });
                // Middle full lines
                let mut line = start_line + 1;
                while line < end_line {
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
                    line: end_line,
                    num_lines: 1,
                    col: 0,
                    num_cols: end_col + 1,
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
            anchor_cursor,
            dimensions: dims,
            background_color,
            scrollbar_thumb: theme.scrollbar_thumb,
            exited: self.exited,
            display_offset,
            history_size,
            desired_cols,
            desired_rows,
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
            font: base_font(),
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
        let font_size = font_size();

        let geom = CellGeometry {
            origin,
            cell_width,
            line_height,
        };

        // PANEFLOW_PIXEL_PROBE: log the per-frame origin once, before any
        // glyph/background record carries it implicitly. Pairs with the
        // `cell_dims` record emitted from `measure_cell()`.
        #[cfg(debug_assertions)]
        pixel_probe::record_origin(origin);

        let base_font = base_font();

        // Clip to element bounds
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // 1. Terminal background + bell-flash overlay
            paint::background::paint_base_fill(&layout, bounds, self.bell_flash_active, window);

            // 2. Per-cell background rects with Ghostty-style edge extension.
            paint::background::paint_cell_backgrounds(&layout, &geom, bounds, window);

            // 2b. Selection highlight
            paint::selection::paint_selection(&layout, &geom, window);

            // 2c. Search match highlight
            paint::overlay::paint_search_highlights(&layout, &geom, window);

            // 2d. Block element quads (pixel-perfect, no font glyph gaps)
            paint::background::paint_block_quads(&layout, &geom, window);

            // 3. Batched text runs
            paint::text::paint_text_runs(&layout, &geom, font_size, window, cx);

            // 3a. PANEFLOW_PIXEL_PROBE_OVERLAY: draw thin red cell borders
            // above the text. Independent of `PANEFLOW_PIXEL_PROBE`; opt-in
            // only. Compiled out in release builds.
            #[cfg(debug_assertions)]
            if pixel_probe::overlay_enabled() {
                paint::overlay::paint_pixel_probe_overlay(&layout, &geom, window);
            }

            // 3b. Hyperlink underline + tooltip (Ctrl+hover)
            paint::overlay::paint_hyperlink_tooltip(self, &layout, &geom, window, cx);

            // 4. Primary cursor
            paint::cursor::paint_cursor(&layout, &geom, &base_font, font_size, window, cx);

            // 4b. Copy-mode selection anchor cursor
            paint::cursor::paint_anchor_cursor(&layout, &geom, window);

            // 5. Scrollbar thumb
            paint::scrollbar::paint_scrollbar(&layout, &geom, bounds, window);

            // 6. IME handler registration + preedit overlay
            let term_for_ime = self.term.clone();
            let view_for_ime = self.terminal_view.clone();
            paint::overlay::paint_ime_preedit(
                self,
                &layout,
                &geom,
                font_size,
                &base_font,
                window,
                cx,
                |cursor_bounds| TerminalInputHandler {
                    terminal_view: view_for_ime,
                    term: term_for_ime,
                    cursor_bounds,
                },
            );

            // 7. Exit overlay
            let exit_fg = rgb_to_hsla(0x6c, 0x70, 0x86); // Overlay6
            paint::overlay::paint_exit_overlay(
                &layout, &geom, bounds, font_size, &base_font, exit_fg, window, cx,
            );
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

// ---------------------------------------------------------------------------
// US-005 fallback — block_char_coverages tests
//
// Discovered via pixel-probe analysis of Claude Code 2.1.119's banner robot:
// the `▐███▌` core uses U+2580..U+2590 (already covered) but the antennas /
// rounded corners use quadrant blocks (`U+2596..U+259F`) which originally
// fell back to font glyphs and rendered with visible vertical gaps. These
// tests lock in coverage for every codepoint added in the US-005 fallback
// extension so a future regression surfaces here instead of as a visual
// artifact reported weeks later.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod block_char_coverage_tests {
    use super::*;

    /// Every original-PRD codepoint must still resolve to a single rect with
    /// the same geometry as before the slice refactor — guards against an
    /// accidental table edit during the US-005 extension.
    #[test]
    fn original_block_chars_are_single_rect() {
        for c in [
            '▀', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█', '▉', '▊', '▋', '▌', '▍', '▎', '▏', '▐',
        ] {
            let rects = block_char_coverages(c)
                .unwrap_or_else(|| panic!("U+{:04X} '{c}' must be covered", c as u32));
            assert_eq!(
                rects.len(),
                1,
                "U+{:04X} '{c}' must emit exactly one rect (got {})",
                c as u32,
                rects.len(),
            );
        }
    }

    /// The full block must cover the entire cell — the canonical sanity check
    /// used by adjacent-block tests in `paint/background.rs`.
    #[test]
    fn full_block_covers_entire_cell() {
        let rects = block_char_coverages('█').expect("█ covered");
        assert_eq!(rects, &[(0.0, 0.0, 1.0, 1.0)]);
    }

    #[test]
    fn upper_one_eighth_block_u2594() {
        // ▔ is the upper-edge complement of ▁ (U+2581 lower 1/8). Same height,
        // anchored at y=0 instead of y=7/8.
        let rects = block_char_coverages('▔').expect("▔ covered");
        assert_eq!(rects.len(), 1);
        let (x, y, w, h) = rects[0];
        assert_eq!((x, y), (0.0, 0.0));
        assert_eq!(w, 1.0);
        assert!((h - 1.0 / 8.0).abs() < 1e-6, "expected h=1/8, got {h}");
    }

    #[test]
    fn single_quadrants_are_one_rect_each() {
        // U+2596..U+2598 + U+259D — the four single-quadrant blocks.
        // Each occupies exactly one corner of the cell, anchored on the grid
        // halfway point, with a 0.5×0.5 extent.
        let cases = [
            ('▖', (0.0, 0.5, 0.5, 0.5)), // lower-left
            ('▗', (0.5, 0.5, 0.5, 0.5)), // lower-right
            ('▘', (0.0, 0.0, 0.5, 0.5)), // upper-left
            ('▝', (0.5, 0.0, 0.5, 0.5)), // upper-right
        ];
        for (c, expected) in cases {
            let rects = block_char_coverages(c).unwrap();
            assert_eq!(rects, &[expected], "U+{:04X} '{c}'", c as u32);
        }
    }

    #[test]
    fn multi_quadrants_emit_two_rects() {
        // The six 3-quadrant + 2-quadrant chars all decompose into two rects.
        for c in ['▙', '▚', '▛', '▜', '▞', '▟'] {
            let rects = block_char_coverages(c).unwrap();
            assert_eq!(
                rects.len(),
                2,
                "U+{:04X} '{c}' must emit 2 rects (got {})",
                c as u32,
                rects.len(),
            );
        }
    }

    #[test]
    fn multi_quadrant_diagonals_have_no_overlap_or_gap() {
        // ▚ (U+259A) and ▞ (U+259E) are the two pure diagonals — opposing
        // quadrants only. Their rects must touch at the cell center but not
        // overlap, otherwise we'd double-paint or leave a sub-pixel hole.
        for c in ['▚', '▞'] {
            let rects = block_char_coverages(c).unwrap();
            // Total coverage area = exactly half the cell (two 0.5×0.5 quads).
            let total_area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
            assert!(
                (total_area - 0.5).abs() < 1e-6,
                "U+{:04X} '{c}' total coverage area = {total_area}, expected 0.5",
                c as u32,
            );
        }
    }

    #[test]
    fn three_quadrant_chars_cover_three_quarters_of_cell() {
        // ▙ ▛ ▜ ▟ each cover exactly 3 of 4 quadrants (= 0.75 of cell area).
        // Even though they emit only 2 rects, the second rect is half-cell-wide
        // (covering 2 quadrants in one go).
        for c in ['▙', '▛', '▜', '▟'] {
            let rects = block_char_coverages(c).unwrap();
            let total_area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
            assert!(
                (total_area - 0.75).abs() < 1e-6,
                "U+{:04X} '{c}' total coverage = {total_area}, expected 0.75",
                c as u32,
            );
        }
    }

    /// The US-005 extension targets exactly the codepoints found in the
    /// `claude` 2.1.119 binary that were *not* in the original table.
    /// If Claude Code (or another TUI) ships a new robot that uses a codepoint
    /// outside this list, the gap will reappear and this test won't catch it —
    /// but the pixel probe will, and the table is one match-arm away from
    /// covering the new char.
    #[test]
    fn us005_claude_code_codepoints_all_covered() {
        for c in [
            '▔', // U+2594 upper 1/8
            '▖', '▗', '▘', '▝', // single quadrants
            '▙', '▚', '▛', '▜', '▞', '▟', // multi quadrants
        ] {
            assert!(
                block_char_coverages(c).is_some(),
                "U+{:04X} '{c}' must be covered to render Claude Code's banner gap-free",
                c as u32,
            );
        }
    }

    /// Codepoints we deliberately *don't* cover — shaded blocks need alpha
    /// (out of scope for this fix), geometric shapes are a different path.
    /// Locks the boundary so a future "extend everything" edit can't sneak
    /// half-broken coverage past review.
    #[test]
    fn shaded_and_geometric_blocks_remain_uncovered() {
        for c in ['░', '▒', '▓', '■', '□', '●', '○'] {
            assert!(
                block_char_coverages(c).is_none(),
                "U+{:04X} '{c}' must NOT be covered (alpha or geometric path)",
                c as u32,
            );
        }
    }
}
