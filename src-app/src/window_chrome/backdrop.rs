//! Windows system backdrop integration.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_MAINWINDOW: u32 = 2;

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: isize,
        attribute: u32,
        value: *const std::ffi::c_void,
        value_size: u32,
    ) -> i32;
}

/// Requests standard Mica on Windows 11.
///
/// Mica samples the desktop wallpaper rather than other windows behind
/// PaneFlow. Windows 10 returns an error for this attribute and keeps the
/// Acrylic effect selected through GPUI's `WindowBackgroundAppearance::Blurred`.
pub(crate) fn apply_wallpaper_mica(window: &gpui::Window) {
    let Ok(window_handle) = HasWindowHandle::window_handle(window) else {
        log::warn!("Could not obtain the Win32 window handle for Mica");
        return;
    };
    let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
        log::warn!("PaneFlow received a non-Win32 window handle on Windows");
        return;
    };

    let backdrop = DWMSBT_MAINWINDOW;
    let result = unsafe {
        DwmSetWindowAttribute(
            handle.hwnd.get(),
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&backdrop as *const u32).cast(),
            std::mem::size_of_val(&backdrop) as u32,
        )
    };

    if result < 0 {
        log::debug!("Mica is unavailable; retaining the GPUI backdrop");
    }
}
