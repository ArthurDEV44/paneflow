use gpui::{
    div, prelude::*, px, rgb, svg, AnyElement, Context, Decorations, IntoElement, MouseButton,
    Pixels, Render, Styled, Window, WindowButton, WindowButtonLayout, WindowControls,
};

/// Maximum workspace name display length before truncation.
const MAX_WORKSPACE_NAME_LEN: usize = 40;

/// Default button layout when the DE doesn't provide one.
fn default_button_layout() -> WindowButtonLayout {
    WindowButtonLayout {
        left: [None, None, None],
        right: [
            Some(WindowButton::Minimize),
            Some(WindowButton::Maximize),
            Some(WindowButton::Close),
        ],
    }
}

pub struct TitleBar {
    should_move: bool,
    pub workspace_name: Option<String>,
    pub sidebar_width: Pixels,
}

impl TitleBar {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            should_move: false,
            workspace_name: None,
            sidebar_width: px(220.),
        }
    }
}

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
            render_button_group("l", &layout.left, is_maximized, &supported)
        } else {
            None
        };

        let right_controls = if is_csd {
            render_button_group("r", &layout.right, is_maximized, &supported)
        } else {
            None
        };

        // --- Left section: "PaneFlow" brand, fixed width aligned with sidebar ---
        let brand = div()
            .w(self.sidebar_width)
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .pl_3()
            .overflow_x_hidden()
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("PaneFlow"),
            );

        // --- Right section: workspace name (fills remaining space) ---
        let mut content = div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .overflow_x_hidden();

        if let Some(name) = &self.workspace_name {
            let display_name = if name.chars().count() > MAX_WORKSPACE_NAME_LEN {
                let truncated: String = name.chars().take(MAX_WORKSPACE_NAME_LEN).collect();
                format!("{truncated}...")
            } else {
                name.clone()
            };

            content = content.child(
                div()
                    .text_color(rgb(0xa6adc8))
                    .text_sm()
                    .child(display_name),
            );
        }

        let csd_rounding = px(10.);

        let mut bar = div()
            .id("title-bar")
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
            .child(brand)
            .child(content)
            .children(right_controls)
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
) -> Option<AnyElement> {
    let children: Vec<AnyElement> = buttons
        .iter()
        .filter_map(|slot| *slot)
        .filter(|button| match button {
            WindowButton::Minimize => supported.minimize,
            WindowButton::Maximize => supported.maximize,
            WindowButton::Close => true,
        })
        .map(|button| render_window_button(side, button, is_maximized))
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

    let element_id = format!("{id}-{side}");

    div()
        .id(gpui::SharedString::from(element_id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(22.))
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x45475a)))
        .active(|s| s.bg(rgb(0x585b70)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            match button {
                WindowButton::Minimize => window.minimize_window(),
                WindowButton::Maximize => window.zoom_window(),
                WindowButton::Close => cx.quit(),
            }
        })
        .child(
            svg()
                .size(px(16.))
                .flex_none()
                .path(icon_path)
                .text_color(rgb(0xcdd6f4)),
        )
        .into_any_element()
}
