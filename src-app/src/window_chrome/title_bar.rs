use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Decorations, EventEmitter,
    FontWeight, IntoElement, MouseButton, Pixels, Render, Styled, Transformation, Window,
    WindowControlArea, div, percentage, prelude::*, px, rgb, svg,
};

use super::csd::default_button_layout;

pub struct TitleBar {
    should_move: bool,
    pub workspace_name: Option<String>,
    pub sidebar_width: Pixels,
    /// Set by PaneFlowApp when a newer version is detected.
    pub update_available: Option<UpdateInfo>,
    /// US-009 (prd-agents-view.md): the current top-level UI mode,
    /// pushed every render by [`crate::PaneFlowApp::render`]. Drives
    /// the title-bar toggle icon and (indirectly, via the
    /// `sidebar_width` field above) the sidebar slot width.
    pub mode: paneflow_config::schema::AppMode,
    /// US-009 (prd-agents-view.md): human-readable shortcut for the
    /// `OpenAgentsView` action (e.g. `"Ctrl+Shift+A"` on Linux,
    /// `"Cmd+Shift+A"` on macOS). Resolved by PaneFlowApp from the
    /// live keybinding registry and pushed every render so the
    /// tooltip stays in sync if the user remaps the binding.
    pub agents_view_shortcut: Option<String>,
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
    /// Background install completed; the next click only invokes
    /// `cx.restart()`. Mirrors Zed's "Restart to Update" CTA — the heavy
    /// work happened while the user was busy doing something else, so the
    /// click→restart latency is bounded by GPUI's relauncher only (~100 ms).
    ReadyToRestart,
    Errored,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SystemPackageKind {
    /// Immutable Fedora variants (Silverblue / Kinoite / Bazzite) —
    /// detected via `/run/ostree-booted`. The pill surfaces a
    /// `rpm-ostree upgrade` hint rather than the usual `dnf`/`apt` copy.
    RpmOstree,
    /// `SystemPackage` was detected but neither apt nor dnf nor ostree
    /// markers were present (e.g., `eopkg` on Solus, `xbps` on Void).
    /// Apt/Dnf are intentionally absent: they route through the in-app
    /// pkexec installer (UpdatePillKind::InApp), not SystemManaged.
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
            // US-009 (prd-agents-view.md): defaults that PaneFlowApp
            // overwrites on every render. Defaulting here means the
            // title bar still renders sensibly if a future caller
            // forgets to push the field.
            mode: paneflow_config::schema::AppMode::Cli,
            agents_view_shortcut: None,
        }
    }
}

pub enum TitleBarEvent {
    CloseRequested,
    /// US-009 (prd-agents-view.md): user clicked the agents-view
    /// toggle icon. PaneFlowApp's subscriber dispatches the
    /// `OpenAgentsView` action so the existing US-008 handler runs
    /// (no duplication of focus-restore / lazy-mount logic).
    ToggleAgentsView,
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
            // `PaneFlowApp` decides whether to download (in-app), trigger
            // an instant restart (ReadyToRestart), or show the
            // package-manager hint toast (system-managed).
            let (label, style): (String, PillStyle) = match info.kind {
                UpdatePillKind::InApp(state) => match state {
                    SelfUpdatePillState::Idle => {
                        (format!("v{} available", info.version), PillStyle::Clickable)
                    }
                    SelfUpdatePillState::Downloading => {
                        ("Downloading update…".to_string(), PillStyle::Busy)
                    }
                    SelfUpdatePillState::Installing => {
                        ("Installing update…".to_string(), PillStyle::Busy)
                    }
                    SelfUpdatePillState::ReadyToRestart => {
                        ("Restart Paneflow".to_string(), PillStyle::Clickable)
                    }
                    SelfUpdatePillState::Errored => {
                        ("Update failed".to_string(), PillStyle::Clickable)
                    }
                },
                UpdatePillKind::SystemManaged(kind) => {
                    let label = match kind {
                        SystemPackageKind::RpmOstree => "Update via rpm-ostree".to_string(),
                        SystemPackageKind::Other => "Update via package manager".to_string(),
                    };
                    (label, PillStyle::SystemHint)
                }
            };

            // Leading icon. Clickable renders `download.svg` for the
            // pre-install CTA and `refresh.svg` for the post-install
            // "Restart for vX" CTA so the user has a visual cue that the
            // heavy work is already done; Busy renders a `loader-circle.svg`
            // arc continuously rotating via GPUI's declarative
            // Animation+Transformation API (one full revolution per second,
            // repeat forever). Pattern mirrors
            // `crates/gpui/examples/animation.rs` in the upstream Zed repo.
            let is_ready_to_restart = matches!(
                info.kind,
                UpdatePillKind::InApp(SelfUpdatePillState::ReadyToRestart)
            );
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
                    .path(if is_ready_to_restart {
                        "icons/refresh.svg"
                    } else {
                        "icons/download.svg"
                    })
                    .text_color(ui.muted)
                    .into_any_element(),
                PillStyle::SystemHint => svg()
                    .size(px(11.))
                    .flex_none()
                    .path("icons/tool.svg")
                    .text_color(ui.muted)
                    .into_any_element(),
            };

            // The pill sits inside the title bar's `WindowControlArea::Drag`
            // region declared on the parent. GPUI's hit-testing picks the
            // most-specific element on hover, so `cursor_pointer()` on the
            // pill itself overrides the parent's cursor without needing
            // to detach from the drag area; the `cx.stop_propagation()`
            // mouse-down further down ensures the click doesn't trigger
            // a window drag. Same idiom Zed's `ButtonLike` relies on.
            // US-007 AC3: a small `×` dismiss affordance on the
            // non-busy states. We deliberately omit it during
            // Downloading/Installing/ReadyToRestart — those have a
            // user-perceivable side effect already in flight (or
            // sitting one click away from `cx.restart()`); a stray
            // dismiss there would be jarring. Errored remains
            // dismissable so a user with a chronic install failure
            // can hide the pill without having to bounce the app.
            let pill_dismissable = matches!(
                info.kind,
                UpdatePillKind::InApp(SelfUpdatePillState::Idle | SelfUpdatePillState::Errored)
                    | UpdatePillKind::SystemManaged(_)
            );

            let mut pill = div()
                .id("update-pill")
                .ml_auto()
                .mr_2()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .px(px(8.))
                // Match the CLI/Agents toggle pill outer height
                // (1 border + 2 py + 18 segment + 2 py + 1 border = 24 px).
                .h(px(24.))
                .rounded(px(6.))
                .border_1()
                .border_color(ui.border)
                .bg(ui.subtle)
                .text_color(ui.text)
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(leading_icon)
                .child(label);

            if pill_dismissable {
                let muted = ui.muted;
                let text = ui.text;
                pill = pill.child(
                    div()
                        .id("update-pill-dismiss")
                        .ml(px(2.))
                        .px(px(4.))
                        .text_color(muted)
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .cursor_pointer()
                        .hover(move |s| s.text_color(text))
                        // stop_propagation on BOTH mouse-down and click
                        // so the click never reaches the parent pill's
                        // `on_click` handler that dispatches
                        // `StartSelfUpdate` — otherwise hitting the `×`
                        // would (a) dismiss the pill (b) immediately
                        // start the update we just dismissed.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(|_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(Box::new(crate::DismissUpdate), cx);
                        })
                        .child("×"),
                );
            }
            match style {
                // Dispatch on mouse-DOWN, not on click (mouse-up). At cold
                // start the update check resolves before the user has
                // touched the window, so the very first press on the pill
                // happens against a window the compositor still considers
                // inactive (Wayland focus-stealing prevention often
                // rejects `cx.activate(true)`) and a focus chain that
                // isn't yet initialized. In that state, `on_click`
                // (which needs a matched press+release pair routed
                // through the focus chain) silently drops the first
                // interaction; the user has to click elsewhere to wake
                // the chain, then re-click. Press-based dispatch avoids
                // both races and matches the title-bar button idiom in
                // Zed/VS Code/Discord. The pkexec modal confirms the
                // action, so we don't lose "drag-out to cancel".
                PillStyle::Clickable => {
                    pill = pill
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.surface).border_color(ui.muted)
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(Box::new(crate::StartSelfUpdate), cx);
                        });
                }
                PillStyle::Busy => {
                    pill = pill.opacity(0.7);
                }
                // SystemHint is a button that copies the upgrade command
                // to the clipboard via a toast. It's still a button, so
                // the cursor must hint that — `cursor_pointer()` matches
                // every other clickable surface in the chrome.
                PillStyle::SystemHint => {
                    pill = pill
                        .opacity(0.8)
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.surface).border_color(ui.muted).opacity(1.0)
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
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
            .child(brand)
            .child(content)
            .children(update_pill)
            // Agents-view toggle sits where the profile button used to;
            // the profile affordance has been removed.
            .child(render_agents_toggle_button(self, cx))
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
    }
}

/// US-009 (prd-agents-view.md): toggle icon for the Agents view.
/// Shows the *destination* of the click: a chat-style sessions icon
/// when in CLI mode (click = go to Agents), a terminal icon when
/// already in Agents mode (click = go back to CLI). The tooltip
/// resolves the active binding via the `agents_view_shortcut` field
/// PaneFlowApp pushes every render -- a user-remapped binding shows
/// the user's chosen key, not the hardcoded default.
fn render_agents_toggle_button(title_bar: &TitleBar, cx: &mut Context<TitleBar>) -> AnyElement {
    use paneflow_config::schema::AppMode;
    let ui = crate::theme::ui_colors();
    let in_agents = matches!(title_bar.mode, AppMode::Agents);
    let shortcut_suffix = title_bar
        .agents_view_shortcut
        .as_deref()
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    let tooltip_text = format!("Toggle Agents view{shortcut_suffix}");

    let segment = |label: &'static str, is_active: bool, id: &'static str| {
        let mut seg = div()
            .id(id)
            .px(px(8.))
            .py(px(0.))
            .h(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM);
        if is_active {
            // Active segment: subtle dark grey (`#3a3a3a`) — one step
            // lighter than the outer pill's `ui.surface`, so the
            // active mode reads through quiet contrast rather than a
            // loud chip. Same segmented-control language as Linear /
            // Vercel / Cursor. White text for max legibility.
            seg = seg.bg(rgb(0x3a3a3a)).text_color(ui.text);
        } else {
            seg = seg
                .text_color(ui.muted)
                .cursor_pointer()
                .hover(|s| {
                    let ui = crate::theme::ui_colors();
                    s.text_color(ui.text)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|_, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    cx.emit(TitleBarEvent::ToggleAgentsView);
                }));
        }
        seg.child(label).into_any_element()
    };

    div()
        .id("title-bar-mode-toggle")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .ml(px(8.))
        .px(px(2.))
        .py(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(ui.border)
        .bg(ui.surface)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .tooltip(move |_window, cx| {
            cx.new(|_| AgentsTooltip {
                text: tooltip_text.clone(),
            })
            .into()
        })
        .child(segment("CLI", !in_agents, "title-bar-mode-cli"))
        .child(segment("Agents", in_agents, "title-bar-mode-agents"))
        .into_any_element()
}

/// US-009 (prd-agents-view.md): minimal styled tooltip card. GPUI's
/// `.tooltip()` builder returns an [`gpui::AnyView`], so we need
/// *some* Entity to render the hover text -- Paneflow has no shared
/// tooltip widget yet (Zed's `ui::Tooltip` lives in a crate we don't
/// pull in). This struct is intentionally trivial; if a project-wide
/// tooltip primitive emerges, swap callers to it and delete.
struct AgentsTooltip {
    text: String,
}

impl Render for AgentsTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors();
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(theme.title_bar_background)
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .text_size(px(11.))
            .child(self.text.clone())
    }
}
