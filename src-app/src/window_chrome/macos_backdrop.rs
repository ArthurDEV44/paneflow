//! Native macOS material for PaneFlow's unified sidebar and title bar.
#![allow(deprecated, unexpected_cfgs)]

use cocoa::{
    appkit::{
        NSAppearance, NSAppearanceNameVibrantDark, NSAppearanceNameVibrantLight, NSView,
        NSViewHeightSizable, NSViewWidthSizable, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    },
    base::{id, nil},
};
use gpui::WindowBackgroundAppearance;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;

thread_local! {
    static SIDEBAR_MATERIAL: RefCell<Option<SidebarMaterial>> = const { RefCell::new(None) };
}

struct SidebarMaterial {
    effect_view: id,
    is_light: bool,
}

/// Installs AppKit's semantic sidebar material below GPUI's renderer.
///
/// The material spans the full content view, including the transparent native
/// title bar. PaneFlow's opaque terminal surface hides it everywhere except the
/// navigation chrome. AppKit owns focus dimming and accessibility adaptation.
pub(crate) fn apply_subtle_sidebar_material(window: &gpui::Window, is_light: bool) {
    match try_apply_subtle_sidebar_material(window, is_light) {
        Ok(effect_view) => {
            SIDEBAR_MATERIAL.with(|slot| {
                *slot.borrow_mut() = Some(SidebarMaterial {
                    effect_view,
                    is_light,
                });
            });
        }
        Err(error) => {
            log::warn!("Could not install the native macOS sidebar material: {error}");
            window.set_background_appearance(WindowBackgroundAppearance::Blurred);
        }
    }
}

/// Keeps AppKit vibrancy aligned with PaneFlow rather than macOS's appearance.
pub(crate) fn sync_subtle_sidebar_material_theme(is_light: bool) {
    SIDEBAR_MATERIAL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(material) = slot.as_mut() else {
            return;
        };
        if material.is_light == is_light {
            return;
        }

        set_material_appearance(material.effect_view, is_light);
        material.is_light = is_light;
    });
}

// The `objc` 0.2 `msg_send!` macro expands to `cfg(feature = "cargo-clippy")`,
// which rustc's `unexpected_cfgs` check flags in this (the destination) crate
// since we declare no such feature. The cfg is internal to the macro, not ours
// — silence it for the only function that invokes `msg_send!`.
#[allow(unexpected_cfgs)]
fn try_apply_subtle_sidebar_material(
    window: &gpui::Window,
    is_light: bool,
) -> Result<id, &'static str> {
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
        set_material_appearance(effect_view, is_light);

        let _: () = msg_send![
            content_view,
            addSubview: effect_view
            positioned: NSWindowOrderingMode::NSWindowBelow
            relativeTo: native_view
        ];
        let _: () = msg_send![effect_view, release];

        Ok(effect_view)
    }
}

fn set_material_appearance(effect_view: id, is_light: bool) {
    // SAFETY: this module only runs on AppKit's main thread. The semantic
    // vibrant appearances are designed for NSVisualEffectView materials.
    unsafe {
        let name = if is_light {
            NSAppearanceNameVibrantLight
        } else {
            NSAppearanceNameVibrantDark
        };
        NSView::setAppearance(effect_view, NSAppearance(name));
    }
}
