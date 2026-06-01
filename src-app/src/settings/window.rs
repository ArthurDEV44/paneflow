//! `SettingsWindow` view shell — owns the struct, lifecycle (`new`, `cleanup`),
//! focus wiring, and the `Render` impl that composes the title bar, sidebar,
//! tab content, and the CSD resize-edge hitbox.
//!
//! Tab bodies live in `settings::tabs::*`; keyboard handlers live in
//! `settings::keyboard`.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    App, Bounds, Context, CursorStyle, Decorations, FocusHandle, Focusable, HitboxBehavior, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement,
    Pixels, Point, Render, ResizeEdge, Styled, Window, WindowControlArea, canvas, div, point,
    prelude::*, px, transparent_black,
};

use crate::keybindings;
use crate::window_chrome::csd::{default_button_layout, resize_edge};

pub(crate) const SETTINGS_SIDEBAR_WIDTH: Pixels = px(220.);
pub(crate) const SETTINGS_RESIZE_BORDER: Pixels = px(10.);

/// Sidebar background — same token as the agents sidebar so the two
/// surfaces feel like one app chrome.
pub(crate) fn settings_sidebar_bg() -> Hsla {
    crate::theme::active_theme().title_bar_background
}

/// Content area background — same as the sidebar to read as a single
/// panel (agents view also flattens chrome/content into one color).
pub(crate) fn settings_content_bg() -> Hsla {
    crate::theme::active_theme().title_bar_background
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SettingsSection {
    Shortcuts,
    Appearance,
    /// AI agent toggles (Claude Code bypass-permissions, future Codex flags).
    /// Persisted to `paneflow.json` like every other settings tab — users can
    /// also edit the JSON directly.
    AiAgent,
}

pub struct SettingsWindow {
    pub(super) section: SettingsSection,
    pub(super) effective_shortcuts: Vec<keybindings::ShortcutEntry>,
    pub(super) recording_shortcut_idx: Option<usize>,
    pub(super) settings_focus: FocusHandle,
    pub(super) mono_font_names: Vec<String>,
    pub(super) font_dropdown_open: bool,
    pub(super) font_search: String,
    pub(super) should_move: bool,
    pub(super) content_scroll: gpui::ScrollHandle,
    pub(super) content_drag: Option<crate::widgets::scrollbar::ScrollDragState>,
    /// US-012 — MCP bridge install affordance state. Read-only `status`
    /// snapshot (cached so render never does config I/O), the last
    /// `install` result (Ok per-agent recap, or Err refusal message), and
    /// a busy flag while the background install runs. All three are
    /// mutated only from the GPUI main thread inside `cx.spawn` callbacks.
    pub(super) mcp_status: Option<Vec<paneflow_mcp_install::StatusReport>>,
    pub(super) mcp_install: Option<Result<Vec<paneflow_mcp_install::InstallReport>, String>>,
    pub(super) mcp_busy: bool,
}

impl SettingsWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            this.cleanup(cx);
        })
        .detach();

        let config = paneflow_config::loader::load_config();

        let window = Self {
            section: SettingsSection::Shortcuts,
            effective_shortcuts: keybindings::effective_shortcuts(&config.shortcuts),
            recording_shortcut_idx: None,
            settings_focus: cx.focus_handle(),
            mono_font_names: Vec::new(),
            font_dropdown_open: false,
            font_search: String::new(),
            should_move: false,
            content_scroll: gpui::ScrollHandle::new(),
            content_drag: None,
            mcp_status: None,
            mcp_install: None,
            mcp_busy: false,
        };

        // US-012: warm the MCP bridge status off the main thread so the
        // AI-agent tab can render its button label without ever doing
        // config I/O during a frame.
        window.refresh_mcp_status(cx);
        window
    }

    /// Compose the scrollable settings content area + visible scrollbar
    /// overlay. Lives on the `SettingsWindow` Entity so the drag state
    /// (`content_drag`) is local to this window — the parent `PaneFlowApp`
    /// has its own copy for the inline-settings render path.
    fn render_content_scroll_area(
        &self,
        content: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::widgets::scrollbar;

        let inner = div()
            .id("settings-content")
            .flex_1()
            .pr(scrollbar::SCROLLBAR_GUTTER)
            .bg(settings_content_bg())
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .flex()
            .flex_col()
            .items_start()
            .child(
                div()
                    .w_full()
                    .max_w(px(640.))
                    .mx_auto()
                    .px(px(20.))
                    .pt(px(20.))
                    .pb(px(20.))
                    .child(content),
            );

        let bar = scrollbar::render(
            &self.content_scroll,
            crate::theme::ui_colors(),
            None,
            "settings-content-scrollbar-track",
            "settings-content-scrollbar-thumb",
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if let Some(off) =
                    scrollbar::track_click_offset(&this.content_scroll, ev.position.y)
                {
                    this.content_scroll.set_offset(Point::new(px(0.), px(off)));
                    cx.notify();
                }
            }),
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                this.content_drag =
                    Some(scrollbar::begin_drag(&this.content_scroll, ev.position.y));
                cx.stop_propagation();
            }),
        );

        div()
            .id("settings-content-wrapper")
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some(drag) = this.content_drag
                    && let Some(off) =
                        scrollbar::drag_offset(&this.content_scroll, &drag, ev.position.y)
                {
                    this.content_scroll.set_offset(Point::new(px(0.), px(off)));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.content_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(inner)
            .when_some(bar, |d, sb| d.child(sb))
    }

    fn cleanup(&mut self, cx: &mut App) {
        if self.recording_shortcut_idx.is_some() {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
        }
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.settings_focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let decorations = window.window_decorations();

        let content = match self.section {
            SettingsSection::Shortcuts => self.render_shortcuts_content(cx).into_any_element(),
            SettingsSection::Appearance => self.render_appearance_content(cx).into_any_element(),
            SettingsSection::AiAgent => self.render_ai_agent_content(cx).into_any_element(),
        };

        match decorations {
            Decorations::Client { .. } => window.set_client_inset(SETTINGS_RESIZE_BORDER),
            Decorations::Server => window.set_client_inset(px(0.0)),
        }

        // ── Settings title bar ──────────────────────────────────────────
        // Reuses `window_chrome::csd::render_button_group`; only the brand
        // and the centered "Settings" label are bespoke. Close dispatches
        // `window.remove_window()` directly (no PaneFlowApp event bus).
        let title_bar = {
            let height = (1.75 * window.rem_size()).max(px(34.));
            let is_csd = matches!(decorations, Decorations::Client { .. });
            let theme = crate::theme::active_theme();
            let bg_color = if window.is_window_active() {
                theme.title_bar_background
            } else {
                theme.title_bar_inactive_background
            };
            let layout = cx.button_layout().unwrap_or_else(default_button_layout);
            let is_maximized = window.is_maximized();
            let supported = window.window_controls();
            let on_close = |window: &mut Window, _cx: &mut gpui::App| window.remove_window();

            let left_controls = if is_csd {
                crate::window_chrome::csd::render_button_group(
                    "l",
                    &layout.left,
                    is_maximized,
                    &supported,
                    on_close,
                )
            } else {
                None
            };
            let right_controls = if is_csd {
                crate::window_chrome::csd::render_button_group(
                    "r",
                    &layout.right,
                    is_maximized,
                    &supported,
                    on_close,
                )
            } else {
                None
            };

            let brand = div()
                .w(SETTINGS_SIDEBAR_WIDTH)
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .pl_3()
                .overflow_x_hidden()
                .child(
                    div()
                        .text_color(ui.text)
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("Paneflow"),
                );

            let title = div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .px(px(12.))
                .child(
                    div()
                        .text_color(ui.text)
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("Settings"),
                );

            let csd_rounding = px(10.);
            let mut bar = div()
                .id("settings-title-bar")
                .window_control_area(WindowControlArea::Drag)
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(height)
                .bg(bg_color)
                .pr(px(12.));

            if let Decorations::Client { tiling } = decorations {
                if !(tiling.top || tiling.left) {
                    bar = bar.rounded_tl(csd_rounding);
                }
                if !(tiling.top || tiling.right) {
                    bar = bar.rounded_tr(csd_rounding);
                }
                bar = bar
                    .mt(px(-1.))
                    .mb(px(-1.))
                    .border(px(1.))
                    .border_color(bg_color);
            }

            bar.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = false;
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.should_move = false;
            }))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.should_move {
                    this.should_move = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.zoom_window();
                }
            })
            .when(supported.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |ev, window, _| {
                    window.show_window_menu(ev.position);
                })
            })
            .children(left_controls)
            .child(brand)
            .child(title)
            .children(right_controls)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(ui.border),
            )
            .into_any_element()
        };

        let app_content = div()
            .id("settings-window")
            .font_family("IBM Plex Sans")
            .track_focus(&self.settings_focus)
            .on_key_down(cx.listener(Self::handle_settings_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(settings_content_bg())
            .child(title_bar)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_settings_sidebar(self.section, ui, cx))
                    .child(self.render_content_scroll_area(content, cx)),
            );

        div()
            .id("settings-window-backdrop")
            .bg(transparent_black())
            .size_full()
            .map(|d| match decorations {
                Decorations::Server => d,
                Decorations::Client { tiling } => d
                    .child(
                        canvas(
                            |_bounds, window, _cx| {
                                window.insert_hitbox(
                                    Bounds::new(
                                        point(px(0.0), px(0.0)),
                                        window.window_bounds().get_bounds().size,
                                    ),
                                    HitboxBehavior::Normal,
                                )
                            },
                            move |_bounds, hitbox, window, _cx| {
                                let mouse = window.mouse_position();
                                let win_size = window.window_bounds().get_bounds().size;
                                let Some(edge) =
                                    resize_edge(mouse, SETTINGS_RESIZE_BORDER, win_size, tiling)
                                else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .when(!tiling.top, |d| d.pt(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.bottom, |d| d.pb(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.left, |d| d.pl(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.right, |d| d.pr(SETTINGS_RESIZE_BORDER))
                    .on_mouse_move(|_e, window, _cx| window.refresh())
                    .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
                        let win_size = window.window_bounds().get_bounds().size;
                        if let Some(edge) =
                            resize_edge(e.position, SETTINGS_RESIZE_BORDER, win_size, tiling)
                        {
                            window.start_window_resize(edge);
                        }
                    }),
            })
            .child(app_content)
    }
}
