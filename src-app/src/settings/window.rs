//! `SettingsWindow` view shell — owns the struct, lifecycle (`new`, `cleanup`),
//! focus wiring, and the `Render` impl that composes the title bar, sidebar,
//! tab content, and the CSD resize-edge hitbox.
//!
//! Tab bodies live in `settings::tabs::*`; keyboard handlers live in
//! `settings::keyboard`.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    App, Bounds, Context, CursorStyle, Decorations, FocusHandle, Focusable, HitboxBehavior,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, ResizeEdge,
    Styled, Window, WindowControlArea, canvas, div, point, prelude::*, px, rgb, transparent_black,
};

use crate::keybindings;
use crate::window_chrome::csd::{default_button_layout, resize_edge};

pub(crate) const SETTINGS_SIDEBAR_WIDTH: Pixels = px(180.);
pub(crate) const SETTINGS_RESIZE_BORDER: Pixels = px(10.);

pub(crate) fn settings_sidebar_bg() -> gpui::Rgba {
    let theme = crate::theme::active_theme();
    if theme.background.l > 0.5 {
        rgb(0xe8e8e8)
    } else {
        rgb(0x141414)
    }
}

pub(crate) fn settings_content_bg() -> gpui::Rgba {
    let theme = crate::theme::active_theme();
    if theme.background.l > 0.5 {
        rgb(0xefefef)
    } else {
        rgb(0x1a1a1a)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SettingsSection {
    Shortcuts,
    Appearance,
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
}

impl SettingsWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            this.cleanup(cx);
        })
        .detach();

        let config = paneflow_config::loader::load_config();

        Self {
            section: SettingsSection::Shortcuts,
            effective_shortcuts: keybindings::effective_shortcuts(&config.shortcuts),
            recording_shortcut_idx: None,
            settings_focus: cx.focus_handle(),
            mono_font_names: Vec::new(),
            font_dropdown_open: false,
            font_search: String::new(),
            should_move: false,
        }
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
        let ui_font = crate::terminal::element::resolve_font_family(
            paneflow_config::loader::load_config()
                .font_family
                .as_deref(),
        );
        let decorations = window.window_decorations();

        let content = match self.section {
            SettingsSection::Shortcuts => self.render_shortcuts_content(cx).into_any_element(),
            SettingsSection::Appearance => self.render_appearance_content(cx).into_any_element(),
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
                        .child("PaneFlow"),
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
            .font_family(ui_font)
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
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .bg(settings_content_bg())
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(720.))
                                    .px(px(28.))
                                    .pt(px(24.))
                                    .pb(px(16.))
                                    .child(content),
                            ),
                    ),
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
