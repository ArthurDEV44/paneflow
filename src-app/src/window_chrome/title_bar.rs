use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Decorations, EventEmitter,
    IntoElement, MouseButton, Pixels, Point, Render, Styled, Transformation, Window,
    WindowControlArea, div, percentage, prelude::*, px, svg,
};

use super::csd::default_button_layout;

pub struct TitleBar {
    should_move: bool,
    pub workspace_name: Option<String>,
    pub sidebar_width: Pixels,
    /// Set by PaneFlowApp when a newer version is detected.
    pub update_available: Option<UpdateInfo>,
}

#[derive(Clone)]
pub struct UpdateInfo {
    pub version: String,
    /// Which pill to render — the in-app flow or the system-package hint.
    pub kind: UpdatePillKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UpdatePillKind {
    /// In-app self-update flow (AppImage / tar.gz / unknown fallback).
    InApp(SelfUpdatePillState),
    /// Managed by the host's package manager (US-012). Clicking the pill
    /// never downloads — it shows a toast with the exact upgrade command.
    SystemManaged(SystemPackageKind),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelfUpdatePillState {
    Idle,
    Downloading,
    Installing,
    Errored,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SystemPackageKind {
    Apt,
    Dnf,
    /// Immutable Fedora variants (Silverblue / Kinoite / Bazzite) —
    /// detected via `/run/ostree-booted`. The pill surfaces a
    /// `rpm-ostree upgrade` hint rather than the usual `dnf`/`apt` copy.
    RpmOstree,
    /// `SystemPackage` was detected but neither apt nor dnf nor ostree
    /// markers were present (e.g., `eopkg` on Solus, `xbps` on Void).
    Other,
}

/// Internal visual/interaction mode for the update pill.
#[derive(Clone, Copy)]
enum PillStyle {
    /// Default accent pill; pointer cursor + hover fade. Dispatches the
    /// in-app update action.
    Clickable,
    /// De-emphasized, non-interactive (download/install in flight).
    Busy,
    /// System-managed install: de-emphasized, default cursor, still clickable
    /// to reveal the package-manager hint toast.
    SystemHint,
}

impl TitleBar {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            should_move: false,
            workspace_name: None,
            sidebar_width: px(220.),
            update_available: None,
        }
    }
}

pub enum TitleBarEvent {
    ToggleMenu(Point<Pixels>),
    ToggleProfile(Point<Pixels>),
    CloseRequested,
}

impl EventEmitter<TitleBarEvent> for TitleBar {}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (1.75 * window.rem_size()).max(px(34.));
        let decorations = window.window_decorations();
        let is_csd = matches!(decorations, Decorations::Client { .. });

        // --- Title bar background from theme, switching on window focus ---
        let theme = crate::theme::active_theme();
        let bg_color = if window.is_window_active() {
            theme.title_bar_background
        } else {
            theme.title_bar_inactive_background
        };

        // --- Read DE button layout ---
        let layout = cx.button_layout().unwrap_or_else(default_button_layout);
        let is_maximized = window.is_maximized();
        let supported = window.window_controls();

        // Close handler: emit CloseRequested so `PaneFlowApp` can intercept
        // (e.g., session save) before the window is removed.
        let close_handle = cx.entity().downgrade();
        let on_close = move |_window: &mut Window, cx: &mut gpui::App| {
            if let Some(entity) = close_handle.upgrade() {
                entity.update(cx, |_this, cx| cx.emit(TitleBarEvent::CloseRequested));
            }
        };

        let left_controls = if is_csd {
            super::csd::render_button_group(
                "l",
                &layout.left,
                is_maximized,
                &supported,
                on_close.clone(),
            )
        } else {
            None
        };

        let right_controls = if is_csd {
            super::csd::render_button_group("r", &layout.right, is_maximized, &supported, on_close)
        } else {
            None
        };

        let menu_button_side = if right_controls.is_some() || left_controls.is_none() {
            "r"
        } else {
            "l"
        };

        let left_menu_button = if is_csd && menu_button_side == "l" {
            Some(render_window_menu_button("l", cx))
        } else {
            None
        };

        let right_menu_button = if is_csd && menu_button_side == "r" {
            Some(render_window_menu_button("r", cx))
        } else {
            None
        };

        // --- Left section: "PaneFlow" brand, fixed width aligned with sidebar ---
        let ui = crate::theme::ui_colors();
        // US-011: on macOS, reserve the leftmost ~80px of the custom titlebar
        // for the native red/yellow/green traffic lights (positioned at
        // x=12,y=12 by WindowOptions::titlebar::traffic_light_position in
        // main.rs). On Linux the window controls are rendered elsewhere,
        // so the brand keeps the historical `pl_3()` (12px) padding.
        let brand_pl = if cfg!(target_os = "macos") {
            gpui::px(80.0)
        } else {
            gpui::px(12.0)
        };
        let brand = div()
            .w(self.sidebar_width)
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .pl(brand_pl)
            .overflow_x_hidden()
            .child(
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("PaneFlow"),
            );

        // --- Center section: workspace name breadcrumb (muted) ---
        // Takes the remaining flex space and centers the current workspace
        // name. Acts as drag area when the workspace is unnamed / unset.
        let mut content = div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(12.))
            .min_w_0();
        if let Some(name) = self.workspace_name.as_ref() {
            content = content.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(3.))
                            .rounded_full()
                            .bg(ui.muted)
                            .flex_none(),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(ui.muted)
                            .truncate()
                            .child(name.clone()),
                    ),
            );
        }

        // --- Update available pill ---
        let update_pill = self.update_available.clone().map(|info| {
            // Decide label + visual style per install method. The click handler
            // always dispatches `StartSelfUpdate`; the action handler in
            // `PaneFlowApp` decides whether to download (in-app) or show the
            // package-manager hint toast (system-managed).
            let (label, style): (String, PillStyle) = match info.kind {
                UpdatePillKind::InApp(state) => match state {
                    SelfUpdatePillState::Idle => {
                        (format!("v{} available", info.version), PillStyle::Clickable)
                    }
                    SelfUpdatePillState::Downloading => {
                        ("Downloading…".to_string(), PillStyle::Busy)
                    }
                    SelfUpdatePillState::Installing => ("Installing…".to_string(), PillStyle::Busy),
                    SelfUpdatePillState::Errored => {
                        ("Update failed".to_string(), PillStyle::Clickable)
                    }
                },
                UpdatePillKind::SystemManaged(kind) => {
                    let label = match kind {
                        SystemPackageKind::Apt => "Update via apt".to_string(),
                        SystemPackageKind::Dnf => "Update via dnf".to_string(),
                        SystemPackageKind::RpmOstree => "Update via rpm-ostree".to_string(),
                        SystemPackageKind::Other => "Update via package manager".to_string(),
                    };
                    (label, PillStyle::SystemHint)
                }
            };

            // Leading icon. Clickable/SystemHint render a static SVG;
            // Busy renders a `loader-circle.svg` arc continuously
            // rotating via GPUI's declarative Animation+Transformation
            // API (one full revolution per second, repeat forever).
            // Pattern mirrors `crates/gpui/examples/animation.rs` in
            // the upstream Zed repo.
            let leading_icon: AnyElement = match style {
                PillStyle::Busy => svg()
                    .size(px(11.))
                    .flex_none()
                    .path("icons/loader-circle.svg")
                    .text_color(ui.muted)
                    .with_animation(
                        "update-pill-spinner",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    )
                    .into_any_element(),
                PillStyle::Clickable => svg()
                    .size(px(11.))
                    .flex_none()
                    .path("icons/download.svg")
                    .text_color(ui.muted)
                    .into_any_element(),
                PillStyle::SystemHint => svg()
                    .size(px(11.))
                    .flex_none()
                    .path("icons/tool.svg")
                    .text_color(ui.muted)
                    .into_any_element(),
            };

            let mut pill = div()
                .id("update-pill")
                .ml_auto()
                .mr_2()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(5.))
                .border_1()
                .border_color(ui.border)
                .bg(ui.subtle)
                .text_color(ui.text)
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(leading_icon)
                .child(label);
            match style {
                PillStyle::Clickable => {
                    pill = pill
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.surface).border_color(ui.muted)
                        })
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(crate::StartSelfUpdate), cx);
                        });
                }
                PillStyle::Busy => {
                    pill = pill.opacity(0.7);
                }
                // De-emphasized but still clickable so the hint toast shows.
                // Cursor stays default (per US-012 AC) to signal that clicking
                // doesn't perform the update itself.
                PillStyle::SystemHint => {
                    pill = pill.opacity(0.8).on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::StartSelfUpdate), cx);
                    });
                }
            }
            pill
        });

        let csd_rounding = px(10.);

        let mut bar = div()
            .id("title-bar")
            .window_control_area(WindowControlArea::Drag)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(height)
            .bg(bg_color)
            .pr(px(12.));

        // CSD rounded corners with tiling awareness
        if let Decorations::Client { tiling } = decorations {
            if !(tiling.top || tiling.left) {
                bar = bar.rounded_tl(csd_rounding);
            }
            if !(tiling.top || tiling.right) {
                bar = bar.rounded_tr(csd_rounding);
            }
            // 1px border + negative margins fill transparent gap at rounded corners
            bar = bar
                .mt(px(-1.))
                .mb(px(-1.))
                .border(px(1.))
                .border_color(bg_color);
        }

        bar
            // Drag-to-move state machine
            .on_mouse_down(
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
            // Right-click opens the DE's native window menu
            .when(supported.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |ev, window, _| {
                    window.show_window_menu(ev.position);
                })
            })
            .children(left_controls)
            .children(left_menu_button)
            .child(brand)
            .child(content)
            .children(update_pill)
            .child(render_profile_button(cx))
            .children(right_controls)
            .children(right_menu_button)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(ui.border),
            )
    }
}

/// Render a title bar menu trigger that matches the window control buttons.
fn render_window_menu_button(side: &'static str, cx: &mut Context<TitleBar>) -> AnyElement {
    let element_id = format!("wc-menu-{side}");

    div()
        .id(gpui::SharedString::from(element_id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(22.))
        .ml(px(6.))
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .active(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|_, event: &ClickEvent, _window, cx| {
            cx.stop_propagation();
            cx.emit(TitleBarEvent::ToggleMenu(event.position()));
        }))
        .child({
            let ui = crate::theme::ui_colors();
            svg()
                .size(px(16.))
                .flex_none()
                .path("icons/menu_2.svg")
                .text_color(ui.text)
        })
        .into_any_element()
}

/// Profile button on the far right of the title bar. Opens a user menu via
/// `TitleBarEvent::ToggleProfile`; the menu body is rendered by
/// `PaneFlowApp` (see `app/profile_menu.rs`). Content is a placeholder until
/// the auth system lands.
fn render_profile_button(cx: &mut Context<TitleBar>) -> AnyElement {
    let ui = crate::theme::ui_colors();
    div()
        .id("title-bar-profile")
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .ml(px(8.))
        .rounded_full()
        .border_1()
        .border_color(ui.border)
        .bg(ui.subtle)
        .cursor_pointer()
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.surface).border_color(ui.muted)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|_, event: &ClickEvent, _window, cx| {
            cx.stop_propagation();
            cx.emit(TitleBarEvent::ToggleProfile(event.position()));
        }))
        .child(
            svg()
                .size(px(13.))
                .flex_none()
                .path("icons/user.svg")
                .text_color(ui.muted),
        )
        .into_any_element()
}
