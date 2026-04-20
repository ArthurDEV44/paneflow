//! Cell ↔ pixel conversion state.
//!
//! `CellGeometry` bundles the three scalars that every paint pass needs to
//! map grid coordinates to window pixels: the gutter-adjusted origin, the
//! cell width, and the line height. Passing it around avoids copy-paste of
//! three arguments per call site.
//!
//! Extracted from `terminal_element.rs` per US-015 of the src-app refactor PRD.

use gpui::{Pixels, Point};

/// Pixel-space geometry for a single terminal grid, shared across paint passes.
#[derive(Clone, Copy)]
pub(super) struct CellGeometry {
    /// Top-left corner of the usable grid in window coordinates (includes the
    /// 1-cell left gutter offset applied in `paint()`).
    pub origin: Point<Pixels>,
    pub cell_width: Pixels,
    pub line_height: Pixels,
}
