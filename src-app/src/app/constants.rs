//! Layout & timing constants shared across the app shell.
//!
//! Extracted from `main.rs` per US-002 (anti edit-thrashing). All items
//! are `pub(crate)` and re-exported at the crate root via `main.rs` so
//! existing `crate::SIDEBAR_WIDTH` / `crate::TOAST_HOLD_MS` references in
//! sibling modules keep compiling without import churn.

use gpui::{Hsla, Pixels, WindowBackgroundAppearance, px};

/// Sidebar width in pixels — shared between sidebar and title bar for alignment.
pub(crate) const SIDEBAR_WIDTH: f32 = 240.;

/// Original solid PaneFlow chrome tint used while the window is inactive.
const CHROME_INACTIVE_TINT: u32 = 0x141414;
/// Linux-only: opacity of the neutral `#141414` veil applied over native blur
/// while the window is active (see `cockpit_chrome_background`). Only referenced
/// from the `#[cfg(target_os = "linux")]` branch, so gate the declaration too —
/// otherwise it reads as dead code on the Windows/macOS builds.
#[cfg(target_os = "linux")]
const LINUX_CHROME_ACTIVE_OPACITY: f32 = 0.72;

/// Source tint for selected and hovered sidebar tabs.
const SIDEBAR_TAB_TINT: u32 = 0x30373c;
const SIDEBAR_TAB_ACTIVE_OPACITY: f32 = 0.82;
const SIDEBAR_TAB_HOVER_OPACITY: f32 = 0.55;

/// Generous circular radius approximating Codex's continuous Apple-style corners.
pub(crate) const SIDEBAR_TAB_CORNER_RADIUS: Pixels = px(9.);

/// Native material used behind the main application window.
///
/// Windows delegates to GPUI's system backdrop support. On macOS PaneFlow
/// installs a semantic AppKit sidebar material after the native window opens.
/// Linux starts opaque and switches to a transparent surface only after the
/// compositor advertises a supported blur protocol.
pub(crate) fn window_background_appearance() -> WindowBackgroundAppearance {
    #[cfg(target_os = "windows")]
    {
        if windows_supports_system_backdrop() {
            WindowBackgroundAppearance::MicaBackdrop
        } else {
            WindowBackgroundAppearance::Blurred
        }
    }

    #[cfg(target_os = "macos")]
    {
        WindowBackgroundAppearance::Transparent
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        WindowBackgroundAppearance::Opaque
    }
}

#[cfg(target_os = "windows")]
fn windows_supports_system_backdrop() -> bool {
    #[repr(C)]
    struct RtlOsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        service_pack: [u16; 128],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut RtlOsVersionInfo) -> i32;
    }

    let mut version = RtlOsVersionInfo {
        size: std::mem::size_of::<RtlOsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        service_pack: [0; 128],
    };

    // NTSTATUS values greater than or equal to zero indicate success.
    unsafe { RtlGetVersion(&mut version) >= 0 && version.build >= 22_621 }
}

/// Background used by the title bar and navigation rails.
///
/// Windows keeps PaneFlow's original solid `#141414` while inactive and leaves
/// active chrome transparent for standard Mica. macOS always stays transparent
/// so AppKit's semantic material can perform its native, subtler active/inactive
/// transition. Linux applies a neutral `#141414` veil only when native blur is
/// confirmed, keeping colorful wallpapers subtle and readable.
pub(crate) fn cockpit_chrome_background(background: Hsla, is_window_active: bool) -> Hsla {
    if cfg!(target_os = "macos") {
        return gpui::transparent_black();
    } else if cfg!(target_os = "windows") {
        return if is_window_active {
            gpui::transparent_black()
        } else {
            Hsla::from(gpui::rgb(CHROME_INACTIVE_TINT))
        };
    }

    #[cfg(target_os = "linux")]
    if crate::window_chrome::linux_backdrop::native_blur_active() {
        return if is_window_active {
            Hsla::from(gpui::rgb(CHROME_INACTIVE_TINT)).opacity(LINUX_CHROME_ACTIVE_OPACITY)
        } else {
            Hsla::from(gpui::rgb(CHROME_INACTIVE_TINT))
        };
    }

    background
}

/// Window-level backdrop behind the translucent chrome.
pub(crate) fn cockpit_backdrop_background(background: Hsla) -> Hsla {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        return gpui::transparent_black();
    }

    #[cfg(target_os = "linux")]
    if crate::window_chrome::linux_backdrop::native_blur_active() {
        return gpui::transparent_black();
    }

    background
}

/// Background for the selected tab in the CLI and Agents sidebars.
pub(crate) fn sidebar_tab_active_background() -> Hsla {
    Hsla::from(gpui::rgb(SIDEBAR_TAB_TINT)).opacity(SIDEBAR_TAB_ACTIVE_OPACITY)
}

/// Background for a hovered, non-selected sidebar tab.
pub(crate) fn sidebar_tab_hover_background() -> Hsla {
    Hsla::from(gpui::rgb(SIDEBAR_TAB_TINT)).opacity(SIDEBAR_TAB_HOVER_OPACITY)
}

/// Claude Code spinner glyphs — same characters Claude renders in the terminal.
/// Claude Code keeps this unique glyph spinner; every other agent uses the
/// rotating `loader-circle.svg` arc (shared with the Agents sidebar).
pub(crate) const CLAUDE_SPINNER_FRAMES: [char; 6] = ['·', '✻', '✽', '✶', '✳', '✢'];

/// Toast animation durations (ms). The `hold_ms` carried on each `Toast`
/// must match the dismiss timer in `push_toast` — otherwise the exit
/// animation plays early and the element persists as a ghost.
pub(crate) const TOAST_ENTER_MS: u64 = 180;
pub(crate) const TOAST_HOLD_MS: u64 = 1440;
pub(crate) const TOAST_EXIT_MS: u64 = 180;

/// Maximum number of closed-pane records kept for undo-close-pane (US-014).
pub(crate) const MAX_CLOSED_PANES: usize = 5;

/// Width of the invisible border zone used for CSD edge/corner resize handles.
pub(crate) const RESIZE_BORDER: Pixels = px(10.0);
