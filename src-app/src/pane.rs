//! Pane — a tabbed container holding one or more views (terminals or markdown
//! viewers, freely mixed within the same tab strip).
//!
//! Each leaf in the split tree holds an `Entity<Pane>`. A Pane manages an
//! ordered list of [`TabContent`] tabs and a single `selected_idx` cursor.
//! Markdown tabs and terminal tabs share the strip — the user opens markdown
//! files by clicking the doc icon (or Cmd/Ctrl-clicking a `.md` path inside a
//! terminal), and a new tab is appended to the same pane rather than splitting.
//!
//! Communication with the parent (split tree owner) uses the Zed pattern:
//! Pane emits `PaneEvent` via `cx.emit()`, parent subscribes via `cx.subscribe()`.
//!
//! Tab bar UI is modeled after Zed's tab bar design.

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, Pixels, Point, Render, SharedString, Styled, Window, div,
    prelude::*, px, rgb, svg,
};
use paneflow_config::schema::ButtonCommand;

use crate::markdown::MarkdownView;
use crate::terminal::{TerminalEvent, TerminalView};

// ---------------------------------------------------------------------------
// TabContent — a tab can hold either a terminal or a markdown viewer
// ---------------------------------------------------------------------------

/// A single tab inside a pane. Terminal and markdown tabs share the strip so
/// the user keeps tab navigation (Ctrl+Tab, click) regardless of content type
/// — opening a markdown file from a terminal pane appends a tab next to the
/// existing terminals rather than splitting the layout.
#[derive(Clone)]
pub enum TabContent {
    Terminal(Entity<TerminalView>),
    Markdown(Entity<MarkdownView>),
}

impl TabContent {
    pub fn as_terminal(&self) -> Option<&Entity<TerminalView>> {
        match self {
            TabContent::Terminal(t) => Some(t),
            TabContent::Markdown(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tab bar color helpers — derived from active theme
// ---------------------------------------------------------------------------

fn tab_colors() -> crate::theme::UiColors {
    crate::theme::ui_colors()
}
/// Tab bar total height (matches Zed's 32px at default density)
const TAB_BAR_HEIGHT: f32 = 32.0;
/// Inner content height (bar height minus 1px bottom border compensation)
const TAB_CONTENT_HEIGHT: f32 = 31.0;
/// Horizontal padding inside each tab
const TAB_PX: f32 = 12.0;
/// Gap between tab children (icon, label, close button)
const TAB_GAP: f32 = 6.0;
/// Max tab width — longer labels get truncated with ellipsis
const TAB_MAX_WIDTH: f32 = 200.0;
/// Close button container size (matches Zed's end_slot: 14×14)
const CLOSE_SIZE: f32 = 14.0;
/// Section padding (start/end areas)
const SECTION_PX: f32 = 6.0;
/// Hard upper bound on tab title length in characters. Mirrors Zed's
/// `MAX_TAB_TITLE_LEN` (`zed/crates/editor/src/items.rs:64`). Anything past
/// this is replaced with a trailing ellipsis so the tab chip stays inside
/// `TAB_MAX_WIDTH` even when the flex layout's `max_w(...)` constraint
/// fails to propagate (a known quirk when there's no explicit `w(...)`
/// on the parent and the child has `whitespace_nowrap`).
const MAX_TAB_TITLE_LEN: usize = 24;

/// Char-boundary-safe `truncate_and_trailoff`. Counts chars (not bytes) so
/// filenames with multibyte UTF-8 (accents, CJK, emoji) don't trigger a
/// byte-index panic, and reserves one char for the trailing `…`.
fn truncate_tab_title(raw: &str) -> String {
    if raw.chars().count() <= MAX_TAB_TITLE_LEN {
        return raw.to_string();
    }
    let head: String = raw.chars().take(MAX_TAB_TITLE_LEN - 1).collect();
    format!("{head}…")
}

// ---------------------------------------------------------------------------
// Pane events — emitted to parent via cx.emit()
// ---------------------------------------------------------------------------

pub enum PaneEvent {
    /// The last tab was closed — parent should remove this pane from the split tree.
    Remove,
    /// Request a split in the given direction from this pane.
    Split(crate::layout::SplitDirection),
    /// Open the Claude Code sessions menu for the active terminal's cwd.
    /// Carries the click anchor (window-space) so the popover can be
    /// positioned by the parent renderer (popovers must paint at the
    /// `PaneFlowApp` layer; a `deferred()` inside `Pane::render` is clipped
    /// to the pane's bbox).
    OpenClaudeSessions(Point<Pixels>),
}

// ---------------------------------------------------------------------------
// Pane — tabbed terminal container
// ---------------------------------------------------------------------------

pub struct Pane {
    pub tabs: Vec<TabContent>,
    pub selected_idx: usize,
    /// Set to true when the workspace is zoomed on this pane.
    pub zoomed: bool,
    /// Workspace ID for spawning new terminals with correct env vars.
    pub workspace_id: u64,
    /// Workspace-specific command buttons rendered in the tab bar after the
    /// built-in defaults. Populated/updated by `Workspace::propagate_custom_buttons`.
    pub custom_buttons: Vec<ButtonCommand>,
}

impl EventEmitter<PaneEvent> for Pane {}

impl Pane {
    /// Create a new pane with a single terminal tab.
    pub fn new(terminal: Entity<TerminalView>, workspace_id: u64, cx: &mut Context<Self>) -> Self {
        Self::subscribe_terminal(&terminal, cx);
        Self {
            tabs: vec![TabContent::Terminal(terminal)],
            selected_idx: 0,
            zoomed: false,
            workspace_id,
            custom_buttons: Vec::new(),
        }
    }

    /// Iterate over the terminal entities in this pane. Markdown tabs are
    /// skipped. Used by event handlers that need to scan terminals — sidebar
    /// counters, AI-tool PID owner lookups, layout serialization.
    pub fn terminals(&self) -> impl Iterator<Item = &Entity<TerminalView>> {
        self.tabs.iter().filter_map(TabContent::as_terminal)
    }

    /// True when `terminal` is one of this pane's tabs.
    pub fn contains_terminal(&self, terminal: &Entity<TerminalView>) -> bool {
        self.terminals().any(|t| t == terminal)
    }

    /// Append a new terminal tab and focus it.
    pub fn add_tab(&mut self, terminal: Entity<TerminalView>, cx: &mut Context<Self>) {
        Self::subscribe_terminal(&terminal, cx);
        self.tabs.push(TabContent::Terminal(terminal));
        self.selected_idx = self.tabs.len() - 1;
    }

    /// Append a markdown viewer tab and focus it. Used by the doc-button
    /// handler in this pane's tab strip and by the Cmd/Ctrl-click flow on
    /// `.md` paths inside a terminal — both routes converge on this method
    /// via `PaneFlowApp::open_markdown_in_pane`.
    ///
    /// Markdown tabs don't need an event subscription: `MarkdownView` does
    /// not emit pane-level events. Closing the tab through the tab strip's
    /// close button drops the entity, which in turn drops its file watcher.
    pub fn add_markdown_tab(&mut self, markdown: Entity<MarkdownView>, _cx: &mut Context<Self>) {
        self.tabs.push(TabContent::Markdown(markdown));
        self.selected_idx = self.tabs.len() - 1;
    }

    /// Subscribe to a terminal's events — close tab on exit, repaint on title change.
    fn subscribe_terminal(terminal: &Entity<TerminalView>, cx: &mut Context<Self>) {
        cx.subscribe(terminal, |this, terminal, event: &TerminalEvent, cx| {
            match event {
                TerminalEvent::ChildExited => {
                    if let Some(idx) = this
                        .tabs
                        .iter()
                        .position(|t| t.as_terminal() == Some(&terminal))
                    {
                        this.close_tab_at(idx, cx);
                    }
                }
                TerminalEvent::TitleChanged => {
                    if !crate::terminal::SUPPRESS_REPAINTS
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        cx.notify();
                    }
                }
                // CwdChanged, ActivityBurst, ServiceDetected, SelectionCopied are
                // handled by PaneFlowApp's direct subscription to each TerminalView.
                TerminalEvent::CwdChanged(_)
                | TerminalEvent::ActivityBurst
                | TerminalEvent::ServiceDetected(_)
                | TerminalEvent::CancelSwapMode
                | TerminalEvent::SelectionCopied
                | TerminalEvent::Bell
                | TerminalEvent::OpenMarkdownPath(_)
                | TerminalEvent::OpenCodePath { .. } => {}
            }
        })
        .detach();
    }

    /// Get a display title for a tab. Markdown tabs use the file basename;
    /// terminal tabs detect well-known programs from the OSC title.
    ///
    /// Both variants are capped at 24 chars (Zed `MAX_TAB_TITLE_LEN`,
    /// `crates/editor/src/items.rs:64`). The CSS truncation chain
    /// (`min_w_0 + overflow_x_hidden + text_ellipsis`) on the title div
    /// is a second layer that catches edge cases — but Zed's experience is
    /// that flex layouts with `max_w` (no explicit `w`) sometimes fail to
    /// propagate the constraint, so capping the string up front is
    /// load-bearing for visual consistency. Without this, a long markdown
    /// filename like `prd-opencode-sessions.md` overflows the tab chip.
    fn tab_title(tab: &TabContent, cx: &App) -> String {
        let raw = match tab {
            TabContent::Markdown(md) => md.read(cx).title().to_string(),
            TabContent::Terminal(t) => Self::terminal_tab_title(t, cx),
        };
        truncate_tab_title(&raw)
    }

    /// Icon path for a tab (rendered as a small leading SVG inside the tab
    /// chip). Differentiates terminal and markdown tabs at a glance.
    fn tab_icon(tab: &TabContent) -> &'static str {
        match tab {
            TabContent::Terminal(_) => "icons/terminal.svg",
            TabContent::Markdown(_) => "icons/file-text.svg",
        }
    }

    fn terminal_tab_title(terminal: &Entity<TerminalView>, cx: &App) -> String {
        let raw = &terminal.read(cx).terminal.title;
        if raw.is_empty() {
            return "Terminal".into();
        }
        // Detect well-known programs from OSC title
        let lower = raw.to_lowercase();
        if lower.contains("claude") {
            return "Claude Code".into();
        }
        if lower.contains("codex") {
            return "Codex".into();
        }
        if lower.contains("nvim") || lower.contains("neovim") {
            return "Neovim".into();
        }
        if lower.contains("vim") && !lower.contains("nvim") {
            return "Vim".into();
        }
        if lower.contains("htop")
            || lower.contains("btop")
            || lower.contains("top") && lower.len() < 10
        {
            return "System Monitor".into();
        }
        // For shell titles like "user@host: /path/to/dir", extract the last path component
        if let Some(path_part) = raw.rsplit(':').next() {
            let trimmed = path_part.trim();
            if (trimmed.starts_with('/') || trimmed.starts_with('~'))
                && let Some(last) = trimmed.rsplit('/').next()
            {
                if !last.is_empty() {
                    return last.to_string();
                }
                // Root "/" — show "/"
                return "/".into();
            }
        }
        // Fallback: pass the raw title through. Length capping happens
        // uniformly in `tab_title` via `truncate_tab_title`, which counts
        // chars (not bytes) so multibyte UTF-8 stays sound.
        raw.clone()
    }

    /// Render a small icon button for the tab bar end section.
    fn action_button(
        id: &'static str,
        icon_path: &'static str,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        Self::command_button(
            SharedString::from(id),
            SharedString::from(icon_path),
            tab_colors().muted,
            handler,
        )
    }

    /// Render a small icon button with a caller-supplied tint colour.
    /// Used for the 2 built-in defaults (Claude / Codex brand colours) and
    /// for user-defined `custom_buttons` (muted, matching the other controls).
    fn command_button(
        id: SharedString,
        icon_path: SharedString,
        tint: Hsla,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| {
                let ui = tab_colors();
                s.bg(ui.subtle)
            })
            .on_click(move |e, w, cx| handler(e, w, cx))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(tint),
            )
    }

    /// Close a tab at the given index. Emits `PaneEvent::Remove` if the pane becomes empty.
    fn close_tab_at(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            cx.emit(PaneEvent::Remove);
            return;
        }
        if self.selected_idx >= self.tabs.len() {
            self.selected_idx = self.tabs.len() - 1;
        }
        cx.notify();
    }

    /// Close the currently selected tab. Returns `true` if the pane is now empty.
    pub fn close_selected_tab(&mut self, cx: &mut Context<Self>) -> bool {
        self.close_tab_at(self.selected_idx, cx);
        self.tabs.is_empty()
    }

    /// Get the currently selected terminal entity, if any. Returns `None`
    /// when the active tab is a markdown viewer or the pane is empty — all
    /// callers must handle the absence (event handlers, workspace ops, IPC,
    /// in-pane action buttons) so a markdown tab never triggers a panic.
    pub fn active_terminal_opt(&self) -> Option<&Entity<TerminalView>> {
        self.tabs
            .get(self.selected_idx)
            .and_then(TabContent::as_terminal)
    }

    // -----------------------------------------------------------------------
    // Tab bar rendering — Zed-style design
    // -----------------------------------------------------------------------

    fn render_tab_bar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_count = self.tabs.len();
        let ui = tab_colors();
        let theme = crate::theme::active_theme();
        let chrome_border = ui.border;

        // Outer container: full-width, fixed height, tab_bar background
        let bar = div()
            .flex()
            .flex_none()
            .flex_row()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .bg(theme.title_bar_background);

        // Scrollable tab area (Zed pattern: overflow_x_scroll on inner row)
        let tabs_area = div()
            .id("pane-tabs-area")
            .relative()
            .flex_1()
            .h_full()
            .overflow_x_hidden()
            .on_click(cx.listener(|this, e: &ClickEvent, _window, cx| {
                if matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2) {
                    let ws_id = this.workspace_id;
                    let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                    this.add_tab(terminal, cx);
                    cx.notify();
                }
            }));

        let mut tabs_row = div()
            .id("pane-tabs-scroll")
            .flex()
            .flex_row()
            .h_full()
            .overflow_x_scroll();

        for i in 0..tab_count {
            let is_selected = i == self.selected_idx;
            let tab_idx = i;
            let group_name = SharedString::from(format!("tab-{i}"));

            let mut tab = div()
                .id(SharedString::from(format!("pane-tab-{i}")))
                .group(group_name.clone())
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .flex_shrink_0()
                .max_w(px(TAB_MAX_WIDTH))
                // Belt-and-suspenders against text-ellipsis miss: even if
                // the inner `content` div fails to honour `min_w_0()` and
                // grows past `max_w`, the visual paint is clipped here so
                // the title never bleeds into the next tab. CSS flex with
                // `max-width` on the parent doesn't always propagate a
                // definite size to flex_1 children — and GPUI inherits
                // that quirk from Taffy.
                .overflow_x_hidden()
                .cursor_pointer()
                .text_size(px(14.));

            if is_selected {
                tab = tab
                    .bg(theme.background)
                    .text_color(ui.text)
                    .border_r_1()
                    .border_color(chrome_border);
            } else {
                tab = tab
                    .bg(theme.title_bar_background)
                    .text_color(ui.muted)
                    .border_r_1()
                    .border_b_1()
                    .border_color(chrome_border);
            }

            // Close button — always visible on active tab, hover-only on inactive.
            // The close button container is always present (to reserve space), but
            // the SVG icon inside uses group_hover to control visibility.
            let close_icon = svg()
                .size(px(12.))
                .flex_none()
                .path("icons/close.svg")
                .text_color(ui.muted);

            let close_btn = div()
                .id(SharedString::from(format!("pane-tab-close-{i}")))
                .flex()
                .flex_shrink_0()
                .ml(px(6.))
                .items_center()
                .justify_center()
                .w(px(CLOSE_SIZE))
                .h(px(CLOSE_SIZE))
                .rounded(px(3.))
                .cursor_pointer()
                .hover(|s| {
                    let ui = tab_colors();
                    s.bg(ui.subtle).text_color(rgb(0xf38ba8))
                })
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.close_tab_at(tab_idx, cx);
                    cx.stop_propagation();
                }))
                .opacity(0.)
                .group_hover(group_name, |s| s.opacity(1.))
                .child(close_icon);

            // Inner content row: [icon] [centered label] [close button]
            // The icon (terminal vs markdown) lives in the left slot — its
            // 14px footprint plus the 6px gap mirrors the close button +
            // its 6px ml on the right, so the label stays visually centered.
            let icon_path = Self::tab_icon(&self.tabs[i]);
            let leading_icon = div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(px(CLOSE_SIZE))
                .h(px(CLOSE_SIZE))
                .child(
                    svg()
                        .size(px(12.))
                        .flex_none()
                        .path(icon_path)
                        .text_color(if is_selected { ui.text } else { ui.muted }),
                );
            let content = div()
                .id(SharedString::from(format!("pane-tab-content-{i}")))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(TAB_GAP))
                .h(px(TAB_CONTENT_HEIGHT))
                .px(px(TAB_PX))
                // Critical: as the only flex child of `tab` (which uses
                // `max_w(TAB_MAX_WIDTH)`), `content` defaults to
                // `min-width: auto` and refuses to shrink below its
                // natural size — which for a 24-char title is ~270px,
                // overflowing the tab's 200px cap and pushing the title
                // visibly past the close-button slot. `min_w_0()` opts
                // into the "can shrink to anything" mode so the flex
                // engine actually clamps `content` to the tab's effective
                // width, which in turn lets the title's
                // `flex_1 + min_w_0 + text_ellipsis` chain ellipsize.
                // See Zed `crates/markdown/src/markdown.rs:1291` for the
                // same `flex_1().w_0()` workaround in their list-item
                // path.
                .min_w_0()
                .w_full()
                .on_click(cx.listener(move |this, _, window, cx| {
                    if tab_idx < this.tabs.len() {
                        this.selected_idx = tab_idx;
                        this.focus_handle(cx).focus(window, cx);
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .child(leading_icon)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_align(gpui::TextAlign::Center)
                        .child(Self::tab_title(&self.tabs[i], cx)),
                )
                .child(close_btn);

            tab = tab.child(content);
            tabs_row = tabs_row.child(tab);
        }

        let tabs_area = tabs_area
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .border_b_1()
                    .border_color(chrome_border),
            )
            .child(tabs_row);

        // End section: action buttons
        let mut end_section = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h_full()
            .border_l_1()
            .border_b_1()
            .border_color(chrome_border)
            .px(px(SECTION_PX))
            .gap(px(TAB_GAP));

        // Zoom indicator badge
        if self.zoomed {
            end_section = end_section.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(4.))
                    .h(px(18.))
                    .rounded(px(3.))
                    .bg(ui.accent)
                    .text_size(px(10.))
                    .text_color(ui.base)
                    .child("Z"),
            );
        }

        end_section = end_section
            // New terminal tab
            .child(Self::action_button(
                "pane-btn-new-tab",
                "icons/terminal.svg",
                cx.listener(|this, _, _window, cx| {
                    let ws_id = this.workspace_id;
                    let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                    this.add_tab(terminal, cx);
                    cx.notify();
                }),
            ))
            // Split vertical (panes side by side)
            .child(Self::action_button(
                "pane-btn-split-v",
                "icons/split_vertical.svg",
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(PaneEvent::Split(crate::layout::SplitDirection::Vertical));
                }),
            ))
            // Split horizontal (panes top/bottom)
            .child(Self::action_button(
                "pane-btn-split-h",
                "icons/split_horizontal.svg",
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(PaneEvent::Split(crate::layout::SplitDirection::Horizontal));
                }),
            ))
            // Open a markdown file as a NEW TAB in this pane via the native
            // OS file picker (xdg-portal on Linux, NSOpenPanel on macOS,
            // Win32 file-open on Windows). Picker root = workspace git root
            // walked up from the active terminal's cwd, falling back to the
            // cwd itself when the terminal isn't inside a repo. On selection
            // we re-emit `TerminalEvent::OpenMarkdownPath` from the active
            // terminal so `PaneFlowApp::handle_terminal_event` routes through
            // the same `open_markdown_in_pane` codepath as Cmd/Ctrl-click on
            // a `.md` hyperlink (US-020) — single source of truth for "open
            // markdown tab in this pane".
            //
            // No-op when the active tab is already a markdown viewer (the
            // doc button needs a terminal cwd to anchor the picker).
            .child(Self::action_button(
                "pane-btn-open-markdown",
                "icons/file-text.svg",
                cx.listener(|this, _, _window, cx| {
                    let Some(terminal) = this.active_terminal_opt().cloned() else {
                        return;
                    };
                    let cwd = terminal.read(cx).terminal.cwd_now();
                    let start_dir = cwd
                        .as_deref()
                        .and_then(crate::workspace::find_workdir)
                        .or_else(|| cwd.clone());
                    cx.spawn(async move |_this, cx| {
                        let mut dialog = rfd::AsyncFileDialog::new()
                            .set_title("Open markdown file")
                            .add_filter("Markdown", &["md", "markdown", "mdx"]);
                        if let Some(dir) = start_dir.as_ref() {
                            dialog = dialog.set_directory(dir);
                        }
                        let Some(handle) = dialog.pick_file().await else {
                            return;
                        };
                        let path = handle.path().to_path_buf();
                        terminal.update(cx, |_view, cx_term| {
                            cx_term.emit(TerminalEvent::OpenMarkdownPath(path));
                        });
                    })
                    .detach();
                }),
            ))
            // Claude Code session history for the active terminal's cwd.
            // The actual cwd lookup + filesystem scan happens in
            // `PaneFlowApp::handle_pane_event` so the cwd-source resolution
            // and the off-thread read live next to the popover renderer.
            //
            // Hidden when the user has toggled off every AI-agent button in
            // Settings → AI Agent: with no agent visible the popover would
            // open empty, so the icon itself is suppressed for symmetry
            // with the launcher buttons below.
            .when(
                !crate::agent_sessions::enabled_session_agents().is_empty(),
                |s| {
                    s.child(Self::action_button(
                        "pane-btn-claude-sessions",
                        "icons/sessions.svg",
                        cx.listener(|_this, e: &ClickEvent, _window, cx| {
                            cx.emit(PaneEvent::OpenClaudeSessions(e.position()));
                            cx.stop_propagation();
                        }),
                    ))
                },
            )
            // Built-in default command buttons. Each one is opt-out via
            // Settings → AI Agent (or by editing paneflow.json directly);
            // `None` and `Some(true)` render the button, `Some(false)` hides it.
            .when(
                paneflow_config::loader::load_config()
                    .claude_code_button_visible
                    .unwrap_or(true),
                |s| {
                    s.child(Self::command_button(
                        "pane-btn-claude".into(),
                        "icons/claude-color.svg".into(),
                        rgb(0xd97757).into(),
                        cx.listener(|this, _, _window, cx| {
                            let Some(terminal) = this.active_terminal_opt() else {
                                return;
                            };
                            let bypass = paneflow_config::loader::load_config()
                                .claude_code_bypass_permissions
                                .unwrap_or(false);
                            let cmd = if bypass {
                                "clear && claude --permission-mode bypassPermissions"
                            } else {
                                "clear && claude"
                            };
                            terminal.read(cx).send_command(cmd);
                        }),
                    ))
                },
            )
            .when(
                paneflow_config::loader::load_config()
                    .codex_button_visible
                    .unwrap_or(true),
                |s| {
                    s.child(Self::command_button(
                        "pane-btn-codex".into(),
                        "icons/codex-color.svg".into(),
                        rgb(0x7a9dff).into(),
                        cx.listener(|this, _, _window, cx| {
                            let Some(terminal) = this.active_terminal_opt() else {
                                return;
                            };
                            terminal.read(cx).send_command("clear && codex");
                        }),
                    ))
                },
            )
            .when(
                paneflow_config::loader::load_config()
                    .opencode_button_visible
                    .unwrap_or(true),
                |s| {
                    // Opencode's logo is monochrome (currentColor SVG) — tint
                    // with the theme's primary text color so it stays readable
                    // on both dark and light backgrounds.
                    s.child(Self::command_button(
                        "pane-btn-opencode".into(),
                        "icons/opencode-color.svg".into(),
                        tab_colors().text,
                        cx.listener(|this, _, _window, cx| {
                            let Some(terminal) = this.active_terminal_opt() else {
                                return;
                            };
                            terminal.read(cx).send_command("clear && opencode");
                        }),
                    ))
                },
            );

        // User-defined command buttons (persisted per workspace).
        for btn in &self.custom_buttons {
            let command = btn.command.clone();
            let id = SharedString::from(format!("pane-btn-custom-{}", btn.id));
            let icon = SharedString::from(btn.icon.clone());
            end_section = end_section.child(Self::command_button(
                id,
                icon,
                ui.muted,
                cx.listener(move |this, _, _window, cx| {
                    let Some(terminal) = this.active_terminal_opt() else {
                        return;
                    };
                    terminal.read(cx).send_command(&command);
                }),
            ));
        }

        bar.child(tabs_area).child(end_section)
    }
}

impl gpui::Focusable for Pane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.tabs.get(self.selected_idx) {
            Some(TabContent::Terminal(t)) => t.read(cx).focus_handle(cx),
            Some(TabContent::Markdown(m)) => m.read(cx).focus_handle(cx),
            None => cx.focus_handle(),
        }
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tabs.get(self.selected_idx) {
            Some(TabContent::Terminal(t)) => t.clone().into_any_element(),
            Some(TabContent::Markdown(m)) => m.clone().into_any_element(),
            None => div().size_full().into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_tab_bar(window, cx))
            .child(div().flex_1().size_full().overflow_hidden().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TAB_TITLE_LEN, truncate_tab_title};

    #[test]
    fn short_titles_pass_through_unchanged() {
        assert_eq!(truncate_tab_title("README.md"), "README.md");
        assert_eq!(truncate_tab_title("Terminal"), "Terminal");
    }

    #[test]
    fn exactly_max_chars_is_not_truncated() {
        let s: String = "x".repeat(MAX_TAB_TITLE_LEN);
        assert_eq!(truncate_tab_title(&s), s);
    }

    #[test]
    fn over_max_gets_ellipsis() {
        // 25 chars in -> 24 chars out (23 head + ellipsis).
        let input = "prd-opencode-sessions.mdX";
        let out = truncate_tab_title(input);
        assert_eq!(out.chars().count(), MAX_TAB_TITLE_LEN);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn multibyte_utf8_does_not_panic() {
        // Earlier byte-slice path (`&raw[..23]`) panicked when index 23
        // landed in the middle of an accented or CJK char. The char-based
        // implementation must stay sound.
        let input = "événement-très-très-long-fichier.md"; // many multibyte chars
        let out = truncate_tab_title(input);
        assert_eq!(out.chars().count(), MAX_TAB_TITLE_LEN);
        assert!(out.ends_with('…'));
        let cjk = "プロジェクト・パネフロー・テスト・ドキュメント.md";
        let out = truncate_tab_title(cjk);
        assert_eq!(out.chars().count(), MAX_TAB_TITLE_LEN);
    }
}
