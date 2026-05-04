//! Markdown viewer pane (US-020 — prd-cmux-port-2026-q2.md, EP-006).
//!
//! Three-layer split:
//! - `parser` — pulldown-cmark event walker → owned `MdNode` AST. Pure Rust,
//!   no GPUI deps; unit-tested in isolation.
//! - `theme`  — semantic palette derived from the active `TerminalTheme`. The
//!   markdown viewer never owns colors; it borrows the terminal palette so a
//!   user theme switch repaints everything consistently.
//! - `view`   — `MarkdownView: Render` GPUI entity that walks the AST and
//!   emits a nested `div` element tree.
//!
//! Out of scope here: live reload (US-021), scroll-state persistence (US-022),
//! syntax highlighting (P2 follow-up via `syntect`).

mod parser;
mod state;
mod theme;
mod view;

// Public surface: only `MarkdownView` is consumed outside this module today,
// but the parser primitives are re-exported for upcoming stories (US-021
// live reload re-runs `parse_with_limit`; US-022 walks `MdNode` for search).
#[allow(unused_imports)]
pub use parser::{MAX_INPUT_BYTES, MdNode, ParseError, Span, SpanStyle, parse_with_limit};
pub use view::MarkdownView;
