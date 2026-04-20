//! Centralized keybinding management: defaults, user overrides, hot-reload.
//!
//! Uses Zed's clear → defaults → user-overrides pattern via GPUI's
//! `clear_key_bindings()` + `bind_keys()`.
//!
//! Module layout (US-022):
//! - [`registry`] — `ActionMeta` + `ACTIONS` table (one source of truth)
//! - [`defaults`] — `DEFAULTS` and `MACOS_ONLY_DEFAULTS` binding tables
//! - [`apply`] — registers bindings on the GPUI `App`
//! - [`display`] — formats keystrokes and builds the settings shortcut list

mod apply;
mod defaults;
mod display;
mod registry;

pub use apply::apply_keybindings;
pub use display::{ShortcutEntry, action_name_at, effective_shortcuts, is_bare_modifier};

// AC (US-022): `format_keystroke` is part of the public re-export surface even
// though no caller inside this binary currently uses it directly — it's there
// for the settings UI / menu-bar renderer to consume. `#[allow]` silences the
// binary-crate unused-import lint without dropping the public symbol.
#[allow(unused_imports)]
pub use display::format_keystroke;
