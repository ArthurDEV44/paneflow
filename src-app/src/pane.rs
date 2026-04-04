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
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, SharedString, Styled,
    Window, div, prelude::*, px, rgb, svg,
};

use crate::terminal::{TerminalEvent, TerminalView};

// ---------------------------------------------------------------------------
// Catppuccin Mocha palette — tab bar colors
// ---------------------------------------------------------------------------

/// Tab bar background (= inactive tab background — tabs blend into bar)
const TAB_BAR_BG: u32 = 0x181825; // Mantle
/// Active tab background — slightly lighter, "breaks through" bottom border
const TAB_ACTIVE_BG: u32 = 0x1e1e2e; // Base
/// Border color for tab edges and bottom border
const TAB_BORDER: u32 = 0x313244; // Surface0
/// Active tab text
const TEXT_ACTIVE: u32 = 0xcdd6f4; // Text
/// Inactive tab text
const TEXT_INACTIVE: u32 = 0x6c7086; // Overlay0
/// Close button / muted icon color
const TEXT_MUTED: u32 = 0x585b70; // Surface2
/// Close button hover
const CLOSE_HOVER_BG: u32 = 0x45475a; // Surface1
const CLOSE_HOVER_TEXT: u32 = 0xf38ba8; // Red
/// Hover state for inactive tabs
const TAB_HOVER_BG: u32 = 0x313244; // Surface0

/// Tab bar total height (matches Zed's 32px at default density)
const TAB_BAR_HEIGHT: f32 = 32.0;
/// Inner content height (bar height minus 1px bottom border compensation)
const TAB_CONTENT_HEIGHT: f32 = 31.0;
/// Horizontal padding inside each tab
const TAB_PX: f32 = 4.0;
/// Gap between tab children (icon, label, close button)
const TAB_GAP: f32 = 4.0;
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
    Split(crate::split::SplitDirection),
}

// ---------------------------------------------------------------------------
// Pane — tabbed terminal container
// ---------------------------------------------------------------------------

pub struct Pane {
    pub tabs: Vec<Entity<TerminalView>>,
    pub selected_idx: usize,
}

impl EventEmitter<PaneEvent> for Pane {}

impl Pane {
    /// Create a new pane with a single terminal tab.
    pub fn new(terminal: Entity<TerminalView>, cx: &mut Context<Self>) -> Self {
        Self::subscribe_terminal(&terminal, cx);
        Self {
            tabs: vec![terminal],
            selected_idx: 0,
        }
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
                    cx.notify(); // Trigger tab bar repaint with new title
                }
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
            if trimmed.starts_with('/') || trimmed.starts_with('~') {
                if let Some(last) = trimmed.rsplit('/').next() {
                    if !last.is_empty() {
                        return last.to_string();
                    }
                    // Root "/" — show "/"
                    return "/".into();
                }
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
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(TAB_HOVER_BG)))
            .on_click(move |e, w, cx| handler(e, w, cx))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(rgb(TEXT_MUTED)),
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

    /// Get the currently selected terminal entity.
    pub fn active_terminal(&self) -> &Entity<TerminalView> {
        &self.tabs[self.selected_idx]
    }

    // -----------------------------------------------------------------------
    // Tab bar rendering — Zed-style design
    // -----------------------------------------------------------------------

    fn render_tab_bar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_count = self.tabs.len();

        // Outer container: full-width, fixed height, tab_bar background
        let bar = div()
            .flex()
            .flex_none()
            .flex_row()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .bg(rgb(TAB_BAR_BG));

        // Scrollable tab area with bottom border backdrop (Zed pattern)
        let mut tabs_area = div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_x_hidden()
            // Bottom border backdrop — spans full width behind all tabs.
            // Active tab visually "breaks through" this by not having border_b.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .border_b_1()
                    .border_color(rgb(TAB_BORDER)),
            );

        let mut tabs_row = div().flex().flex_row().h_full();

        for i in 0..tab_count {
            let is_selected = i == self.selected_idx;
            let tab_idx = i;

            // Determine border edges based on position relative to active tab
            // (Zed pattern: active tab has left+right borders, no bottom;
            //  inactive tabs have bottom border, with 1px padding on the side
            //  adjacent to the active tab instead of a drawn border)
            let mut tab = div()
                .id(SharedString::from(format!("pane-tab-{i}")))
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .cursor_pointer()
                .text_size(px(13.))
                .border_color(rgb(TAB_BORDER));

            if is_selected {
                // Active tab: lighter background, no bottom border (pb compensates),
                // left + right 1px borders
                tab = tab
                    .bg(rgb(TAB_ACTIVE_BG))
                    .text_color(rgb(TEXT_ACTIVE))
                    .border_l_1()
                    .border_r_1()
                    .pb(px(1.)); // compensate missing bottom border
            } else {
                // Inactive tab: blends with bar, has bottom border
                tab = tab
                    .bg(rgb(TAB_BAR_BG))
                    .text_color(rgb(TEXT_INACTIVE))
                    .border_b_1()
                    .hover(|s| s.bg(rgb(TAB_HOVER_BG)));

                // Adjacent-to-active border handling:
                // side facing active tab gets 1px padding instead of border
                if i + 1 == self.selected_idx {
                    // This tab is immediately LEFT of active → right padding, no right border
                    tab = tab.pr(px(1.));
                } else if i == self.selected_idx + 1 {
                    // This tab is immediately RIGHT of active → left padding, no left border
                    tab = tab.pl(px(1.));
                }
            }

            // Inner content row
            let content = div()
                .id(SharedString::from(format!("pane-tab-content-{i}")))
                .flex()
                .flex_row()
                .items_center()
                .h(px(TAB_CONTENT_HEIGHT))
                .px(px(TAB_PX))
                .gap(px(TAB_GAP))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    if tab_idx < this.tabs.len() {
                        this.selected_idx = tab_idx;
                        cx.notify();
                    }
                }))
                // Tab label — derived from terminal OSC title
                .child(Self::tab_title(&self.tabs[i], cx))
                // Close button (Zed: 14×14 end_slot, icon Color::Muted)
                .child(
                    div()
                        .id(SharedString::from(format!("pane-tab-close-{i}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(CLOSE_SIZE))
                        .h(px(CLOSE_SIZE))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.bg(rgb(CLOSE_HOVER_BG)).text_color(rgb(CLOSE_HOVER_TEXT)))
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.close_tab_at(tab_idx, cx);
                            cx.stop_propagation();
                        }))
                        .child(
                            svg()
                                .size(px(10.))
                                .flex_none()
                                .path("icons/close.svg")
                                .text_color(rgb(TEXT_MUTED)),
                        ),
                );

            tab = tab.child(content);
            tabs_row = tabs_row.child(tab);
        }

        tabs_area = tabs_area.child(tabs_row);

        // End section: action buttons with left border separator (Zed pattern)
        let end_section = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h_full()
            .px(px(SECTION_PX))
            .gap(px(TAB_GAP))
            .border_b_1()
            .border_color(rgb(TAB_BORDER))
            // New terminal tab
            .child(Self::action_button(
                "pane-btn-new-tab",
                "icons/terminal.svg",
                cx.listener(|this, _, _window, cx| {
                    let terminal = cx.new(TerminalView::new);
                    this.add_tab(terminal, cx);
                    cx.notify();
                }),
            ))
            // Split vertical (panes side by side)
            .child(Self::action_button(
                "pane-btn-split-v",
                "icons/split_vertical.svg",
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(PaneEvent::Split(crate::split::SplitDirection::Vertical));
                }),
            ))
            // Split horizontal (panes top/bottom)
            .child(Self::action_button(
                "pane-btn-split-h",
                "icons/split_horizontal.svg",
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(PaneEvent::Split(crate::split::SplitDirection::Horizontal));
                }),
            ));

        bar.child(tabs_area).child(end_section)
    }
}

impl gpui::Focusable for Pane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_terminal().read(cx).focus_handle(cx)
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
