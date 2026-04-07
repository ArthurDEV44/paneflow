//! Centralized keybinding management: defaults, user overrides, hot-reload.
//!
//! Uses Zed's clear → defaults → user-overrides pattern via GPUI's
//! `clear_key_bindings()` + `bind_keys()`.

use std::collections::HashMap;

use gpui::{Action, App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Keystroke};

use crate::{
    ClosePane, CloseTab, CloseWindow, CloseWorkspace, FocusDown, FocusLeft, FocusRight, FocusUp,
    LayoutEvenHorizontal, LayoutEvenVertical, LayoutMainVertical, LayoutTiled, NewTab,
    NewWorkspace, NextWorkspace, ScrollPageDown, ScrollPageUp, SelectWorkspace1, SelectWorkspace2,
    SelectWorkspace3, SelectWorkspace4, SelectWorkspace5, SelectWorkspace6, SelectWorkspace7,
    SelectWorkspace8, SelectWorkspace9, SplitHorizontally, SplitVertically, TerminalCopy,
    TerminalPaste, ToggleZoom,
};

/// A default keybinding entry: keystroke string, action name, GPUI context filter.
struct DefaultBinding {
    key: &'static str,
    action_name: &'static str,
    context: Option<&'static str>,
}

/// All default keybindings. Order matches the original registration order.
const DEFAULTS: &[DefaultBinding] = &[
    DefaultBinding {
        key: "ctrl-shift-d",
        action_name: "split_horizontally",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-e",
        action_name: "split_vertically",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-w",
        action_name: "close_pane",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-n",
        action_name: "new_workspace",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-q",
        action_name: "close_workspace",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-tab",
        action_name: "next_workspace",
        context: None,
    },
    DefaultBinding {
        key: "alt-left",
        action_name: "focus_left",
        context: None,
    },
    DefaultBinding {
        key: "alt-right",
        action_name: "focus_right",
        context: None,
    },
    DefaultBinding {
        key: "alt-up",
        action_name: "focus_up",
        context: None,
    },
    DefaultBinding {
        key: "alt-down",
        action_name: "focus_down",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-1",
        action_name: "select_workspace_1",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-2",
        action_name: "select_workspace_2",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-3",
        action_name: "select_workspace_3",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-4",
        action_name: "select_workspace_4",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-5",
        action_name: "select_workspace_5",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-6",
        action_name: "select_workspace_6",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-7",
        action_name: "select_workspace_7",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-8",
        action_name: "select_workspace_8",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-9",
        action_name: "select_workspace_9",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-t",
        action_name: "new_tab",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-w",
        action_name: "close_tab",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-c",
        action_name: "terminal_copy",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "ctrl-shift-v",
        action_name: "terminal_paste",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "shift-pageup",
        action_name: "scroll_page_up",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "shift-pagedown",
        action_name: "scroll_page_down",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "ctrl-shift-z",
        action_name: "toggle_zoom",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-1",
        action_name: "layout_even_horizontal",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-2",
        action_name: "layout_even_vertical",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-3",
        action_name: "layout_main_vertical",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-4",
        action_name: "layout_tiled",
        context: None,
    },
];

/// Resolve an action name string to a boxed GPUI action.
fn action_from_name(name: &str) -> Option<Box<dyn Action>> {
    Some(match name {
        "split_horizontally" => Box::new(SplitHorizontally),
        "split_vertically" => Box::new(SplitVertically),
        "close_pane" => Box::new(ClosePane),
        "new_workspace" => Box::new(NewWorkspace),
        "close_workspace" => Box::new(CloseWorkspace),
        "next_workspace" => Box::new(NextWorkspace),
        "focus_left" => Box::new(FocusLeft),
        "focus_right" => Box::new(FocusRight),
        "focus_up" => Box::new(FocusUp),
        "focus_down" => Box::new(FocusDown),
        "select_workspace_1" => Box::new(SelectWorkspace1),
        "select_workspace_2" => Box::new(SelectWorkspace2),
        "select_workspace_3" => Box::new(SelectWorkspace3),
        "select_workspace_4" => Box::new(SelectWorkspace4),
        "select_workspace_5" => Box::new(SelectWorkspace5),
        "select_workspace_6" => Box::new(SelectWorkspace6),
        "select_workspace_7" => Box::new(SelectWorkspace7),
        "select_workspace_8" => Box::new(SelectWorkspace8),
        "select_workspace_9" => Box::new(SelectWorkspace9),
        "new_tab" => Box::new(NewTab),
        "close_tab" => Box::new(CloseTab),
        "terminal_copy" => Box::new(TerminalCopy),
        "terminal_paste" => Box::new(TerminalPaste),
        "scroll_page_up" => Box::new(ScrollPageUp),
        "scroll_page_down" => Box::new(ScrollPageDown),
        "close_window" => Box::new(CloseWindow),
        "toggle_zoom" => Box::new(ToggleZoom),
        "layout_even_horizontal" => Box::new(LayoutEvenHorizontal),
        "layout_even_vertical" => Box::new(LayoutEvenVertical),
        "layout_main_vertical" => Box::new(LayoutMainVertical),
        "layout_tiled" => Box::new(LayoutTiled),
        _ => return None,
    })
}

/// Context for a given action name. Returns `Some("Terminal")` for terminal-scoped actions.
fn context_for_action(name: &str) -> Option<&'static str> {
    match name {
        "terminal_copy" | "terminal_paste" | "scroll_page_up" | "scroll_page_down" => {
            Some("Terminal")
        }
        _ => None,
    }
}

/// Build a `KeyBinding` from a boxed action, using `KeyBinding::load` to avoid
/// the `A: Action` bound on `KeyBinding::new`. Returns `None` on invalid keystroke.
fn make_binding(
    keystrokes: &str,
    action: Box<dyn Action>,
    context: Option<&str>,
) -> Option<KeyBinding> {
    let predicate = context.map(|ctx| {
        KeyBindingContextPredicate::parse(ctx)
            .expect("invalid context predicate")
            .into()
    });
    match KeyBinding::load(
        keystrokes,
        action,
        predicate,
        false,
        None,
        &DummyKeyboardMapper,
    ) {
        Ok(binding) => Some(binding),
        Err(e) => {
            log::warn!("shortcuts: invalid keystroke '{keystrokes}': {e}");
            None
        }
    }
}

/// Apply keybindings: clear all, register defaults, then layer user overrides.
///
/// User shortcuts map keystroke strings to action names. Special values:
/// - `"none"` — unbinds the key (no action registered for it)
/// - Any valid action name — overrides or adds a binding for that key
pub fn apply_keybindings(cx: &mut App, user_shortcuts: &HashMap<String, String>) {
    cx.clear_key_bindings();

    // Register defaults
    let default_bindings: Vec<KeyBinding> = DEFAULTS
        .iter()
        .filter_map(|d| {
            let action = action_from_name(d.action_name)?;
            make_binding(d.key, action, d.context)
        })
        .collect();
    cx.bind_keys(default_bindings);

    // Layer user overrides
    for (key, action_name) in user_shortcuts {
        if action_name == "none" {
            continue;
        }
        let Some(action) = action_from_name(action_name) else {
            log::warn!("shortcuts: unknown action '{action_name}' for key '{key}', skipping");
            continue;
        };
        let context = context_for_action(action_name);
        if let Some(binding) = make_binding(key, action, context) {
            cx.bind_keys([binding]);
        }
    }
}

/// A resolved shortcut entry for display in the settings page.
pub struct ShortcutEntry {
    pub key: String,
    pub description: String,
}

/// Human-readable description for an action name.
fn action_description(name: &str) -> &'static str {
    match name {
        "split_horizontally" => "Split horizontal",
        "split_vertically" => "Split vertical",
        "close_pane" => "Close pane",
        "new_workspace" => "New workspace",
        "close_workspace" => "Close workspace",
        "next_workspace" => "Next workspace",
        "focus_left" => "Focus left",
        "focus_right" => "Focus right",
        "focus_up" => "Focus up",
        "focus_down" => "Focus down",
        "select_workspace_1" => "Select workspace 1",
        "select_workspace_2" => "Select workspace 2",
        "select_workspace_3" => "Select workspace 3",
        "select_workspace_4" => "Select workspace 4",
        "select_workspace_5" => "Select workspace 5",
        "select_workspace_6" => "Select workspace 6",
        "select_workspace_7" => "Select workspace 7",
        "select_workspace_8" => "Select workspace 8",
        "select_workspace_9" => "Select workspace 9",
        "new_tab" => "New tab",
        "close_tab" => "Close tab",
        "terminal_copy" => "Copy",
        "terminal_paste" => "Paste",
        "scroll_page_up" => "Scroll up",
        "scroll_page_down" => "Scroll down",
        "close_window" => "Close window",
        "toggle_zoom" => "Toggle zoom",
        "layout_even_horizontal" => "Layout even horizontal",
        "layout_even_vertical" => "Layout even vertical",
        "layout_main_vertical" => "Layout main vertical",
        "layout_tiled" => "Layout tiled",
        _ => "Unknown",
    }
}

/// Format a GPUI keystroke string for display (e.g. "ctrl-shift-d" → "Ctrl+Shift+D").
fn format_keystroke(key: &str) -> String {
    key.split('-')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_string(),
            "shift" => "Shift".to_string(),
            "alt" => "Alt".to_string(),
            "super" => "Super".to_string(),
            "tab" => "Tab".to_string(),
            "pageup" => "PageUp".to_string(),
            "pagedown" => "PageDown".to_string(),
            "left" => "Left".to_string(),
            "right" => "Right".to_string(),
            "up" => "Up".to_string(),
            "down" => "Down".to_string(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
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

    // Defaults first, with user overrides applied
    for d in DEFAULTS {
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
        let is_default_action = DEFAULTS.iter().any(|d| d.action_name == action_name);
        if !is_default_action && action_from_name(action_name).is_some() {
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

/// Look up the action name for a given shortcut index in the effective list.
/// Returns `(default_key, action_name)` for the binding at that index.
pub fn action_name_at(index: usize) -> Option<&'static str> {
    DEFAULTS.get(index).map(|d| d.action_name)
}
