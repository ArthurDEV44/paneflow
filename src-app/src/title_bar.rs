use gpui::{
    AnyElement, ClickEvent, Context, Decorations, EventEmitter, IntoElement, MouseButton, Pixels,
    Point, Render, Styled, Window, WindowButton, WindowControlArea, WindowControls, div,
    prelude::*, px, svg,
};

use crate::csd::default_button_layout;

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
    /// `SystemPackage` was detected but neither apt nor dnf markers were
    /// present (e.g., `eopkg` on Solus, `xbps` on Void).
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

        let left_controls = if is_csd {
            render_button_group("l", &layout.left, is_maximized, &supported, cx)
        } else {
            None
        };

        let right_controls = if is_csd {
            render_button_group("r", &layout.right, is_maximized, &supported, cx)
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

        // --- Right section: spacer (fills remaining space for drag area) ---
        let content = div().flex_1().flex().flex_row().items_center();

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
                        SystemPackageKind::Other => "Update via package manager".to_string(),
                    };
                    (label, PillStyle::SystemHint)
                }
            };

            let mut pill = div()
                .id("update-pill")
                .ml_auto()
                .mr_2()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .border_1()
                .border_color(ui.accent)
                .text_color(ui.accent)
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label);
            match style {
                PillStyle::Clickable => {
                    pill = pill.cursor_pointer().hover(|s| s.opacity(0.7)).on_click(
                        move |_, window, cx| {
                            window.dispatch_action(Box::new(crate::StartSelfUpdate), cx);
                        },
                    );
                }
                PillStyle::Busy => {
                    pill = pill.opacity(0.75);
                }
                // De-emphasized but still clickable so the hint toast shows.
                // Cursor stays default (per US-012 AC) to signal that clicking
                // doesn't perform the update itself.
                PillStyle::SystemHint => {
                    pill = pill.opacity(0.7).on_click(move |_, window, cx| {
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

/// Render a group of window control buttons for one side (left or right).
///
/// Returns `None` if no buttons are active on this side (all slots are `None`
/// or all are filtered out by the compositor's supported controls).
fn render_button_group(
    side: &'static str,
    buttons: &[Option<WindowButton>; 3],
    is_maximized: bool,
    supported: &WindowControls,
    cx: &mut Context<TitleBar>,
) -> Option<AnyElement> {
    let children: Vec<AnyElement> = buttons
        .iter()
        .filter_map(|slot| *slot)
        .filter(|button| match button {
            WindowButton::Minimize => supported.minimize,
            WindowButton::Maximize => supported.maximize,
            WindowButton::Close => true,
        })
        .map(|button| render_window_button(side, button, is_maximized, cx))
        .collect();

    if children.is_empty() {
        return None;
    }

    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(children)
            .into_any_element(),
    )
}

/// Render a single window control button.
fn render_window_button(
    side: &'static str,
    button: WindowButton,
    is_maximized: bool,
    cx: &mut Context<TitleBar>,
) -> AnyElement {
    let id = match button {
        WindowButton::Minimize => "wc-minimize",
        WindowButton::Maximize => "wc-maximize",
        WindowButton::Close => "wc-close",
    };

    let icon_path = match button {
        WindowButton::Minimize => "icons/generic_minimize.svg",
        WindowButton::Maximize if is_maximized => "icons/generic_restore.svg",
        WindowButton::Maximize => "icons/generic_maximize.svg",
        WindowButton::Close => "icons/generic_close.svg",
    };

    let control_area = match button {
        WindowButton::Minimize => WindowControlArea::Min,
        WindowButton::Maximize => WindowControlArea::Max,
        WindowButton::Close => WindowControlArea::Close,
    };

    let element_id = format!("{id}-{side}");

    div()
        .id(gpui::SharedString::from(element_id))
        .window_control_area(control_area)
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(22.))
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
        .on_click(cx.listener(move |_this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            match button {
                WindowButton::Minimize => window.minimize_window(),
                WindowButton::Maximize => window.zoom_window(),
                WindowButton::Close => cx.emit(TitleBarEvent::CloseRequested),
            }
        }))
        .child({
            let ui = crate::theme::ui_colors();
            svg()
                .size(px(16.))
                .flex_none()
                .path(icon_path)
                .text_color(ui.text)
        })
        .into_any_element()
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
