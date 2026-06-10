//! Bottom-of-sidebar "Settings" footer + popover. Both the CLI and
//! the Agents sidebars host all of their top-level affordances inside
//! the same trigger: a single `Settings` button at the bottom of the
//! sidebar opens an upward popover with the relevant action list.
//!
//! The popover state (`PaneFlowApp::sidebar_actions_menu_open`) is
//! shared because only one sidebar is rendered at a time (mode toggle
//! swaps the whole sidebar tree).

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, SharedString, Styled, Transformation, div, percentage,
    prelude::*, px, svg,
};

use crate::PaneFlowApp;
use crate::window_chrome::title_bar::{SelfUpdatePillState, SystemPackageKind, UpdatePillKind};

impl PaneFlowApp {
    /// Update CTA banner at the bottom of the sidebar, above the Settings
    /// trigger. Replaces the title-bar update pill in the cockpit modes
    /// (Cli/Agents), where the title bar is a rail-confined overlay with no
    /// room for pills. Same states, labels, icons, and dismiss rules as the
    /// title-bar pill (`title_bar.rs`); same mouse-DOWN dispatch (Wayland
    /// focus-stealing prevention silently drops the first on_click after a
    /// cold start — see the title-bar pill comment for the full story).
    /// `None` when no update is available.
    pub(crate) fn render_sidebar_update_banner(
        &self,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let info = self.update_pill_info()?;
        let ui = crate::theme::ui_colors();

        let (label, busy, system_hint): (String, bool, bool) = match info.kind {
            UpdatePillKind::InApp(state) => match state {
                SelfUpdatePillState::Idle => (format!("v{} available", info.version), false, false),
                SelfUpdatePillState::Downloading => ("Downloading update…".into(), true, false),
                SelfUpdatePillState::Installing => ("Installing update…".into(), true, false),
                SelfUpdatePillState::ReadyToRestart => ("Restart Paneflow".into(), false, false),
                SelfUpdatePillState::Errored => ("Update failed".into(), false, false),
            },
            UpdatePillKind::SystemManaged(kind) => {
                let label = match kind {
                    SystemPackageKind::RpmOstree => "Update via rpm-ostree".to_string(),
                    SystemPackageKind::Other => "Update via package manager".to_string(),
                };
                (label, false, true)
            }
        };
        let is_ready_to_restart = matches!(
            info.kind,
            UpdatePillKind::InApp(SelfUpdatePillState::ReadyToRestart)
        );
        let dismissable = matches!(
            info.kind,
            UpdatePillKind::InApp(SelfUpdatePillState::Idle | SelfUpdatePillState::Errored)
                | UpdatePillKind::SystemManaged(_)
        );

        let leading_icon: AnyElement = if busy {
            svg()
                .size(px(14.))
                .flex_none()
                .path("icons/loader-circle.svg")
                .text_color(ui.muted)
                .with_animation(
                    "sidebar-update-spinner",
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
        } else {
            svg()
                .size(px(14.))
                .flex_none()
                .path(if system_hint {
                    "icons/tool.svg"
                } else if is_ready_to_restart {
                    "icons/refresh.svg"
                } else {
                    "icons/download.svg"
                })
                .text_color(ui.muted)
                .into_any_element()
        };

        let mut banner = div()
            .id("sidebar-update-banner")
            .mx(px(6.))
            .mb(px(2.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.subtle)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(leading_icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(label),
            );

        if dismissable {
            let muted = ui.muted;
            let text = ui.text;
            banner = banner.child(
                div()
                    .id("sidebar-update-dismiss")
                    .px(px(4.))
                    .text_color(muted)
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(text))
                    // stop_propagation on BOTH mouse-down and click so the
                    // press never reaches the banner's StartSelfUpdate
                    // dispatch — hitting × must not start the update it
                    // just dismissed.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(crate::DismissUpdate), cx);
                    })
                    .child("×"),
            );
        }

        if busy {
            banner = banner.opacity(0.7);
        } else {
            banner = banner
                .cursor_pointer()
                .when(system_hint, |d| d.opacity(0.8))
                .hover(move |s| {
                    let ui = crate::theme::ui_colors();
                    let s = s.bg(ui.surface).border_color(ui.muted);
                    if system_hint { s.opacity(1.0) } else { s }
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(crate::StartSelfUpdate), cx);
                });
        }

        Some(banner.into_any_element())
    }

    /// "IPC offline" notice at the bottom of the sidebar — the cockpit home
    /// of the title-bar IPC pill (same rail-confinement story as the update
    /// banner). Purely informational, like the original pill: no click
    /// handler. `None` while the IPC server is up.
    pub(crate) fn render_sidebar_ipc_banner(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.ipc_status.state() != crate::ipc::IpcState::Disabled {
            return None;
        }
        let ui = crate::theme::ui_colors();
        Some(
            div()
                .id("sidebar-ipc-banner")
                .mx(px(6.))
                .mb(px(2.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(6.))
                .border_1()
                .border_color(ui.border)
                .bg(ui.subtle)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(
                    svg()
                        .size(px(14.))
                        .flex_none()
                        .path("icons/triangle-alert.svg")
                        .text_color(ui.muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(ui.text)
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child("IPC offline"),
                )
                .into_any_element(),
        )
    }

    /// Render the bottom Settings trigger + the popover overlay that
    /// opens upward when `sidebar_actions_menu_open` is true. Wrap the
    /// result inside a `relative()` container at the bottom of the
    /// sidebar's flex column.
    pub(crate) fn render_sidebar_settings_footer(
        &self,
        items: Vec<SidebarMenuItem>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        let is_open = self.agents_view.sidebar_actions_menu_open;

        let trigger = div()
            .id("sidebar-settings-trigger")
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .when(is_open, |d| d.bg(ui.subtle))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.agents_view.sidebar_actions_menu_open =
                    !this.agents_view.sidebar_actions_menu_open;
                cx.notify();
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/settings.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .truncate()
                    .child("Settings"),
            );

        let popover: Option<AnyElement> = if is_open {
            // Vertical menu opening upward from the trigger. Each row
            // matches the sidebar's row language (icon + label, ghost
            // hover). Background uses the panel surface tint so the
            // popover separates from the sidebar's `title_bar_background`.
            let mut menu = div()
                .id("sidebar-settings-popover")
                .absolute()
                .left(px(6.))
                .right(px(6.))
                // 38px = trigger total height (px8/py6 + 14px icon) plus
                // a 6px gap. Adjust if the trigger padding changes.
                .bottom(px(38.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .p(px(4.))
                .rounded(px(8.))
                .bg(theme.title_bar_background)
                .border_1()
                .border_color(ui.border)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.25),
                    offset: gpui::point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                // Click anywhere outside the popover (or its trigger)
                // dismisses it. Same pattern as `profile_menu.rs:128`.
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.agents_view.sidebar_actions_menu_open {
                        this.agents_view.sidebar_actions_menu_open = false;
                        cx.notify();
                    }
                }));
            for item in items {
                menu = menu.child(render_menu_item(item, ui, cx));
            }
            Some(menu.into_any_element())
        } else {
            None
        };

        let mut footer = div().relative().flex_none().py(px(6.));
        if let Some(popover) = popover {
            footer = footer.child(popover);
        }
        // Cockpit homes of the old title-bar pills, right above the Settings
        // trigger, shared by Cli + Agents: the "IPC offline" notice first,
        // then the update CTA banner.
        if let Some(banner) = self.render_sidebar_ipc_banner(cx) {
            footer = footer.child(banner);
        }
        if let Some(banner) = self.render_sidebar_update_banner(cx) {
            footer = footer.child(banner);
        }
        footer.child(trigger).into_any_element()
    }

    /// CLI / Diff / Agents mode picker, rendered at the very bottom of each
    /// sidebar (below the Settings footer). Codex redesign: a floating
    /// fully-rounded pill (the ChatGPT model-picker language) — `ui.surface`
    /// container with a faint hairline, the active segment a complete
    /// brighter pill (#323232, the same "brightest fill" as the selected
    /// workspace card), each segment carrying a small mode icon.
    pub(crate) fn render_mode_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        use paneflow_config::schema::AppMode;
        let ui = crate::theme::ui_colors();
        let mode = self.mode;

        let inactive_text = ui.text.opacity(0.45);
        let inactive_hover_text = ui.text.opacity(0.85);

        // US-002 (prd-git-diff-mode-2026-Q3.md): the control grew from a
        // hardcoded 2-way CLI/Agents toggle to N segments, so each
        // inactive segment carries its own `activate` callback instead
        // of one shared `match self.mode`. Only inactive segments are
        // clickable; switching modes means clicking a different segment.
        type Activate = Box<dyn Fn(&mut PaneFlowApp, &mut gpui::Window, &mut Context<PaneFlowApp>)>;

        let segment = |label: &'static str,
                       icon: &'static str,
                       is_active: bool,
                       id: &'static str,
                       activate: Activate| {
            let mut seg = div()
                .id(id)
                .flex_1()
                .h(px(24.))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .rounded_full()
                .text_size(px(11.));
            if is_active {
                seg = seg
                    .bg(gpui::rgb(0x323232))
                    .text_color(ui.text)
                    .font_weight(FontWeight::SEMIBOLD);
            } else {
                seg = seg
                    .text_color(inactive_text)
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(inactive_hover_text))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, w, cx| {
                        cx.stop_propagation();
                        activate(this, w, cx);
                    }));
            }
            seg.child(
                svg()
                    .size(px(13.))
                    .flex_none()
                    .path(icon)
                    .text_color(if is_active { ui.text } else { inactive_text }),
            )
            .child(label)
            .into_any_element()
        };

        div()
            .id("sidebar-mode-toggle")
            .mx(px(6.))
            .mb(px(6.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .p(px(3.))
            .rounded_full()
            .bg(ui.surface)
            .border_1()
            .border_color(ui.border.opacity(0.5))
            .child(segment(
                "CLI",
                "icons/terminal.svg",
                matches!(mode, AppMode::Cli),
                "sidebar-mode-cli",
                Box::new(|this, w, cx| this.enter_cli_mode(w, cx)),
            ))
            .child(segment(
                "Diff",
                "icons/git-pull-request.svg",
                matches!(mode, AppMode::Diff),
                "sidebar-mode-diff",
                Box::new(|this, _w, cx| this.enter_diff_mode(cx)),
            ))
            .child(segment(
                "Agents",
                "icons/sessions.svg",
                matches!(mode, AppMode::Agents),
                "sidebar-mode-agents",
                Box::new(|this, _w, cx| this.enter_agents_mode(cx)),
            ))
            .into_any_element()
    }
}

pub(crate) type SidebarMenuAction =
    Box<dyn Fn(&mut PaneFlowApp, &mut gpui::Window, &mut Context<PaneFlowApp>) + 'static>;

/// A single action row in the Settings popover.
pub(crate) struct SidebarMenuItem {
    pub id: SharedString,
    pub icon: &'static str,
    pub label: SharedString,
    pub on_click: SidebarMenuAction,
}

fn render_menu_item(
    item: SidebarMenuItem,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let handler = item.on_click;
    div()
        .id(item.id)
        .px(px(8.))
        .py(px(6.))
        .rounded(px(5.))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .on_click(cx.listener(move |this, _: &ClickEvent, w, cx| {
            handler(this, w, cx);
            this.agents_view.sidebar_actions_menu_open = false;
            cx.notify();
        }))
        .child(
            svg()
                .size(px(14.))
                .flex_none()
                .path(item.icon)
                .text_color(ui.muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .font_weight(FontWeight::NORMAL)
                .text_color(ui.text)
                .truncate()
                .child(item.label),
        )
        .into_any_element()
}
