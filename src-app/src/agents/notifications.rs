//! Desktop notification routing for agent lifecycle events.
//!
//! This module owns both sides of the notification gate:
//! - process-wide focus/Agents-panel visibility flags updated by the GPUI app;
//! - a single cross-platform `notify-rust` firing path used by `ai.*` handlers.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::BackgroundExecutor;
use paneflow_config::schema::{AgentPanelConfig, NotifyWhenAgentWaiting, PaneFlowConfig};

/// Stable Windows AppUserModelID. The WiX Start Menu shortcut mirrors this in
/// `packaging/wix/main.wxs`; the dev/unpackaged path also registers it under
/// HKCU\Software\Classes\AppUserModelId before showing a toast.
#[cfg(target_os = "windows")]
const PANEFLOW_WINDOWS_AUMID: &str = "Strivex.PaneFlow";

/// Window-active gate updated by `cx.observe_window_activation`.
/// `true` while the OS reports the Paneflow window as the focused one.
static WINDOW_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Agents-panel gate updated by `app::agents_view_actions`. `true`
/// while the user is in `AppMode::Agents`.
static AGENTS_PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Update the window-active flag. Called from
/// `cx.observe_window_activation` and from the initial activation
/// tick that GPUI fires when the observer registers.
pub fn set_window_active(active: bool) {
    WINDOW_ACTIVE.store(active, Ordering::Relaxed);
}

/// Is the Paneflow window currently the focused surface?
pub fn window_active() -> bool {
    WINDOW_ACTIVE.load(Ordering::Relaxed)
}

/// Update the agents-panel-visible flag. Called from the mode toggle
/// and from the bootstrap when the persisted session restores into
/// agents mode.
pub fn set_agents_panel_visible(visible: bool) {
    AGENTS_PANEL_VISIBLE.store(visible, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopNotificationUrgency {
    Normal,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopNotification {
    summary: String,
    body: String,
    urgency: DesktopNotificationUrgency,
}

impl DesktopNotification {
    pub(crate) fn turn_finished(workspace_title: &str) -> Self {
        Self {
            summary: "Agent finished".to_string(),
            body: format!("{workspace_title} is ready."),
            urgency: DesktopNotificationUrgency::Normal,
        }
    }

    pub(crate) fn needs_input(workspace_title: &str, message: Option<&str>) -> Self {
        Self {
            summary: "Paneflow needs your input".to_string(),
            body: attention_notification_body(workspace_title, message),
            urgency: DesktopNotificationUrgency::Critical,
        }
    }

    pub(crate) fn agent_exited(workspace_title: &str, exit_code: i32) -> Self {
        Self {
            summary: "Agent exited unexpectedly".to_string(),
            body: agent_exit_notification_body(workspace_title, exit_code),
            urgency: DesktopNotificationUrgency::Critical,
        }
    }

    pub(crate) fn stalled(workspace_title: &str, silent_secs: u64) -> Self {
        Self {
            summary: "Agent may be stuck".to_string(),
            body: stalled_notification_body(workspace_title, silent_secs),
            urgency: DesktopNotificationUrgency::Critical,
        }
    }
}

/// Fire a best-effort desktop notification without blocking the GPUI thread.
pub(crate) fn fire_desktop_notification(
    notification: DesktopNotification,
    config: &PaneFlowConfig,
    source_visible: bool,
    executor: BackgroundExecutor,
) {
    let gate = config.agent_panel.as_ref().map_or(
        NotifyWhenAgentWaiting::PrimaryScreen,
        AgentPanelConfig::resolved_notify_when_agent_waiting,
    );
    if !should_fire_desktop_notification(gate, window_active(), source_visible) {
        return;
    }

    executor
        .spawn(async move {
            let _ = smol::unblock(move || show_desktop_notification(notification)).await;
        })
        .detach();
}

pub(crate) fn should_fire_desktop_notification(
    gate: NotifyWhenAgentWaiting,
    window_active: bool,
    _source_visible: bool,
) -> bool {
    match gate {
        NotifyWhenAgentWaiting::Never => false,
        NotifyWhenAgentWaiting::PrimaryScreen | NotifyWhenAgentWaiting::AllScreens => {
            !window_active
        }
    }
}

/// Bound + sanitize an agent question before it is stored on the session
/// and mirrored to notifications.
pub(crate) fn sanitize_notification_message(raw: &str) -> String {
    crate::markdown::strip_bidi_zero_width(raw.chars().take(512).collect())
}

pub(crate) fn attention_notification_body(workspace_title: &str, message: Option<&str>) -> String {
    match message.filter(|m| !m.trim().is_empty()) {
        Some(m) => format!("{workspace_title}: {m}"),
        None => format!("{workspace_title}: approval needed"),
    }
}

pub(crate) fn agent_exit_notification_body(workspace_title: &str, exit_code: i32) -> String {
    format!("{workspace_title}: agent exited with code {exit_code}")
}

pub(crate) fn stalled_notification_body(workspace_title: &str, silent_secs: u64) -> String {
    format!("{workspace_title}: no agent activity for {silent_secs} s")
}

fn show_desktop_notification(notification: DesktopNotification) -> Result<(), String> {
    let mut builder = notify_rust::Notification::new();
    builder
        .summary(&notification.summary)
        .body(&notification.body)
        .appname("Paneflow")
        .icon("paneflow")
        .timeout(std::time::Duration::from_secs(8));

    #[cfg(any(all(unix, not(target_os = "macos")), target_os = "windows"))]
    builder.urgency(notification_urgency_for_platform(notification.urgency));

    #[cfg(all(unix, not(target_os = "macos")))]
    builder.hint(notify_rust::Hint::DesktopEntry("paneflow".to_string()));

    #[cfg(target_os = "windows")]
    {
        let _ = ensure_windows_process_app_user_model_id();
        let _ = ensure_windows_app_user_model_id_registered();
        builder.app_id(PANEFLOW_WINDOWS_AUMID);
    }

    builder.show().map(|_| ()).map_err(|err| err.to_string())
}

#[cfg(any(all(unix, not(target_os = "macos")), target_os = "windows"))]
fn notification_urgency_for_platform(urgency: DesktopNotificationUrgency) -> notify_rust::Urgency {
    #[cfg(target_os = "windows")]
    {
        match urgency {
            DesktopNotificationUrgency::Normal => notify_rust::Urgency::Normal,
            DesktopNotificationUrgency::Critical => notify_rust::Urgency::Critical,
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        match urgency {
            DesktopNotificationUrgency::Normal => notify_rust::Urgency::Normal,
            DesktopNotificationUrgency::Critical => notify_rust::Urgency::Critical,
        }
    }
}

#[cfg(target_os = "windows")]
fn ensure_windows_process_app_user_model_id() -> Result<(), String> {
    let app_id = windows_wide_null(PANEFLOW_WINDOWS_AUMID);
    let result = unsafe {
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr())
    };
    if result < 0 {
        Err(format!(
            "SetCurrentProcessExplicitAppUserModelID({PANEFLOW_WINDOWS_AUMID}) returned HRESULT 0x{:08X}",
            result as u32
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn ensure_windows_app_user_model_id_registered() -> Result<(), String> {
    let key_path = format!(r"SOFTWARE\Classes\AppUserModelId\{PANEFLOW_WINDOWS_AUMID}");
    let key = windows_registry::CURRENT_USER
        .create(&key_path)
        .map_err(|err| format!("create HKCU\\{key_path}: {err}"))?;
    key.set_string("DisplayName", "Paneflow")
        .map_err(|err| format!("set DisplayName: {err}"))?;
    key.set_string("IconBackgroundColor", "0")
        .map_err(|err| format!("set IconBackgroundColor: {err}"))?;
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.display().to_string();
        key.set_string("IconUri", &exe)
            .map_err(|err| format!("set IconUri: {err}"))?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_gate_honors_never_and_window_focus() {
        assert!(!should_fire_desktop_notification(
            NotifyWhenAgentWaiting::Never,
            false,
            false
        ));
        assert!(!should_fire_desktop_notification(
            NotifyWhenAgentWaiting::PrimaryScreen,
            true,
            true
        ));
        assert!(
            !should_fire_desktop_notification(NotifyWhenAgentWaiting::PrimaryScreen, true, false),
            "active Paneflow window suppresses OS notifications even when the source is hidden"
        );
        assert!(
            should_fire_desktop_notification(NotifyWhenAgentWaiting::PrimaryScreen, false, true),
            "inactive Paneflow window still notifies even if the source surface would be visible"
        );
        assert!(should_fire_desktop_notification(
            NotifyWhenAgentWaiting::AllScreens,
            false,
            true
        ));
    }

    #[test]
    fn notification_message_is_bounded_and_bidi_stripped() {
        let spoofed = "Allow \u{202E}?fr- mr\u{202C} ?";
        let clean = sanitize_notification_message(spoofed);
        assert!(!clean.contains('\u{202E}'), "RLO stripped");
        assert!(!clean.contains('\u{202C}'), "PDF stripped");
        assert!(clean.contains("Allow"), "visible text kept: {clean}");

        let long = "é".repeat(600);
        assert_eq!(
            sanitize_notification_message(&long).chars().count(),
            512,
            "char-bounded, multibyte-safe"
        );
    }

    #[test]
    fn notification_bodies_are_specific_and_non_empty() {
        assert_eq!(
            attention_notification_body("backend", Some("Allow `cargo test`?")),
            "backend: Allow `cargo test`?"
        );
        assert_eq!(
            attention_notification_body("backend", None),
            "backend: approval needed"
        );
        assert_eq!(
            attention_notification_body("backend", Some("   ")),
            "backend: approval needed"
        );
        assert_eq!(
            agent_exit_notification_body("api", 1),
            "api: agent exited with code 1"
        );
        assert_eq!(
            stalled_notification_body("api", 300),
            "api: no agent activity for 300 s"
        );
    }

    #[test]
    fn desktop_notification_constructors_set_title_body_and_urgency() {
        let finished = DesktopNotification::turn_finished("backend");
        assert_eq!(finished.summary, "Agent finished");
        assert_eq!(finished.body, "backend is ready.");
        assert_eq!(finished.urgency, DesktopNotificationUrgency::Normal);

        let attention = DesktopNotification::needs_input("backend", Some("Approve edit?"));
        assert_eq!(attention.summary, "Paneflow needs your input");
        assert_eq!(attention.body, "backend: Approve edit?");
        assert_eq!(attention.urgency, DesktopNotificationUrgency::Critical);
    }
}
