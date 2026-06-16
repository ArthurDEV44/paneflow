//! Codex-style bottom terminal dock for the Agents view.
//!
//! A full-width panel that slides up from the bottom of the Agents main area
//! (toggled by the `layout-bottombar` button in the environment toolbar). It
//! hosts a tab strip: one tab per shell terminal, opened as many as you like via
//! `+` and closed each independently. The panel's top edge is draggable to
//! resize. (The git diff lives in the right-side dock, not here.)
//!
//! Each terminal is a real [`crate::terminal::view::TerminalView`] entity, so
//! its PTY, scrollback, and I/O threads survive tab switches and panel
//! close/reopen (the entities are retained in [`crate::AgentsViewState`] until
//! the tab is closed). PTY env ids come from a namespace disjoint from CLI
//! workspaces and Agents threads so they can never collide.

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, CursorStyle, Focusable, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, svg,
};

use crate::PaneFlowApp;
use crate::settings::components::with_alpha;

/// Initial panel height. Roughly a dozen rows of shell output — enough to be
/// useful without swallowing the agent surface above it.
pub(crate) const BOTTOM_PANEL_DEFAULT_HEIGHT: f32 = 320.0;

/// Floor for the resize drag: keeps the tab strip + a couple of rows visible.
const BOTTOM_PANEL_MIN_HEIGHT: f32 = 140.0;

/// Ceiling for the resize drag: never let the dock fully eat the surface above.
const BOTTOM_PANEL_MAX_HEIGHT: f32 = 760.0;

/// Env-id namespace for bottom-panel PTYs. CLI workspaces live in `0..2^32` and
/// Agents threads in `(1<<32)..` (via [`crate::project::thread_env_id`]); `2<<32`
/// gives every bottom terminal an id that can collide with neither, since the
/// per-session terminal counter never approaches `2^32`.
const BOTTOM_TERMINAL_ENV_ID_BASE: u64 = 2u64 << 32;

impl PaneFlowApp {
    /// The cwd a new bottom terminal should target: the currently selected
    /// thread's working directory, empty if none.
    fn bottom_panel_cwd(&self) -> String {
        self.current_thread_view_target()
            .and_then(|target| self.thread_for_target(target))
            .map(|thread| thread.cwd.clone())
            .unwrap_or_default()
    }

    /// Toggle the bottom dock. Opening with no terminals yet spawns the first
    /// one in the active thread's cwd and focuses it; opening with terminals
    /// already present just re-reveals them and refocuses the active tab.
    /// Closing only hides the panel — terminals stay alive for a warm reopen.
    pub(crate) fn toggle_agents_bottom_panel(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agents_view.bottom_panel_open {
            self.agents_view.bottom_panel_open = false;
            self.agents_view.bottom_panel_drag = None;
            cx.notify();
            return;
        }
        self.agents_view.bottom_panel_open = true;
        if self.agents_view.bottom_terminals.is_empty() {
            self.spawn_bottom_terminal(window, cx);
        } else {
            self.focus_bottom_panel_active(window, cx);
        }
        cx.notify();
    }

    /// Close the whole dock (panel × button). Terminals are retained.
    pub(crate) fn close_agents_bottom_panel(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agents_view.bottom_panel_open = false;
        self.agents_view.bottom_panel_drag = None;
        cx.notify();
    }

    /// Spawn a fresh shell terminal as a new tab in the active thread's cwd and
    /// make it active. Mirrors `ensure_terminal_view_mounted`: the PTY opens on a
    /// background thread, and an OSC-title subscription keeps the tab label in
    /// sync with the running process.
    pub(crate) fn spawn_bottom_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.bottom_panel_cwd();
        let seq = self.agents_view.bottom_terminal_seq + 1;
        self.agents_view.bottom_terminal_seq = seq;
        let id = seq;
        let env_id = BOTTOM_TERMINAL_ENV_ID_BASE + seq;
        let cwd_path = if cwd.trim().is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&cwd))
        };

        let view =
            cx.new(|cx| crate::terminal::view::TerminalView::with_cwd(env_id, cwd_path, None, cx));

        // OSC 0/2 title → tab label, so a tab reads "zsh" / "claude" / a cwd
        // rather than a frozen "Terminal N". Detached: the entity owns its
        // lifecycle and the listener drops when the tab (and entity) is removed.
        cx.subscribe(
            &view,
            move |this, src, event: &crate::terminal::view::TerminalEvent, cx| {
                if matches!(event, crate::terminal::view::TerminalEvent::TitleChanged) {
                    let title = src.read(cx).terminal.title.clone();
                    this.note_bottom_terminal_title(id, title, cx);
                }
            },
        )
        .detach();

        self.agents_view
            .bottom_terminals
            .push(crate::BottomTerminal {
                id,
                title: format!("Terminal {seq}"),
                view: view.clone(),
            });
        self.agents_view.bottom_panel_active = Some(id);
        view.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Select a terminal tab and route the keyboard to its PTY.
    pub(crate) fn select_bottom_terminal_tab(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agents_view.bottom_panel_active = Some(id);
        if let Some(term) = self
            .agents_view
            .bottom_terminals
            .iter()
            .find(|t| t.id == id)
        {
            term.view.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Close one terminal tab. Its entity drops here, tearing the shell down via
    /// `TerminalView`'s `Drop`. If the closed tab was active, focus the nearest
    /// surviving terminal, or drop to the empty state when none remain.
    pub(crate) fn close_bottom_terminal(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self
            .agents_view
            .bottom_terminals
            .iter()
            .position(|t| t.id == id)
        else {
            return;
        };
        self.agents_view.bottom_terminals.remove(pos);

        if self.agents_view.bottom_panel_active == Some(id) {
            if self.agents_view.bottom_terminals.is_empty() {
                self.agents_view.bottom_panel_active = None;
            } else {
                let idx = pos.min(self.agents_view.bottom_terminals.len() - 1);
                let next = &self.agents_view.bottom_terminals[idx];
                self.agents_view.bottom_panel_active = Some(next.id);
                let view = next.view.clone();
                view.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    /// Update a terminal tab's label from its PTY's reported title. Strips
    /// spinner glyphs and ignores alacritty's `Terminal` reset fallback, mirroring
    /// the thread-title handler so a finished agent doesn't blank a useful label.
    pub(crate) fn note_bottom_terminal_title(
        &mut self,
        id: u64,
        raw: String,
        cx: &mut Context<Self>,
    ) {
        let Some(normalized) = crate::project::clean_sidebar_title(&raw) else {
            return;
        };
        if normalized == "Terminal" {
            return;
        }
        if let Some(term) = self
            .agents_view
            .bottom_terminals
            .iter_mut()
            .find(|t| t.id == id)
            && term.title != normalized
        {
            term.title = normalized;
            cx.notify();
        }
    }

    fn focus_bottom_panel_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.agents_view.bottom_panel_active
            && let Some(term) = self
                .agents_view
                .bottom_terminals
                .iter()
                .find(|t| t.id == id)
        {
            term.view.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Apply a live resize drag: set the dock height so its top edge tracks the
    /// cursor. Driven by the Agents main area's `on_mouse_move` (a wide capture
    /// surface, so the drag survives the cursor leaving the dock). No-op when no
    /// drag is in progress.
    pub(crate) fn drag_bottom_panel_resize(&mut self, cursor_y: f32, cx: &mut Context<Self>) {
        if let Some((anchor_y, anchor_h)) = self.agents_view.bottom_panel_drag {
            let delta = anchor_y - cursor_y;
            self.agents_view.bottom_panel_height =
                (anchor_h + delta).clamp(BOTTOM_PANEL_MIN_HEIGHT, BOTTOM_PANEL_MAX_HEIGHT);
            cx.notify();
        }
    }

    /// End a resize drag (mouse up / button released mid-move). Returns whether a
    /// drag was actually in progress, so the caller can skip a redundant notify.
    pub(crate) fn end_bottom_panel_resize(&mut self, cx: &mut Context<Self>) -> bool {
        if self.agents_view.bottom_panel_drag.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// Render the dock: a draggable top edge, the tab strip, and the active
    /// terminal's surface. Spans the full width of the Agents main area.
    pub(crate) fn render_agents_bottom_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let height = self.agents_view.bottom_panel_height;
        let active = self.agents_view.bottom_panel_active;
        let tabs: Vec<(u64, SharedString)> = self
            .agents_view
            .bottom_terminals
            .iter()
            .map(|t| (t.id, SharedString::from(t.title.clone())))
            .collect();

        let content: AnyElement = match active.and_then(|id| {
            self.agents_view
                .bottom_terminals
                .iter()
                .find(|t| t.id == id)
        }) {
            Some(term) => render_bottom_terminal_surface(term.view.clone(), ui),
            None => render_bottom_empty(ui),
        };

        div()
            .id("agents-bottom-panel")
            .relative()
            .flex_none()
            .w_full()
            .h(px(height))
            .flex()
            .flex_col()
            .bg(ui.base)
            .border_t_1()
            .border_color(ui.border)
            .child(render_bottom_resize_handle(ui, cx))
            .child(render_bottom_tab_strip(tabs, active, ui, cx))
            .child(content)
            .into_any_element()
    }
}

/// The thin, row-resize hit target straddling the panel's top border. Captures
/// the drag anchor `(cursor_y, height_at_grab)`; the actual resize math runs in
/// the panel root's `on_mouse_move`.
fn render_bottom_resize_handle(
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    div()
        .id("agents-bottom-resize")
        .absolute()
        .top(px(-3.))
        .left_0()
        .right_0()
        .h(px(7.))
        .cursor(CursorStyle::ResizeUpDown)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.06)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, _w, cx| {
                let h = this.agents_view.bottom_panel_height;
                this.agents_view.bottom_panel_drag = Some((f32::from(event.position.y), h));
                cx.notify();
            }),
        )
        .into_any_element()
}

fn render_bottom_tab_strip(
    tabs: Vec<(u64, SharedString)>,
    active: Option<u64>,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let mut scroll = div()
        .id("agents-bottom-tabs-scroll")
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.))
        .overflow_x_scroll();
    for (id, label) in tabs {
        let is_active = active == Some(id);
        scroll = scroll.child(render_bottom_terminal_tab(id, label, is_active, ui, cx));
    }
    // Keep + with the tabs: it trails the last terminal inside the scroll row
    // instead of pinning to the far edge, so it reads as "append another".
    scroll = scroll.child(render_bottom_add_button(ui, cx));

    div()
        .id("agents-bottom-tabstrip")
        .flex_none()
        .h(px(40.))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .pl(px(8.))
        .pr(px(6.))
        .bg(ui.base)
        .child(scroll)
        .child(render_bottom_panel_close_button(ui, cx))
        .into_any_element()
}

fn render_bottom_terminal_tab(
    id: u64,
    label: SharedString,
    active: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let (bg, fg) = tab_colors(active, ui);
    let hover_bg = with_alpha(ui.text, if active { 0.09 } else { 0.05 });
    div()
        .id(SharedString::from(format!("agents-bottom-tab-{id}")))
        .flex_none()
        .h(px(28.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(7.))
        .pl(px(11.))
        .pr(px(5.))
        .rounded(px(8.))
        .bg(bg)
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(hover_bg))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
            this.select_bottom_terminal_tab(id, window, cx);
        }))
        .child(
            svg()
                .size(px(13.))
                .flex_none()
                .path("icons/terminal.svg")
                .text_color(fg),
        )
        .child(
            div()
                .max_w(px(150.))
                .min_w_0()
                .overflow_x_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.5))
                .text_color(fg)
                .child(label),
        )
        .child(render_bottom_tab_close_button(id, ui, cx))
        .into_any_element()
}

fn render_bottom_tab_close_button(
    id: u64,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("agents-bottom-tab-x-{id}")))
        .flex_none()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.14)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
            this.close_bottom_terminal(id, window, cx);
        }))
        .child(
            svg()
                .size(px(11.))
                .flex_none()
                .path("icons/close.svg")
                .text_color(ui.muted),
        )
        .into_any_element()
}

fn render_bottom_add_button(
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    div()
        .id("agents-bottom-add")
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.08)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
            this.spawn_bottom_terminal(window, cx);
        }))
        .child(
            svg()
                .size(px(15.))
                .flex_none()
                .path("icons/plus.svg")
                .text_color(with_alpha(ui.text, 0.75)),
        )
        .into_any_element()
}

fn render_bottom_panel_close_button(
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    div()
        .id("agents-bottom-close")
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.08)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
            this.close_agents_bottom_panel(event, window, cx);
        }))
        .child(
            svg()
                .size(px(14.))
                .flex_none()
                .path("icons/close.svg")
                .text_color(ui.muted),
        )
        .into_any_element()
}

/// The active terminal tab's content. `TerminalView` paints `size_full` with no
/// chrome of its own, so the caller owns the background.
fn render_bottom_terminal_surface(
    view: gpui::Entity<crate::terminal::view::TerminalView>,
    ui: crate::theme::UiColors,
) -> AnyElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .bg(ui.base)
        .child(view.into_any_element())
        .into_any_element()
}

fn render_bottom_empty(ui: crate::theme::UiColors) -> AnyElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(ui.muted)
        .child("No terminal open. Use + to start one.")
        .into_any_element()
}

/// Toolbar button (sibling to the diff dock toggle) that opens the bottom dock.
/// Matches the other environment-toolbar glyph toggles: bare at rest, a whisper
/// fill on hover or while the dock is open.
pub(crate) fn render_agents_bottom_toggle_button(
    open: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let fill = with_alpha(ui.text, if open { 0.08 } else { 0.0 });
    let hover = with_alpha(ui.text, 0.08);
    div()
        .id("agents-env-toolbar-bottom")
        .flex_none()
        .h(px(28.))
        .w(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .cursor(CursorStyle::PointingHand)
        .bg(fill)
        .hover(move |d| d.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.toggle_agents_bottom_panel(event, window, cx);
        }))
        .child(
            svg()
                .size(px(16.))
                .flex_none()
                .path("icons/layout-bottombar.svg")
                .text_color(with_alpha(ui.text, 0.7)),
        )
        .into_any_element()
}

/// Resting background + foreground for a tab, by active state.
fn tab_colors(active: bool, ui: crate::theme::UiColors) -> (gpui::Hsla, gpui::Hsla) {
    if active {
        (with_alpha(ui.text, 0.09), ui.text)
    } else {
        (with_alpha(ui.text, 0.0), ui.muted)
    }
}
