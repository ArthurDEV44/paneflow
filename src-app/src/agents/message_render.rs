// US-014 (prd-agents-view.md): inline markdown + syntect rendering
// for chat messages inside [`super::thread_view::ThreadView`].
//
// Stand-alone from [`crate::markdown`] (the file-viewer pane) on
// purpose: chat layout (user bubble vs full-width assistant body) and
// embedded syntect highlighting are different enough that sharing the
// AST + renderer would force both to compromise. Keeping the two
// paths separate also avoids dragging the file viewer's watcher /
// scroll-persistence machinery into the per-message hot path.
//
// Why pulldown-cmark events directly (not the existing
// [`crate::markdown::MdNode`] AST):
// - We need GFM task lists in chat (PRD AC #3) and the shared AST
//   does not model them yet. Extending the shared AST would touch
//   the markdown file-viewer's render path; out of US-014's scope.
// - Walking events in-place is fewer allocations and the chat
//   renderer never needs to re-render the same content.
//
// Public surface is intentionally narrow: a single
// [`render_message_body`] entry point the ThreadView calls from its
// `render_message` closure. Helpers stay private.

#![allow(dead_code)]

use gpui::{
    AnyElement, ClickEvent, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px, rgb, svg,
};
use markdown::{Markdown, MarkdownElement};
use paneflow_threads::{ContentBlock, MessageRole};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, Theme as SyntectTheme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Threshold above which a single block of plain text is rendered
/// collapsed (AC #11 -- "Tool result message containing a 10KB file
/// dump [...] defaults collapsed showing first 5 lines + expander").
///
/// 5 lines is the PRD copy verbatim; the threshold is also a byte
/// safety net so a single line that wraps to 100 visual rows still
/// gets the collapsible treatment.
pub(crate) const COLLAPSIBLE_LINE_THRESHOLD: usize = 5;
pub(crate) const COLLAPSIBLE_BYTE_THRESHOLD: usize = 1024;

/// Render every [`ContentBlock`] in a message into a GPUI element.
/// User messages are wrapped in a right-aligned rounded card; assistant
/// messages render full-width with zero chrome (AC #1, AC #2).
///
/// `markdown_entity` is the persistent `Markdown` view-model the
/// `ThreadView` keeps parallel to each chat message (assistant + user
/// rows have one; system notes do not). When `Some`, we render via
/// Zed's `MarkdownElement` pipeline for proper GFM + heading +
/// code-block typography. When `None`, we fall back to the in-house
/// renderer below (kept around for system notes + as a safety net).
pub(crate) fn render_message_body(
    role: MessageRole,
    content: &[ContentBlock],
    markdown_entity: Option<Entity<Markdown>>,
    ui: crate::theme::UiColors,
    cwd: Option<std::path::PathBuf>,
) -> AnyElement {
    // US-016 (audit P2-3): lazy `join_text_blocks` -- the markdown-
    // entity path never reads `body_text`, so the alloc was wasted on
    // every visible message per frame. Build it only when we fall
    // through to the in-house renderer.
    match role {
        MessageRole::User => match markdown_entity {
            Some(md) => render_user_bubble_md(md, ui, cwd).into_any_element(),
            None => render_user_bubble(&join_text_blocks(content), ui).into_any_element(),
        },
        MessageRole::Assistant => match markdown_entity {
            Some(md) => render_assistant_body_md(md, ui, cwd).into_any_element(),
            None => render_assistant_body(&join_text_blocks(content), ui).into_any_element(),
        },
        MessageRole::System => render_system_note(&join_text_blocks(content), ui).into_any_element(),
    }
}

/// Zed-pipeline assistant body. Uses the persistent `Markdown` entity
/// so streaming chunks `append` into the parsed AST instead of
/// re-parsing the full source on every paint.
///
/// Body size 12 px + softened off-white text: pure `#ffffff` on a dark
/// background reads as harsh; a slightly muted off-white (close to
/// `#d4d4d4`) lowers the perceived contrast and matches the more
/// compact, refined typography of polished agent-panel renderings.
pub(crate) fn render_assistant_body_md(
    md: Entity<Markdown>,
    ui: crate::theme::UiColors,
    cwd: Option<std::path::PathBuf>,
) -> impl IntoElement {
    let body_color = gpui::rgb(0xc4c4c4).into();
    // Mirror Zed's `MarkdownStyle::themed(MarkdownFont::Agent, ...)`
    // typography at `crates/markdown/src/markdown.rs:188` ->
    // `line_height = buffer_font_size * 1.75`. The 1.75x multiplier
    // is what gives Zed's assistant copy its airy editorial feel; the
    // 1.5x default in `paneflow_markdown_style` is reserved for
    // single-line chunks (tool labels, user-bubble previews) where
    // 1.75x would push rows too tall and re-introduce the row-bleed
    // artefact we just fixed in `inline_tool_call::render_tool_call_label`.
    // Zero outer padding so adjacent Text chunks of the same assistant
    // turn read as one continuous paragraph -- the surrounding
    // `render_assistant_message` wrapper handles `px_5 py_1p5 gap_3`.
    let style =
        super::markdown_style::paneflow_markdown_style_with_line_height(ui, body_color, 14.0, 1.75);
    div().child(MarkdownElement::new(md, style).on_url_click(make_link_handler(cwd)))
}

/// Two-stage cleanup before forwarding a markdown link to `cx.open_url`:
///
/// 1. `xdg-open` (and gio / gnome-open / kde-open / portals) refuses any
///    path with a `:line[:col]` suffix -- a tooling convention (rustc /
///    grep -n / IDE link format) the desktop opener does not parse. Up
///    to two trailing numeric segments are stripped.
/// 2. GPUI's `App::open_url` requires the URL to carry a scheme. LLM
///    output frequently emits bare paths (`[foo](src/foo.rs)` -- no
///    `file://`), so the handler resolves a path-shaped href against
///    the thread's cwd and re-prefixes it as `file://<absolute>`.
///
/// Plain `http(s)://` or `file://` URLs flow through unchanged.
/// (Logs reported the "URI must contain a scheme" failure on
/// 2026-05-26.)
fn make_link_handler(
    cwd: Option<std::path::PathBuf>,
) -> impl Fn(SharedString, &mut gpui::Window, &mut gpui::App) + 'static {
    move |href, _w, cx| {
        // First try the user's configured external editor (Zed / Cursor /
        // Windsurf / VSCode). It receives the path with the `:line[:col]`
        // suffix preserved so the editor jumps to the target position.
        // On `false` (no editor configured, none detected, or spawn
        // failure) we fall through to `cx.open_url` which defers to
        // xdg-open / open / start.
        if !href.contains("://") && super::external_editor::open(&href, cwd.as_deref()) {
            return;
        }
        let target = resolve_link(&href, cwd.as_deref());
        cx.open_url(&target);
    }
}

fn resolve_link(href: &str, cwd: Option<&std::path::Path>) -> String {
    let stripped = strip_line_col_suffix(href).unwrap_or_else(|| href.to_string());
    // Already a URL with a scheme -- forward as-is.
    if stripped.contains("://") {
        return stripped;
    }
    // Bare path. Resolve relative components against the thread cwd
    // (if available) so `xdg-open` can find the file; produce a
    // `file://<absolute>` URI so GPUI's scheme check is satisfied.
    let path = std::path::Path::new(&stripped);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(path)
    } else {
        // No cwd to anchor against -- best effort: forward as-is so
        // `cx.open_url` surfaces the scheme error rather than us
        // silently opening the wrong file.
        return stripped;
    };
    format!("file://{}", abs.display())
}

fn strip_line_col_suffix(href: &str) -> Option<String> {
    // Keep the scheme prefix (`file://`, `https://`, ...) intact so the
    // colon inside it is never mistaken for a `:line` separator.
    let (scheme, path) = match href.find("://") {
        Some(pos) => (&href[..pos + 3], &href[pos + 3..]),
        None => ("", href),
    };
    let mut trim_at: Option<usize> = None;
    let mut idx = path.len();
    for _ in 0..2 {
        let Some(prev_colon) = path[..idx].rfind(':') else {
            break;
        };
        let tail = &path[prev_colon + 1..idx];
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        trim_at = Some(prev_colon);
        idx = prev_colon;
    }
    trim_at.map(|p| format!("{scheme}{}", &path[..p]))
}

/// Zed-pipeline user bubble. Ported from
/// `agent_ui/src/conversation_view/thread_view.rs` (US-007 in
/// `prd-agent-ui-visual-parity-2026-Q3.md`): full-width container with
/// `pt_2/pb_3/px_2/gap_1p5`, inner bordered card with `editor_background`
/// (mapped to `ui.base`), `shadow_md`, and a hover that nudges the
/// border toward `focus_border` (mapped to `ui.accent`).
fn render_user_bubble_md(
    md: Entity<Markdown>,
    ui: crate::theme::UiColors,
    cwd: Option<std::path::PathBuf>,
) -> impl IntoElement {
    let body_color = gpui::rgb(0xc4c4c4).into();
    let style = super::markdown_style::paneflow_markdown_style(ui, body_color, 12.0);
    div()
        .pt(px(8.))
        .pb(px(12.))
        .px(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(
            div()
                .py(px(12.))
                .px(px(8.))
                .rounded(px(6.))
                .bg(ui.base)
                .border_1()
                .border_color(ui.border)
                .shadow_md()
                .hover(|s| s.border_color(ui.accent.alpha(0.8)))
                .text_size(px(12.))
                .text_color(body_color)
                .child(MarkdownElement::new(md, style).on_url_click(make_link_handler(cwd))),
        )
}

/// User messages (plain-text fallback). Same Zed-shape outer + inner
/// card as the markdown variant — so streaming and plain notes don't
/// diverge visually.
pub(crate) fn render_user_bubble(text: &str, ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .pt(px(8.))
        .pb(px(12.))
        .px(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(
            div()
                .py(px(12.))
                .px(px(8.))
                .rounded(px(6.))
                .bg(ui.base)
                .border_1()
                .border_color(ui.border)
                .shadow_md()
                .hover(|s| s.border_color(ui.accent.alpha(0.8)))
                .text_size(px(12.))
                .text_color(ui.text)
                .child(render_markdown(text, ui, MessageRole::User)),
        )
}

/// Assistant messages: full-width, no avatar, no header.
fn render_assistant_body(text: &str, ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .px(px(16.))
        .py(px(6.))
        .text_color(ui.text)
        .text_size(px(13.))
        .child(render_markdown(text, ui, MessageRole::Assistant))
}

fn render_system_note(text: &str, ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .px(px(16.))
        .py(px(6.))
        .text_color(ui.muted)
        .text_size(px(12.))
        .italic()
        .child(text.to_string())
}

/// Collapse every [`ContentBlock::Text`] into one string. Non-text
/// blocks emit placeholder summaries for now -- US-019 (image attach)
/// and US-017 (tool result cards) will render those inline.
fn join_text_blocks(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t.text);
            }
            ContentBlock::Image(_) => out.push_str("\n[image]"),
            ContentBlock::Audio(_) => out.push_str("\n[audio]"),
            ContentBlock::ResourceLink(_) => out.push_str("\n[resource link]"),
            ContentBlock::Resource(_) => out.push_str("\n[resource]"),
            _ => out.push_str("\n[unknown content block]"),
        }
    }
    out
}

// ----- Markdown walker -----------------------------------------------------

/// Parse `text` with pulldown-cmark (GFM extensions enabled per AC #3)
/// and emit a GPUI element tree.
fn render_markdown(text: &str, ui: crate::theme::UiColors, role: MessageRole) -> AnyElement {
    // AC #11 collapsible: when the entire message looks like a plain
    // text dump (no fenced blocks, no markdown structure) and exceeds
    // the line threshold, render it as a collapsible code-like block
    // rather than as a wall of `<p>`. The heuristic is simple on
    // purpose (PRD AC focuses on "Tool result" messages, which arrive
    // verbatim, not as marked-up prose).
    if role == MessageRole::Assistant && looks_like_plain_dump(text) {
        return render_collapsible_dump(text, ui).into_any_element();
    }

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, opts);
    let mut walker = Walker::new(ui);
    for event in parser {
        walker.event(event);
    }
    walker.finish()
}

/// pulldown-cmark event walker. Maintains a stack of "in progress"
/// blocks; each `End` pops one and installs the rendered element in
/// its parent (or in `root` when the stack is empty).
struct Walker {
    ui: crate::theme::UiColors,
    /// Accumulated text + style for the inline run currently being
    /// built. Flushed into the containing block on every block close
    /// and on every block start.
    inline_buffer: Vec<InlineRun>,
    /// Current style flags for incoming text events.
    style_stack: Vec<InlineStyle>,
    /// `Some(url)` while we are inside `Tag::Link`. Spans collected
    /// during this period are emitted as clickable link runs.
    link_url: Option<String>,
    /// Active code-block accumulator. `Some` while inside `Tag::CodeBlock`.
    code_block: Option<CodeBlockBuffer>,
    /// Active table accumulator. `Some` while inside `Tag::Table`.
    table: Option<TableBuffer>,
    /// Block stack. The top-most variant carries the in-progress
    /// children of the currently-open block.
    block_stack: Vec<BlockFrame>,
    /// Finished top-level elements. Whatever survives once the
    /// walker is done is wrapped into the message body.
    root: Vec<AnyElement>,
}

enum BlockFrame {
    Paragraph,
    Heading(HeadingLevel),
    BlockQuote(Vec<AnyElement>),
    List {
        ordered_start: Option<u64>,
        items: Vec<Vec<AnyElement>>,
    },
    Item(Vec<AnyElement>),
    TaskListItem {
        checked: bool,
        children: Vec<AnyElement>,
    },
    Footnote {
        label: String,
        children: Vec<AnyElement>,
    },
}

#[derive(Clone, Copy, Default)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
}

#[derive(Clone)]
struct InlineRun {
    text: String,
    style: InlineStyle,
    link: Option<String>,
}

struct CodeBlockBuffer {
    lang: Option<String>,
    text: String,
}

struct TableBuffer {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<InlineRun>>>,
    /// `Some` while accumulating a cell.
    current_cell: Option<Vec<InlineRun>>,
    /// `Some(true)` if we are inside the header row.
    in_header: bool,
}

impl Walker {
    fn new(ui: crate::theme::UiColors) -> Self {
        Self {
            ui,
            inline_buffer: Vec::new(),
            style_stack: Vec::new(),
            link_url: None,
            code_block: None,
            table: None,
            block_stack: Vec::new(),
            root: Vec::new(),
        }
    }

    fn current_style(&self) -> InlineStyle {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, mutate: impl FnOnce(&mut InlineStyle)) {
        let mut next = self.current_style();
        mutate(&mut next);
        self.style_stack.push(next);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Inside a code block: accumulate raw text.
        if let Some(buf) = self.code_block.as_mut() {
            buf.text.push_str(text);
            return;
        }
        let style = self.current_style();
        let link = self.link_url.clone();
        // Inside a table cell: accumulate inline runs there.
        if let Some(table) = self.table.as_mut()
            && let Some(cell) = table.current_cell.as_mut()
        {
            push_run(cell, text, style, link);
            return;
        }
        push_run(&mut self.inline_buffer, text, style, link);
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.push_text(&t),
            Event::Code(c) => {
                let mut style = self.current_style();
                style.code = true;
                let link = self.link_url.clone();
                push_run(&mut self.inline_buffer, &c, style, link);
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => {
                self.flush_inline_paragraph();
                let ui = self.ui;
                self.emit(div().my(px(8.)).h(px(1.)).bg(ui.border).into_any_element());
            }
            Event::TaskListMarker(checked) => {
                // pulldown-cmark emits this at the start of a task
                // list item, before any text. Replace the topmost
                // `BlockFrame::Item` with a `TaskListItem` marker so
                // the list emission knows to render a checkbox.
                if let Some(BlockFrame::Item(_)) = self.block_stack.last() {
                    let frame = self.block_stack.pop();
                    if let Some(BlockFrame::Item(children)) = frame {
                        self.block_stack
                            .push(BlockFrame::TaskListItem { checked, children });
                    }
                }
            }
            Event::FootnoteReference(label) => {
                let label_str = label.into_string();
                self.push_text(&format!("[^{label_str}]"));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                // Strip HTML by emitting the raw bytes as plain text.
                // Chat output rarely contains raw HTML; rendering it as
                // text is safer than parsing.
                self.push_text(&html);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.block_stack.push(BlockFrame::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.block_stack.push(BlockFrame::Heading(level));
            }
            Tag::BlockQuote => {
                self.block_stack.push(BlockFrame::BlockQuote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(s) if !s.is_empty() => Some(s.into_string()),
                    _ => None,
                };
                self.code_block = Some(CodeBlockBuffer {
                    lang,
                    text: String::new(),
                });
            }
            Tag::List(start_num) => {
                self.block_stack.push(BlockFrame::List {
                    ordered_start: start_num,
                    items: Vec::new(),
                });
            }
            Tag::Item => {
                self.block_stack.push(BlockFrame::Item(Vec::new()));
            }
            Tag::Emphasis => self.push_style(|s| s.emphasis = true),
            Tag::Strong => self.push_style(|s| s.strong = true),
            Tag::Strikethrough => self.push_style(|s| s.strike = true),
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.into_string());
            }
            Tag::Table(alignments) => {
                self.table = Some(TableBuffer {
                    alignments,
                    rows: Vec::new(),
                    current_cell: None,
                    in_header: false,
                });
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_header = true;
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut()
                    && !t.in_header
                {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some(t) = self.table.as_mut() {
                    t.current_cell = Some(Vec::new());
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.block_stack.push(BlockFrame::Footnote {
                    label: label.into_string(),
                    children: Vec::new(),
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let spans = std::mem::take(&mut self.inline_buffer);
                let ui = self.ui;
                let el = div().my(px(4.)).child(spans_to_element(spans, ui));
                self.emit(el.into_any_element());
                // Pop frame.
                self.block_stack.pop();
            }
            TagEnd::Heading(_) => {
                let level = match self.block_stack.last() {
                    Some(BlockFrame::Heading(l)) => *l,
                    _ => HeadingLevel::H3,
                };
                let spans = std::mem::take(&mut self.inline_buffer);
                let ui = self.ui;
                let el = heading_element(level, spans, ui);
                self.emit(el);
                self.block_stack.pop();
            }
            TagEnd::BlockQuote => {
                let frame = self.block_stack.pop();
                let ui = self.ui;
                if let Some(BlockFrame::BlockQuote(children)) = frame {
                    let mut body = div().flex().flex_col().gap(px(4.));
                    for c in children {
                        body = body.child(c);
                    }
                    self.emit(
                        div()
                            .my(px(6.))
                            .pl(px(10.))
                            .border_l_2()
                            .border_color(ui.border)
                            .text_color(ui.muted)
                            .child(body)
                            .into_any_element(),
                    );
                }
            }
            TagEnd::CodeBlock => {
                if let Some(buf) = self.code_block.take() {
                    let ui = self.ui;
                    self.emit(render_code_block(&buf, ui).into_any_element());
                }
            }
            TagEnd::List(_) => {
                let frame = self.block_stack.pop();
                let ui = self.ui;
                if let Some(BlockFrame::List {
                    ordered_start,
                    items,
                }) = frame
                {
                    self.emit(render_list(ordered_start, items, ui).into_any_element());
                }
            }
            TagEnd::Item => {
                let frame = self.block_stack.pop();
                if let Some(BlockFrame::Item(children)) = frame {
                    if let Some(BlockFrame::List { items, .. }) = self.block_stack.last_mut() {
                        items.push(children);
                    }
                } else if let Some(BlockFrame::TaskListItem { checked, children }) = frame {
                    // Wrap the children in a row prefixed by a checkbox
                    // icon so the parent List frame still receives a
                    // standard `Vec<AnyElement>` per item.
                    let mut row = div().flex().flex_row().items_start().gap(px(6.));
                    row = row.child(task_checkbox(checked, self.ui));
                    let mut body = div().flex().flex_col().gap(px(2.));
                    for c in children {
                        body = body.child(c);
                    }
                    row = row.child(body);
                    if let Some(BlockFrame::List { items, .. }) = self.block_stack.last_mut() {
                        items.push(vec![row.into_any_element()]);
                    }
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.link_url = None;
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let ui = self.ui;
                    self.emit(render_table(table, ui).into_any_element());
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_header = false;
                }
            }
            TagEnd::TableRow => {}
            TagEnd::TableCell => {
                if let Some(t) = self.table.as_mut()
                    && let Some(cell) = t.current_cell.take()
                    && let Some(last_row) = t.rows.last_mut()
                {
                    last_row.push(cell);
                }
            }
            TagEnd::FootnoteDefinition => {
                let frame = self.block_stack.pop();
                let ui = self.ui;
                if let Some(BlockFrame::Footnote { label, children }) = frame {
                    let mut body = div().flex().flex_col().gap(px(2.));
                    for c in children {
                        body = body.child(c);
                    }
                    self.emit(
                        div()
                            .my(px(4.))
                            .pl(px(8.))
                            .text_size(px(11.))
                            .text_color(ui.muted)
                            .child(format!("[^{label}]"))
                            .child(body)
                            .into_any_element(),
                    );
                }
            }
            _ => {}
        }
    }

    fn flush_inline_paragraph(&mut self) {
        if self.inline_buffer.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline_buffer);
        let ui = self.ui;
        self.emit(spans_to_element(spans, ui).into_any_element());
    }

    fn emit(&mut self, el: AnyElement) {
        // Install element in the innermost block-with-children
        // (BlockQuote / Item / Footnote / TaskListItem). Otherwise
        // append to the root.
        for frame in self.block_stack.iter_mut().rev() {
            match frame {
                BlockFrame::BlockQuote(children)
                | BlockFrame::Item(children)
                | BlockFrame::TaskListItem { children, .. }
                | BlockFrame::Footnote { children, .. } => {
                    children.push(el);
                    return;
                }
                _ => {}
            }
        }
        self.root.push(el);
    }

    fn finish(mut self) -> AnyElement {
        // Flush any trailing inline run as a final paragraph.
        self.flush_inline_paragraph();
        let mut out = div().flex().flex_col().gap(px(2.));
        for c in self.root {
            out = out.child(c);
        }
        out.into_any_element()
    }
}

fn push_run(target: &mut Vec<InlineRun>, text: &str, style: InlineStyle, link: Option<String>) {
    // Coalesce adjacent identically-styled runs to keep the element
    // count low.
    if let Some(last) = target.last_mut()
        && last.style == style
        && last.link == link
    {
        last.text.push_str(text);
        return;
    }
    target.push(InlineRun {
        text: text.to_string(),
        style,
        link,
    });
}

impl PartialEq for InlineStyle {
    fn eq(&self, other: &Self) -> bool {
        self.strong == other.strong
            && self.emphasis == other.emphasis
            && self.strike == other.strike
            && self.code == other.code
    }
}

// ----- Inline run rendering ------------------------------------------------

fn spans_to_element(runs: Vec<InlineRun>, ui: crate::theme::UiColors) -> impl IntoElement {
    let mut row = div().flex().flex_row().flex_wrap().items_baseline();
    for run in runs {
        row = row.child(render_inline_run(run, ui));
    }
    row
}

fn render_inline_run(run: InlineRun, ui: crate::theme::UiColors) -> AnyElement {
    // Apply style flags to a plain `Div`. Link-handling needs a
    // stateful element (id + on_click), so we branch at the end.
    let mut el = div().child(run.text.clone());
    if run.style.strong {
        el = el.font_weight(FontWeight::BOLD);
    }
    if run.style.emphasis {
        el = el.italic();
    }
    if run.style.strike {
        el = el.line_through();
    }
    if run.style.code {
        // Inline code: monospace, subtle background. AC #5.
        el = el
            .bg(ui.subtle)
            .text_color(ui.text)
            .px(px(4.))
            .py(px(0.))
            .rounded(px(3.))
            .font_family("monospace");
    } else if run.link.is_some() {
        el = el.text_color(ui.accent).underline();
    } else {
        el = el.text_color(ui.text);
    }
    match run.link {
        Some(url) => {
            // AC #6: links in ui.accent, clickable, open in default
            // browser via `open::that`. Restrict to http(s) so a crafted
            // assistant message cannot launch arbitrary URI handlers
            // (javascript:, file:, data:, etc.) via the OS opener.
            let id: SharedString = format!("md-link-{}", djb2(&url)).into();
            el.id(id)
                .cursor_pointer()
                .on_click(move |_e: &ClickEvent, _w, _cx| {
                    if is_safe_link_scheme(&url) {
                        let _ = open::that(&url);
                    } else {
                        tracing::warn!(
                            target: "paneflow::agents::message_render",
                            url = %url,
                            "blocked open::that for non-http(s) URL scheme",
                        );
                    }
                })
                .into_any_element()
        }
        None => el.into_any_element(),
    }
}

/// Allow only `http://` and `https://` URLs to reach `open::that`. Other
/// schemes (javascript:, file:, data:, vbscript:, mailto:, etc.) are
/// blocked because assistant output is partially-trusted and the OS-level
/// URL opener can invoke arbitrary registered handlers.
fn is_safe_link_scheme(url: &str) -> bool {
    let lower = url.trim_start();
    let lower_lc = lower.to_ascii_lowercase();
    lower_lc.starts_with("https://") || lower_lc.starts_with("http://")
}

fn djb2(s: &str) -> u64 {
    let mut hash: u64 = 5381;
    for b in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u64);
    }
    hash
}

// ----- Block element builders ----------------------------------------------

fn heading_element(
    level: HeadingLevel,
    spans: Vec<InlineRun>,
    ui: crate::theme::UiColors,
) -> AnyElement {
    let size = match level {
        HeadingLevel::H1 => 18.,
        HeadingLevel::H2 => 16.,
        HeadingLevel::H3 => 15.,
        HeadingLevel::H4 => 14.,
        HeadingLevel::H5 | HeadingLevel::H6 => 13.,
    };
    div()
        .my(px(6.))
        .text_color(ui.text)
        .text_size(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .child(spans_to_element(spans, ui))
        .into_any_element()
}

fn render_list(
    ordered_start: Option<u64>,
    items: Vec<Vec<AnyElement>>,
    ui: crate::theme::UiColors,
) -> impl IntoElement {
    let mut list = div().my(px(4.)).flex().flex_col().gap(px(2.));
    let mut counter = ordered_start.unwrap_or(0);
    for item in items {
        let marker_text = if let Some(start) = ordered_start {
            let n = if counter == 0 { start } else { counter };
            counter = n + 1;
            format!("{n}.")
        } else {
            "•".to_string()
        };
        let mut row = div().flex().flex_row().items_start().gap(px(6.));
        row = row.child(
            div()
                .flex_none()
                .min_w(px(16.))
                .text_size(px(12.))
                .text_color(ui.muted)
                .child(marker_text),
        );
        let mut body = div().flex_1().flex().flex_col().gap(px(2.));
        for c in item {
            body = body.child(c);
        }
        row = row.child(body);
        list = list.child(row);
    }
    list
}

fn task_checkbox(checked: bool, ui: crate::theme::UiColors) -> impl IntoElement {
    let inner_color = if checked { ui.accent } else { ui.muted };
    let mut box_div = div()
        .flex_none()
        .w(px(12.))
        .h(px(12.))
        .mt(px(2.))
        .rounded(px(2.))
        .border_1()
        .border_color(ui.border);
    if checked {
        box_div = box_div.child(
            svg()
                .size(px(10.))
                .path("icons/checks.svg")
                .text_color(inner_color),
        );
    }
    box_div
}

// ----- Code block rendering (syntect) --------------------------------------

fn render_code_block(buf: &CodeBlockBuffer, ui: crate::theme::UiColors) -> impl IntoElement {
    let highlighted = highlight_code(&buf.text, buf.lang.as_deref());
    let mut block = div()
        .id(SharedString::from(format!("md-code-{}", djb2(&buf.text))))
        .my(px(6.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(ui.subtle)
        .border_1()
        .border_color(ui.border)
        // AC #10: long single lines stay horizontally scrollable in
        // their own container, NOT in the message list.
        .overflow_x_scroll()
        .font_family("monospace")
        .text_size(px(12.))
        .flex()
        .flex_col();
    if let Some(lang) = &buf.lang {
        // Tiny language badge in the top-right corner.
        block = block.child(
            div()
                .text_size(px(10.))
                .text_color(ui.muted)
                .pb(px(4.))
                .child(lang.clone()),
        );
    }
    match highlighted {
        Some(lines) => {
            for line in lines {
                let mut line_div = div().flex().flex_row();
                for (style, text) in line {
                    line_div = line_div.child(
                        div()
                            .text_color(syntect_color(style))
                            .child(text.replace('\n', "")),
                    );
                }
                block = block.child(line_div);
            }
        }
        None => {
            // Unhappy path (AC #9): unrecognised language or syntect
            // failure -> plain monospace fallback.
            block = block.child(div().text_color(ui.text).child(buf.text.clone()));
        }
    }
    block
}

/// Run syntect on the given source. Returns `None` when the language
/// is unrecognised OR when highlighting fails for any reason (AC #9
/// "render as plain monospace without crashing"). The outer `Vec` is
/// one entry per source line.
type HighlightedLine = Vec<(SyntectStyle, String)>;
fn highlight_code(source: &str, lang: Option<&str>) -> Option<Vec<HighlightedLine>> {
    let syntax_set = global_syntax_set();
    let theme = global_theme();

    let syntax = lang
        .and_then(|name| {
            syntax_set
                .find_syntax_by_token(name)
                .or_else(|| syntax_set.find_syntax_by_name(name))
        })
        .or_else(|| syntax_set.find_syntax_by_first_line(source))?;

    let mut hl = HighlightLines::new(syntax, theme);
    let mut out = Vec::new();
    for line in LinesWithEndings::from(source) {
        let regions = hl.highlight_line(line, syntax_set).ok()?;
        out.push(
            regions
                .into_iter()
                .map(|(s, t)| (s, t.to_string()))
                .collect(),
        );
    }
    Some(out)
}

fn global_syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn global_theme() -> &'static SyntectTheme {
    static THEME: OnceLock<SyntectTheme> = OnceLock::new();
    THEME.get_or_init(|| {
        let set = ThemeSet::load_defaults();
        set.themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_else(|| {
                set.themes
                    .values()
                    .next()
                    .cloned()
                    .expect("syntect ships at least one default theme")
            })
    })
}

fn syntect_color(style: SyntectStyle) -> gpui::Rgba {
    let c = style.foreground;
    rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32))
}

// ----- Table rendering -----------------------------------------------------

fn render_table(buf: TableBuffer, ui: crate::theme::UiColors) -> impl IntoElement {
    let mut table = div()
        .id("md-table")
        .my(px(6.))
        .border_1()
        .border_color(ui.border)
        .rounded(px(4.))
        .overflow_x_scroll()
        .flex()
        .flex_col();
    for (i, row) in buf.rows.iter().enumerate() {
        let is_header = i == 0;
        let is_odd = i % 2 == 1;
        let mut row_div = div().flex().flex_row();
        if is_header {
            row_div = row_div.bg(ui.subtle).font_weight(FontWeight::SEMIBOLD);
        } else if is_odd {
            // Alternating background for readability (AC #7).
            row_div = row_div.bg(ui.surface);
        }
        for (j, cell) in row.iter().enumerate() {
            let mut cell_div = div()
                .px(px(8.))
                .py(px(4.))
                .border_r_1()
                .border_color(ui.border)
                .text_size(px(12.));
            if j + 1 == row.len() {
                cell_div = cell_div.border_r_0();
            }
            cell_div = cell_div.child(spans_to_element(cell.clone(), ui));
            row_div = row_div.child(cell_div);
        }
        if !is_header && i + 1 < buf.rows.len() {
            row_div = row_div.border_b_1().border_color(ui.border);
        }
        table = table.child(row_div);
    }
    let _ = buf.alignments; // alignment hints reserved for follow-up
    table
}

// ----- Plain-text dump collapsible (AC #11) --------------------------------

/// Heuristic: a "dump" is multi-line plain text with no fence and no
/// markdown structure markers. Cheap to detect with a small scan; we
/// only invoke it for assistant messages where the AC applies.
fn looks_like_plain_dump(text: &str) -> bool {
    if text.len() < COLLAPSIBLE_BYTE_THRESHOLD {
        return false;
    }
    let lines = text.lines().count();
    if lines < COLLAPSIBLE_LINE_THRESHOLD {
        return false;
    }
    // Reject anything that has obvious markdown structure -- those
    // get the full event walker path.
    let has_markdown_structure = text.contains("```")
        || text.contains("\n# ")
        || text.contains("\n## ")
        || text.contains("\n- ")
        || text.contains("\n* ")
        || text.contains("\n| ")
        || text.starts_with("# ")
        || text.starts_with("- ")
        || text.starts_with("* ");
    !has_markdown_structure
}

fn render_collapsible_dump(text: &str, ui: crate::theme::UiColors) -> impl IntoElement {
    // Show first 5 lines + an "expand" pseudo-button. For US-014 we
    // ship the visual (the AC's collapsed-by-default state); fully
    // toggling open/close requires per-message UI state, which is
    // owned by US-017 (tool call cards). The expander row here is
    // the placeholder hook -- it stays visually present so a future
    // story can wire the click.
    let preview = text
        .lines()
        .take(COLLAPSIBLE_LINE_THRESHOLD)
        .collect::<Vec<_>>()
        .join("\n");
    let remaining = text
        .lines()
        .count()
        .saturating_sub(COLLAPSIBLE_LINE_THRESHOLD);

    div()
        .my(px(6.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(ui.subtle)
        .border_1()
        .border_color(ui.border)
        .font_family("monospace")
        .text_size(px(12.))
        .text_color(ui.text)
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .id("md-dump-preview")
                .overflow_x_scroll()
                .child(preview),
        )
        .when(remaining > 0, |d| {
            d.child(
                div()
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .child(format!("... {remaining} more lines")),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_code_returns_none_for_unknown_language() {
        // AC #9: unknown fence -> plain monospace fallback (None
        // signals the caller to skip syntect entirely).
        assert!(highlight_code("hello", Some("madeup-lang-xyzzy")).is_none());
    }

    #[test]
    fn highlight_code_succeeds_for_known_language() {
        // AC #8 manual verification anchor: bash keywords highlight.
        let result = highlight_code("echo hi\nif true; then\n  echo ok\nfi\n", Some("bash"));
        assert!(result.is_some(), "syntect should highlight bash code");
        let lines = result.unwrap();
        assert!(lines.len() >= 3, "got {} lines", lines.len());
    }

    #[test]
    fn looks_like_plain_dump_detects_long_unstructured_text() {
        // A 10KB file dump with 100 short lines -- the AC's canonical
        // "Tool result message containing a 10KB file dump" case.
        let mut dump = String::new();
        for i in 0..100 {
            dump.push_str(&format!("line {i}: some content here that fills a row\n"));
        }
        assert!(dump.len() > COLLAPSIBLE_BYTE_THRESHOLD);
        assert!(
            looks_like_plain_dump(&dump),
            "10KB plain-text dump must be detected as collapsible"
        );
    }

    #[test]
    fn looks_like_plain_dump_skips_markdown_structured_text() {
        let md = "# Heading\n\n- item\n- item\n\n```\nfoo\n```\n".repeat(50);
        assert!(!looks_like_plain_dump(&md));
    }

    #[test]
    fn looks_like_plain_dump_skips_short_input() {
        let short = "one\ntwo\n";
        assert!(!looks_like_plain_dump(short));
    }

    #[test]
    fn djb2_is_stable_and_distinct() {
        assert_eq!(djb2("hello"), djb2("hello"));
        assert_ne!(djb2("hello"), djb2("world"));
    }
}
