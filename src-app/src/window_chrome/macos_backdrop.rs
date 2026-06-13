//! Native macOS material for PaneFlow's unified sidebar and title bar.
#![allow(deprecated, unexpected_cfgs)]

use cocoa::{
    appkit::{
        NSView, NSViewHeightSizable, NSViewWidthSizable, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    },
    base::{id, nil},
};
use gpui::WindowBackgroundAppearance;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Installs AppKit's semantic sidebar material below GPUI's renderer.
///
/// The material spans the full content view, including the transparent native
/// title bar. PaneFlow's opaque terminal surface hides it everywhere except the
/// navigation chrome. AppKit owns focus dimming and accessibility adaptation.
pub(crate) fn apply_subtle_sidebar_material(window: &gpui::Window) {
    if let Err(error) = try_apply_subtle_sidebar_material(window) {
        log::warn!("Could not install the native macOS sidebar material: {error}");
        window.set_background_appearance(WindowBackgroundAppearance::Blurred);
    }
}

fn try_apply_subtle_sidebar_material(window: &gpui::Window) -> Result<(), &'static str> {
    let window_handle = HasWindowHandle::window_handle(window)
        .map_err(|_| "GPUI did not expose an AppKit window handle")?;
    let RawWindowHandle::AppKit(handle) = window_handle.as_raw() else {
        return Err("GPUI returned a non-AppKit window handle on macOS");
    };

    let native_view = handle.ns_view.as_ptr() as id;

    // SAFETY: GPUI invokes this callback on AppKit's main thread and the raw
    // handle guarantees that `native_view` remains a valid NSView for the
    // lifetime of `window`.
    unsafe {
        let content_view: id = msg_send![native_view, superview];
        if content_view == nil {
            return Err("GPUI's native view is not attached to an NSWindow");
        }

        let frame = NSView::bounds(content_view);
        let effect_view = NSVisualEffectView::initWithFrame_(NSVisualEffectView::alloc(nil), frame);
        if effect_view == nil {
            return Err("AppKit could not create NSVisualEffectView");
        }

        NSView::setAutoresizingMask_(effect_view, NSViewWidthSizable | NSViewHeightSizable);
        NSVisualEffectView::setMaterial_(effect_view, NSVisualEffectMaterial::Sidebar);
        NSVisualEffectView::setBlendingMode_(effect_view, NSVisualEffectBlendingMode::BehindWindow);
        NSVisualEffectView::setState_(effect_view, NSVisualEffectState::FollowsWindowActiveState);

        let _: () = msg_send![
            content_view,
            addSubview: effect_view
            positioned: NSWindowOrderingMode::NSWindowBelow
            relativeTo: native_view
        ];
        let _: () = msg_send![effect_view, release];
    }

    Ok(())
}
