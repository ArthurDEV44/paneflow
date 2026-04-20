//! Render shortcut entries for the settings UI + menu bar.

use std::collections::HashMap;

use gpui::Keystroke;

use super::defaults::{DEFAULTS, MACOS_ONLY_DEFAULTS};
use super::registry::{ACTIONS, action_description};

/// A resolved shortcut entry for display in the settings page.
pub struct ShortcutEntry {
    pub key: String,
    pub description: String,
}

/// Format a GPUI keystroke string for display.
///
/// On Linux: `"secondary-shift-d"` → `"Ctrl+Shift+D"` (readable, plus-separated).
/// On macOS: `"secondary-shift-d"` → `"⌘⇧D"` (Apple HIG glyphs, no separator —
/// matches the native macOS menu bar convention consumed by US-012).
///
/// `secondary` is GPUI's cross-platform shorthand that resolves to `cmd` on
/// macOS and `ctrl` elsewhere (see `Keystroke::parse`). Rendering it here
/// mirrors that resolution so the menu bar always shows the actual key the
/// user will press.
pub fn format_keystroke(key: &str) -> String {
    let is_macos = cfg!(target_os = "macos");
    let parts = key.split('-').map(|part| match part {
        // Modifiers — platform-dependent rendering.
        "secondary" => {
            if is_macos {
                "\u{2318}".to_string() // ⌘
            } else {
                "Ctrl".to_string()
            }
        }
        "cmd" | "super" | "win" => {
            if is_macos {
                "\u{2318}".to_string() // ⌘
            } else {
                "Super".to_string()
            }
        }
        "ctrl" => {
            if is_macos {
                "\u{2303}".to_string() // ⌃
            } else {
                "Ctrl".to_string()
            }
        }
        "shift" => {
            if is_macos {
                "\u{21E7}".to_string() // ⇧
            } else {
                "Shift".to_string()
            }
        }
        "alt" => {
            if is_macos {
                "\u{2325}".to_string() // ⌥
            } else {
                "Alt".to_string()
            }
        }
        // Non-modifier tokens — same on both platforms, just capitalized.
        "tab" => "Tab".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        other => other.to_uppercase(),
    });
    if is_macos {
        // Apple HIG: modifier glyphs flow directly into the key label, no `+`.
        parts.collect::<String>()
    } else {
        parts.collect::<Vec<_>>().join("+")
    }
}

/// Compute the effective shortcut list by merging defaults with user overrides.
///
/// User overrides replace default bindings for the same action. Additional user
/// bindings (new keys) are appended. Keys bound to `"none"` are excluded.
pub fn effective_shortcuts(user_shortcuts: &HashMap<String, String>) -> Vec<ShortcutEntry> {
    // Build reverse map: action_name → user key (last one wins if duplicates)
    let mut user_by_action: HashMap<&str, &str> = HashMap::new();
    for (key, action_name) in user_shortcuts {
        if action_name != "none" {
            user_by_action.insert(action_name.as_str(), key.as_str());
        }
    }

    // Collect keys that user has explicitly unbound
    let unbound_keys: std::collections::HashSet<&str> = user_shortcuts
        .iter()
        .filter(|(_, v)| v.as_str() == "none")
        .map(|(k, _)| k.as_str())
        .collect();

    let mut entries = Vec::new();

    // Defaults first, with user overrides applied. US-010: include the
    // macOS-only defaults so the settings page reflects cmd-c/cmd-v on
    // macOS (and stays unchanged on Linux where MACOS_ONLY_DEFAULTS is empty).
    for d in DEFAULTS.iter().chain(MACOS_ONLY_DEFAULTS.iter()) {
        // If this default key was unbound by user, skip it
        if unbound_keys.contains(d.key) {
            continue;
        }

        // If user overrode this action to a different key, use the user's key
        let key = if let Some(user_key) = user_by_action.get(d.action_name) {
            format_keystroke(user_key)
        } else {
            format_keystroke(d.key)
        };

        entries.push(ShortcutEntry {
            key,
            description: action_description(d.action_name).to_string(),
        });
    }

    // Add user bindings for actions not in defaults (if any)
    for (key, action_name) in user_shortcuts {
        if action_name == "none" {
            continue;
        }
        let is_default_action = DEFAULTS
            .iter()
            .chain(MACOS_ONLY_DEFAULTS.iter())
            .any(|d| d.action_name == action_name);
        if !is_default_action && ACTIONS.iter().any(|a| a.name == action_name) {
            entries.push(ShortcutEntry {
                key: format_keystroke(key),
                description: action_description(action_name).to_string(),
            });
        }
    }

    entries
}

/// Returns `true` if the keystroke is a bare modifier press (no actual key).
pub fn is_bare_modifier(keystroke: &Keystroke) -> bool {
    matches!(
        keystroke.key.as_str(),
        "shift" | "control" | "alt" | "platform" | "function"
    )
}

/// Look up the action name for the binding at `index` in `DEFAULTS`.
///
/// Note: this indexes the platform-independent `DEFAULTS` slice only; it does
/// not cover `MACOS_ONLY_DEFAULTS` (carried over from the pre-US-022 layout).
pub fn action_name_at(index: usize) -> Option<&'static str> {
    DEFAULTS.get(index).map(|d| d.action_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_shortcuts_defaults_include_core_actions() {
        let entries = effective_shortcuts(&HashMap::new());
        let descriptions: Vec<&str> = entries.iter().map(|e| e.description.as_str()).collect();
        assert!(
            descriptions.contains(&"Split horizontal"),
            "Missing split horizontal"
        );
        assert!(
            descriptions.contains(&"Split vertical"),
            "Missing split vertical"
        );
        assert!(descriptions.contains(&"Close pane"), "Missing close pane");
        assert!(
            descriptions.contains(&"Next workspace"),
            "Missing next workspace"
        );
        assert!(descriptions.contains(&"Focus left"), "Missing focus left");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn effective_shortcuts_user_override_replaces_key() {
        let mut overrides = HashMap::new();
        overrides.insert("ctrl-alt-h".to_string(), "split_horizontally".to_string());
        let entries = effective_shortcuts(&overrides);
        let split_h = entries
            .iter()
            .find(|e| e.description == "Split horizontal")
            .expect("Split horizontal should be in effective list");
        assert_eq!(
            split_h.key, "Ctrl+Alt+H",
            "User override should replace the default key"
        );
    }

    #[test]
    fn effective_shortcuts_none_unbinds_key() {
        let mut overrides = HashMap::new();
        // US-009: default is now `secondary-shift-d`; unbinding requires the
        // canonical default key string.
        overrides.insert("secondary-shift-d".to_string(), "none".to_string());
        let entries = effective_shortcuts(&overrides);
        let has_split_h = entries.iter().any(|e| e.description == "Split horizontal");
        assert!(
            !has_split_h,
            "Unbound default binding should not appear in effective list"
        );
    }

    #[test]
    fn effective_shortcuts_invalid_action_ignored() {
        let mut overrides = HashMap::new();
        overrides.insert("ctrl+x".to_string(), "bogus_action".to_string());
        let entries = effective_shortcuts(&overrides);
        // Invalid action should not appear
        let has_bogus = entries
            .iter()
            .any(|e| e.description == "Unknown" && e.key == "Ctrl+X");
        assert!(!has_bogus, "Invalid action should not be in effective list");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn effective_shortcuts_preserves_unoverridden_defaults() {
        let mut overrides = HashMap::new();
        overrides.insert("ctrl+alt+h".to_string(), "split_horizontally".to_string());
        let entries = effective_shortcuts(&overrides);
        // close_pane should still be at its default key. US-009: default is
        // `secondary-shift-w`, which renders as "Ctrl+Shift+W" on Linux.
        let close = entries
            .iter()
            .find(|e| e.description == "Close pane")
            .expect("Close pane should be in effective list");
        assert_eq!(
            close.key, "Ctrl+Shift+W",
            "Unoverridden action should keep default key"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn format_keystroke_produces_readable_output() {
        assert_eq!(format_keystroke("ctrl-shift-d"), "Ctrl+Shift+D");
        assert_eq!(format_keystroke("alt-left"), "Alt+Left");
        assert_eq!(format_keystroke("ctrl-1"), "Ctrl+1");
        assert_eq!(format_keystroke("shift-pageup"), "Shift+PageUp");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn secondary_renders_as_ctrl_on_linux() {
        // AC2: secondary resolves to Ctrl on Linux; format_keystroke mirrors
        // that so the menu bar / shortcut list shows the key the user will
        // actually press.
        assert_eq!(format_keystroke("secondary-shift-d"), "Ctrl+Shift+D");
        assert_eq!(format_keystroke("secondary-tab"), "Ctrl+Tab");
        assert_eq!(format_keystroke("secondary-1"), "Ctrl+1");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn secondary_renders_as_cmd_glyph_on_macos() {
        // AC6: macOS menu bar expects Apple HIG glyphs, no plus separator.
        assert_eq!(format_keystroke("secondary-shift-d"), "\u{2318}\u{21E7}D");
        assert_eq!(format_keystroke("secondary-tab"), "\u{2318}Tab");
        assert_eq!(format_keystroke("secondary-1"), "\u{2318}1");
        // Explicit `cmd` token also renders as ⌘ (user override form from AC5).
        assert_eq!(format_keystroke("cmd-shift-d"), "\u{2318}\u{21E7}D");
    }
}
