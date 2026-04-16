//! Centralized keybinding management: defaults, user overrides, hot-reload.
//!
//! Uses Zed's clear → defaults → user-overrides pattern via GPUI's
//! `clear_key_bindings()` + `bind_keys()`.

use std::collections::HashMap;

use gpui::{Action, App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Keystroke};

use crate::{
    ClearScrollHistory, ClosePane, CloseTab, CloseWindow, CloseWorkspace, CopyWorkspacePath,
    DismissSearch, FocusDown, FocusLeft, FocusRight, FocusUp, JumpToPromptNext, JumpToPromptPrev,
    LayoutEvenHorizontal, LayoutEvenVertical, LayoutMainVertical, LayoutTiled, NewTab,
    NewWorkspace, NextWorkspace, OpenWorkspaceInCursor, OpenWorkspaceInVsCode,
    OpenWorkspaceInWindsurf, OpenWorkspaceInZed, ResetTerminal, RevealWorkspaceInFileManager,
    ScrollPageDown, ScrollPageUp, SearchNext, SearchPrev, SelectWorkspace1, SelectWorkspace2,
    SelectWorkspace3, SelectWorkspace4, SelectWorkspace5, SelectWorkspace6, SelectWorkspace7,
    SelectWorkspace8, SelectWorkspace9, SplitEqualize, SplitHorizontally, SplitVertically,
    SwapPane, TerminalCopy, TerminalPaste, ToggleCopyMode, ToggleSearch, ToggleSearchRegex,
    ToggleZoom, UndoClosePane,
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
        key: "ctrl-shift-alt-c",
        action_name: "copy_workspace_path",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-r",
        action_name: "reveal_workspace_in_file_manager",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-z",
        action_name: "open_workspace_in_zed",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-c",
        action_name: "open_workspace_in_cursor",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-v",
        action_name: "open_workspace_in_vscode",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-w",
        action_name: "open_workspace_in_windsurf",
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
        action_name: "undo_close_pane",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-alt-t",
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
    DefaultBinding {
        key: "ctrl-shift-=",
        action_name: "split_equalize",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-s",
        action_name: "swap_pane",
        context: None,
    },
    DefaultBinding {
        key: "ctrl-shift-x",
        action_name: "toggle_copy_mode",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "ctrl-shift-f",
        action_name: "toggle_search",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "enter",
        action_name: "search_next",
        context: Some("Search"),
    },
    DefaultBinding {
        key: "shift-enter",
        action_name: "search_prev",
        context: Some("Search"),
    },
    DefaultBinding {
        key: "escape",
        action_name: "dismiss_search",
        context: Some("Search"),
    },
    DefaultBinding {
        key: "alt-r",
        action_name: "toggle_search_regex",
        context: Some("Search"),
    },
    DefaultBinding {
        key: "ctrl-shift-up",
        action_name: "jump_to_prompt_prev",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "ctrl-shift-down",
        action_name: "jump_to_prompt_next",
        context: Some("Terminal"),
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
        "copy_workspace_path" => Box::new(CopyWorkspacePath),
        "reveal_workspace_in_file_manager" => Box::new(RevealWorkspaceInFileManager),
        "open_workspace_in_zed" => Box::new(OpenWorkspaceInZed),
        "open_workspace_in_cursor" => Box::new(OpenWorkspaceInCursor),
        "open_workspace_in_vscode" => Box::new(OpenWorkspaceInVsCode),
        "open_workspace_in_windsurf" => Box::new(OpenWorkspaceInWindsurf),
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
        "split_equalize" => Box::new(SplitEqualize),
        "swap_pane" => Box::new(SwapPane),
        "undo_close_pane" => Box::new(UndoClosePane),
        "toggle_copy_mode" => Box::new(ToggleCopyMode),
        "toggle_search" => Box::new(ToggleSearch),
        "toggle_search_regex" => Box::new(ToggleSearchRegex),
        "search_next" => Box::new(SearchNext),
        "search_prev" => Box::new(SearchPrev),
        "dismiss_search" => Box::new(DismissSearch),
        "jump_to_prompt_prev" => Box::new(JumpToPromptPrev),
        "jump_to_prompt_next" => Box::new(JumpToPromptNext),
        "clear_scroll_history" => Box::new(ClearScrollHistory),
        "reset_terminal" => Box::new(ResetTerminal),
        _ => return None,
    })
}

/// Context for a given action name. Returns `Some("Terminal")` for terminal-scoped actions.
fn context_for_action(name: &str) -> Option<&'static str> {
    match name {
        "terminal_copy"
        | "terminal_paste"
        | "scroll_page_up"
        | "scroll_page_down"
        | "toggle_copy_mode"
        | "toggle_search"
        | "jump_to_prompt_prev"
        | "jump_to_prompt_next"
        | "clear_scroll_history"
        | "reset_terminal" => Some("Terminal"),
        "search_next" | "search_prev" | "dismiss_search" | "toggle_search_regex" => Some("Search"),
        _ => None,
    }
}

/// Normalize a user-friendly keystroke string to GPUI format.
///
/// Users may write `"ctrl+shift+d"` (plus separators) in `paneflow.json`,
/// but GPUI expects `"ctrl-shift-d"` (dash separators).
fn normalize_keystroke(keystrokes: &str) -> String {
    keystrokes.replace('+', "-")
}

/// Build a `KeyBinding` from a boxed action, using `KeyBinding::load` to avoid
/// the `A: Action` bound on `KeyBinding::new`. Returns `None` on invalid keystroke.
fn make_binding(
    keystrokes: &str,
    action: Box<dyn Action>,
    context: Option<&str>,
) -> Option<KeyBinding> {
    let normalized = normalize_keystroke(keystrokes);
    let predicate = context.map(|ctx| {
        KeyBindingContextPredicate::parse(ctx)
            .expect("invalid context predicate")
            .into()
    });
    match KeyBinding::load(
        &normalized,
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

    // Collect keys that user has explicitly unbound via "none"
    let unbound_keys: std::collections::HashSet<&str> = user_shortcuts
        .iter()
        .filter(|(_, v)| v.as_str() == "none")
        .map(|(k, _)| k.as_str())
        .collect();

    // Collect actions that user has remapped to a different key
    let remapped_actions: std::collections::HashSet<&str> = user_shortcuts
        .iter()
        .filter(|(_, v)| v.as_str() != "none")
        .filter_map(|(_, action_name)| {
            if action_from_name(action_name).is_some() {
                Some(action_name.as_str())
            } else {
                None
            }
        })
        .collect();

    // Register defaults, skipping unbound keys and remapped actions
    let default_bindings: Vec<KeyBinding> = DEFAULTS
        .iter()
        .filter(|d| !unbound_keys.contains(d.key))
        .filter(|d| !remapped_actions.contains(d.action_name))
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
        "copy_workspace_path" => "Copy path",
        "reveal_workspace_in_file_manager" => "Reveal in file manager",
        "open_workspace_in_zed" => "Open in Zed",
        "open_workspace_in_cursor" => "Open in Cursor",
        "open_workspace_in_vscode" => "Open in VS Code",
        "open_workspace_in_windsurf" => "Open in Windsurf",
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
        "toggle_copy_mode" => "Toggle copy mode",
        "close_window" => "Close window",
        "toggle_zoom" => "Toggle zoom",
        "layout_even_horizontal" => "Layout even horizontal",
        "layout_even_vertical" => "Layout even vertical",
        "layout_main_vertical" => "Layout main vertical",
        "layout_tiled" => "Layout tiled",
        "split_equalize" => "Equalize panes",
        "swap_pane" => "Swap pane",
        "undo_close_pane" => "Undo close pane",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keystroke_converts_plus_to_dash() {
        assert_eq!(normalize_keystroke("ctrl+shift+d"), "ctrl-shift-d");
        assert_eq!(normalize_keystroke("alt+left"), "alt-left");
    }

    #[test]
    fn normalize_keystroke_already_dashed_unchanged() {
        assert_eq!(normalize_keystroke("ctrl-shift-d"), "ctrl-shift-d");
    }

    #[test]
    fn action_from_name_known_actions() {
        assert!(action_from_name("split_horizontally").is_some());
        assert!(action_from_name("close_pane").is_some());
        assert!(action_from_name("toggle_zoom").is_some());
        assert!(action_from_name("undo_close_pane").is_some());
        assert!(action_from_name("swap_pane").is_some());
        assert!(action_from_name("split_equalize").is_some());
        assert!(action_from_name("toggle_copy_mode").is_some());
    }

    #[test]
    fn action_from_name_unknown_returns_none() {
        assert!(action_from_name("nonexistent_action").is_none());
        assert!(action_from_name("").is_none());
    }

    #[test]
    fn context_for_terminal_actions() {
        assert_eq!(context_for_action("terminal_copy"), Some("Terminal"));
        assert_eq!(context_for_action("toggle_copy_mode"), Some("Terminal"));
        assert_eq!(context_for_action("toggle_search"), Some("Terminal"));
        assert_eq!(context_for_action("split_horizontally"), None);
    }

    // --- effective_shortcuts tests (US-016) ---

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
        overrides.insert("ctrl-shift-d".to_string(), "none".to_string());
        let entries = effective_shortcuts(&overrides);
        // The default ctrl-shift-d binding should be excluded
        let has_ctrl_shift_d = entries.iter().any(|e| e.key == "Ctrl+Shift+D");
        assert!(
            !has_ctrl_shift_d,
            "Unbound key should not appear in effective list"
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

    #[test]
    fn effective_shortcuts_preserves_unoverridden_defaults() {
        let mut overrides = HashMap::new();
        overrides.insert("ctrl+alt+h".to_string(), "split_horizontally".to_string());
        let entries = effective_shortcuts(&overrides);
        // close_pane should still be at its default key
        let close = entries
            .iter()
            .find(|e| e.description == "Close pane")
            .expect("Close pane should be in effective list");
        assert_eq!(
            close.key, "Ctrl+Shift+W",
            "Unoverridden action should keep default key"
        );
    }

    #[test]
    fn default_bindings_cover_all_core_actions() {
        // Verify all core actions are in DEFAULTS
        let action_names: Vec<&str> = DEFAULTS.iter().map(|d| d.action_name).collect();
        for name in &[
            "split_horizontally",
            "split_vertically",
            "close_pane",
            "next_workspace",
            "focus_left",
            "focus_right",
            "focus_up",
            "focus_down",
            "terminal_copy",
            "terminal_paste",
            "toggle_zoom",
            "toggle_copy_mode",
            "toggle_search",
            "split_equalize",
            "swap_pane",
            "undo_close_pane",
        ] {
            assert!(
                action_names.contains(name),
                "Action '{name}' missing from DEFAULTS"
            );
        }
    }

    #[test]
    fn format_keystroke_produces_readable_output() {
        assert_eq!(format_keystroke("ctrl-shift-d"), "Ctrl+Shift+D");
        assert_eq!(format_keystroke("alt-left"), "Alt+Left");
        assert_eq!(format_keystroke("ctrl-1"), "Ctrl+1");
        assert_eq!(format_keystroke("shift-pageup"), "Shift+PageUp");
    }
}
