//! Pane — a tabbed container holding one or more terminal views.
//!
//! Each leaf in the split tree holds an `Entity<Pane>`. A Pane manages
//! a list of terminal tabs with a tab bar for switching between them.
//!
//! Communication with the parent (split tree owner) uses the Zed pattern:
//! Pane emits `PaneEvent` via `cx.emit()`, parent subscribes via `cx.subscribe()`.
//!
//! Tab bar UI is modeled after Zed's tab bar design.

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, Render, SharedString, Styled, Window, div, prelude::*, px,
    rgb, svg,
};
use paneflow_config::schema::ButtonCommand;

use crate::markdown::MarkdownView;
use crate::terminal::{TerminalEvent, TerminalView};

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

// ---------------------------------------------------------------------------
// Pane events — emitted to parent via cx.emit()
// ---------------------------------------------------------------------------

pub enum PaneEvent {
    /// The last tab was closed — parent should remove this pane from the split tree.
    Remove,
    /// Request a split in the given direction from this pane.
    Split(crate::layout::SplitDirection),
}

// ---------------------------------------------------------------------------
// Pane — tabbed terminal container
// ---------------------------------------------------------------------------

pub struct Pane {
    pub tabs: Vec<Entity<TerminalView>>,
    pub selected_idx: usize,
    /// Set to true when the workspace is zoomed on this pane.
    pub zoomed: bool,
    /// Workspace ID for spawning new terminals with correct env vars.
    pub workspace_id: u64,
    /// Workspace-specific command buttons rendered in the tab bar after the
    /// built-in defaults. Populated/updated by `Workspace::propagate_custom_buttons`.
    pub custom_buttons: Vec<ButtonCommand>,
    /// US-020 — when `Some`, this pane shows a markdown viewer instead of a
    /// terminal. `tabs` is empty in that case. The PRD asks for a `Pane` enum
    /// with `Terminal | Markdown` variants; the additive `Option` field is
    /// the equivalent pattern that keeps the existing terminal hot paths
    /// untouched while allowing the renderer to branch on a single field.
    pub markdown_content: Option<Entity<MarkdownView>>,
}

impl EventEmitter<PaneEvent> for Pane {}

impl Pane {
    /// Create a new pane with a single terminal tab.
    pub fn new(terminal: Entity<TerminalView>, workspace_id: u64, cx: &mut Context<Self>) -> Self {
        Self::subscribe_terminal(&terminal, cx);
        Self {
            tabs: vec![terminal],
            selected_idx: 0,
            zoomed: false,
            workspace_id,
            custom_buttons: Vec::new(),
            markdown_content: None,
        }
    }

    /// US-020 — create a pane that hosts a markdown viewer. The pane has no
    /// terminal tabs; `markdown_content` carries the rendering view.
    pub fn new_markdown(
        markdown: Entity<MarkdownView>,
        workspace_id: u64,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            selected_idx: 0,
            zoomed: false,
            workspace_id,
            custom_buttons: Vec::new(),
            markdown_content: Some(markdown),
        }
    }

    /// US-020 — true when this pane renders a markdown viewer rather than a
    /// terminal tab strip. Used by close / focus / render paths to branch.
    pub fn is_markdown(&self) -> bool {
        self.markdown_content.is_some()
    }

    /// Add a new terminal tab and select it.
    pub fn add_tab(&mut self, terminal: Entity<TerminalView>, cx: &mut Context<Self>) {
        Self::subscribe_terminal(&terminal, cx);
        self.tabs.push(terminal);
        self.selected_idx = self.tabs.len() - 1;
    }

    /// Subscribe to a terminal's events — close tab on exit, repaint on title change.
    fn subscribe_terminal(terminal: &Entity<TerminalView>, cx: &mut Context<Self>) {
        cx.subscribe(terminal, |this, terminal, event: &TerminalEvent, cx| {
            match event {
                TerminalEvent::ChildExited => {
                    if let Some(idx) = this.tabs.iter().position(|t| *t == terminal) {
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
                | TerminalEvent::OpenMarkdownPath(_) => {}
            }
        })
        .detach();
    }

    /// Get a display title for a terminal tab.
    /// Detects well-known programs and returns a clean label.
    fn tab_title(terminal: &Entity<TerminalView>, cx: &App) -> String {
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
        // Fallback: use the raw title, truncated
        if raw.len() > 24 {
            format!("{}…", &raw[..23])
        } else {
            raw.clone()
        }
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
    /// For markdown panes (US-020) there is no tab strip — closing always
    /// removes the entire pane.
    pub fn close_selected_tab(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_markdown() {
            cx.emit(PaneEvent::Remove);
            return true;
        }
        self.close_tab_at(self.selected_idx, cx);
        self.tabs.is_empty()
    }

    /// Get the currently selected terminal entity.
    ///
    /// **Panics** when called on a markdown pane (US-020) where `tabs` is
    /// empty. Internal pane code is gated by `markdown_content` checks before
    /// reaching this; external callers must use [`Self::active_terminal_opt`]
    /// or guard with [`Self::is_markdown`] first.
    pub fn active_terminal(&self) -> &Entity<TerminalView> {
        debug_assert!(
            !self.tabs.is_empty(),
            "active_terminal() called on a markdown pane — use active_terminal_opt() or is_markdown()"
        );
        &self.tabs[self.selected_idx]
    }

    /// US-020 — fallible version of [`Self::active_terminal`] that returns
    /// `None` for markdown panes instead of panicking. External call sites
    /// (event handlers, workspace ops, IPC) must use this entry point so a
    /// markdown pane in the layout tree never triggers an index-out-of-bounds.
    pub fn active_terminal_opt(&self) -> Option<&Entity<TerminalView>> {
        self.tabs.get(self.selected_idx)
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

            // Inner content row: [spacer] [centered label] [close button]
            // The left spacer mirrors the close button width so the label
            // is visually centered within the tab.
            let content = div()
                .id(SharedString::from(format!("pane-tab-content-{i}")))
                .flex()
                .flex_row()
                .items_center()
                .h(px(TAB_CONTENT_HEIGHT))
                .px(px(TAB_PX))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if tab_idx < this.tabs.len() {
                        this.selected_idx = tab_idx;
                        this.focus_handle(cx).focus(window, cx);
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .child(div().w(px(CLOSE_SIZE)).flex_shrink_0())
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
            // Built-in default command buttons (always present, not removable).
            .child(Self::command_button(
                "pane-btn-claude".into(),
                "icons/claude-color.svg".into(),
                rgb(0xd97757).into(),
                cx.listener(|this, _, _window, cx| {
                    this.active_terminal()
                        .read(cx)
                        .send_command("clear && claude");
                }),
            ))
            .child(Self::command_button(
                "pane-btn-codex".into(),
                "icons/codex-color.svg".into(),
                rgb(0x7a9dff).into(),
                cx.listener(|this, _, _window, cx| {
                    this.active_terminal()
                        .read(cx)
                        .send_command("clear && codex");
                }),
            ));

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
                    this.active_terminal().read(cx).send_command(&command);
                }),
            ));
        }

        bar.child(tabs_area).child(end_section)
    }
}

impl gpui::Focusable for Pane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some(md) = &self.markdown_content {
            return md.read(cx).focus_handle(cx);
        }
        self.active_terminal().read(cx).focus_handle(cx)
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // US-020: markdown panes skip the tab strip and render the viewer
        // full-bleed. A 1px header rail with the file's basename gives the
        // pane visual parity with the terminal tab strip while signaling that
        // tab interactions are intentionally disabled.
        if let Some(md) = &self.markdown_content {
            let title = md.read(cx).title();
            let ui = tab_colors();
            let theme = crate::theme::active_theme();
            let header = div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(TAB_BAR_HEIGHT))
                .px(px(TAB_PX))
                .bg(theme.title_bar_background)
                .border_b_1()
                .border_color(ui.border)
                .text_color(ui.muted)
                .child(title);
            return div().flex().flex_col().size_full().child(header).child(
                div()
                    .flex_1()
                    .size_full()
                    .overflow_hidden()
                    .child(md.clone()),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_tab_bar(window, cx))
            .child(
                div()
                    .flex_1()
                    .size_full()
                    .overflow_hidden()
                    .child(self.active_terminal().clone()),
            )
    }
}
