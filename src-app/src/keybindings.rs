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
    OpenWorkspaceInWindsurf, OpenWorkspaceInZed, Quit, ResetTerminal, RevealWorkspaceInFileManager,
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
    // US-009: app-global split/workspace bindings use the `secondary`
    // modifier so GPUI resolves to `cmd` on macOS and `ctrl` on Linux/Windows.
    // `secondary` keeps Linux on Ctrl+Shift+… (no user-visible regression)
    // while giving macOS users the expected Cmd+Shift+… shortcuts.
    DefaultBinding {
        key: "secondary-shift-d",
        action_name: "split_horizontally",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-e",
        action_name: "split_vertically",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-w",
        action_name: "close_pane",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-n",
        action_name: "new_workspace",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-q",
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
        key: "secondary-tab",
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
        key: "secondary-1",
        action_name: "select_workspace_1",
        context: None,
    },
    DefaultBinding {
        key: "secondary-2",
        action_name: "select_workspace_2",
        context: None,
    },
    DefaultBinding {
        key: "secondary-3",
        action_name: "select_workspace_3",
        context: None,
    },
    DefaultBinding {
        key: "secondary-4",
        action_name: "select_workspace_4",
        context: None,
    },
    DefaultBinding {
        key: "secondary-5",
        action_name: "select_workspace_5",
        context: None,
    },
    DefaultBinding {
        key: "secondary-6",
        action_name: "select_workspace_6",
        context: None,
    },
    DefaultBinding {
        key: "secondary-7",
        action_name: "select_workspace_7",
        context: None,
    },
    DefaultBinding {
        key: "secondary-8",
        action_name: "select_workspace_8",
        context: None,
    },
    DefaultBinding {
        key: "secondary-9",
        action_name: "select_workspace_9",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-t",
        action_name: "undo_close_pane",
        context: None,
    },
    DefaultBinding {
        key: "secondary-alt-t",
        action_name: "new_tab",
        context: None,
    },
    DefaultBinding {
        key: "secondary-w",
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
        key: "secondary-shift-z",
        action_name: "toggle_zoom",
        context: None,
    },
    DefaultBinding {
        key: "secondary-alt-1",
        action_name: "layout_even_horizontal",
        context: None,
    },
    DefaultBinding {
        key: "secondary-alt-2",
        action_name: "layout_even_vertical",
        context: None,
    },
    DefaultBinding {
        key: "secondary-alt-3",
        action_name: "layout_main_vertical",
        context: None,
    },
    DefaultBinding {
        key: "secondary-alt-4",
        action_name: "layout_tiled",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-=",
        action_name: "split_equalize",
        context: None,
    },
    DefaultBinding {
        key: "secondary-shift-s",
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

/// Platform-specific default bindings layered on top of [`DEFAULTS`].
///
/// US-010 binds `cmd-c` / `cmd-v` to terminal copy/paste on macOS so muscle
/// memory from iTerm2 / Terminal.app / WezTerm works. Kept empty on Linux
/// (AC5) because Linux keyboards don't have a `cmd` key by default, and
/// duplicates would clutter the settings page.
///
/// The existing `ctrl-shift-c/v` Terminal bindings stay intact on both
/// platforms — these are purely additive.
#[cfg(target_os = "macos")]
const MACOS_ONLY_DEFAULTS: &[DefaultBinding] = &[
    DefaultBinding {
        key: "cmd-c",
        action_name: "terminal_copy",
        context: Some("Terminal"),
    },
    DefaultBinding {
        key: "cmd-v",
        action_name: "terminal_paste",
        context: Some("Terminal"),
    },
    // US-012: Cmd+Q quits the app and populates the "⌘Q" shortcut next to
    // the Quit PaneFlow menu item. Global context so the menu picks it up
    // whether or not a terminal pane holds focus.
    DefaultBinding {
        key: "cmd-q",
        action_name: "quit",
        context: None,
    },
];

#[cfg(not(target_os = "macos"))]
const MACOS_ONLY_DEFAULTS: &[DefaultBinding] = &[];

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
        // US-012: Quit menu action (bound to cmd-q on macOS via
        // MACOS_ONLY_DEFAULTS; also reachable from PaneFlow > Quit PaneFlow).
        "quit" => Box::new(Quit),
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

    // Register defaults, skipping unbound keys and remapped actions.
    // US-010: chain macOS-only defaults (cmd-c/cmd-v in Terminal context).
    let default_bindings: Vec<KeyBinding> = DEFAULTS
        .iter()
        .chain(MACOS_ONLY_DEFAULTS.iter())
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
        "quit" => "Quit",
        _ => "Unknown",
    }
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
fn format_keystroke(key: &str) -> String {
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn format_keystroke_produces_readable_output() {
        assert_eq!(format_keystroke("ctrl-shift-d"), "Ctrl+Shift+D");
        assert_eq!(format_keystroke("alt-left"), "Alt+Left");
        assert_eq!(format_keystroke("ctrl-1"), "Ctrl+1");
        assert_eq!(format_keystroke("shift-pageup"), "Shift+PageUp");
    }

    // -- US-009 ---------------------------------------------------------

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

    #[test]
    fn secondary_binding_parses_successfully() {
        // AC2/AC3: make_binding accepts the `secondary` prefix on both
        // platforms. GPUI's Keystroke::parse resolves it internally.
        let binding = make_binding(
            "secondary-shift-d",
            Box::new(crate::SplitHorizontally),
            None,
        );
        assert!(
            binding.is_some(),
            "secondary-shift-d must parse into a valid KeyBinding"
        );
    }

    #[test]
    fn cmd_override_parses_on_any_platform() {
        // AC5: a user writing `"split_horizontally": "cmd-shift-d"` in
        // paneflow.json must produce a valid binding on Linux as well as
        // macOS (GPUI accepts `cmd` as a synonym for the platform modifier).
        let binding = make_binding("cmd-shift-d", Box::new(crate::SplitHorizontally), None);
        assert!(
            binding.is_some(),
            "cmd-shift-d override must parse on any platform"
        );
    }

    #[test]
    fn us009_migrated_defaults_use_secondary() {
        // AC1: the seven migrated actions must carry a `secondary-` prefix.
        let migrated = [
            "split_horizontally",
            "split_vertically",
            "close_pane",
            "new_workspace",
            "close_workspace",
            "next_workspace",
            "select_workspace_1",
            "select_workspace_2",
            "select_workspace_3",
            "select_workspace_4",
            "select_workspace_5",
            "select_workspace_6",
            "select_workspace_7",
            "select_workspace_8",
            "select_workspace_9",
        ];
        for action in migrated {
            let entry = DEFAULTS
                .iter()
                .find(|d| d.action_name == action)
                .unwrap_or_else(|| panic!("missing DEFAULTS entry for {action}"));
            assert!(
                entry.key.starts_with("secondary-"),
                "action `{action}` still uses `{}` — US-009 requires `secondary-` prefix",
                entry.key,
            );
        }
    }

    #[test]
    fn us009_terminal_copy_paste_untouched() {
        // AC4: terminal copy/paste must keep `ctrl-shift-c/v` so Linux users
        // retain the terminal-standard bindings and Ctrl+C stays SIGINT-safe.
        let copy = DEFAULTS
            .iter()
            .find(|d| d.action_name == "terminal_copy")
            .expect("terminal_copy must be a default");
        assert_eq!(copy.key, "ctrl-shift-c");
        assert_eq!(copy.context, Some("Terminal"));

        let paste = DEFAULTS
            .iter()
            .find(|d| d.action_name == "terminal_paste")
            .expect("terminal_paste must be a default");
        assert_eq!(paste.key, "ctrl-shift-v");
        assert_eq!(paste.context, Some("Terminal"));
    }

    // -- US-010 ---------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn us010_cmd_c_cmd_v_bound_on_macos() {
        // AC1/AC2: macOS registers cmd-c and cmd-v as Terminal-context
        // aliases for terminal_copy / terminal_paste, IN ADDITION to the
        // existing ctrl-shift-c/v defaults.
        let copy = MACOS_ONLY_DEFAULTS
            .iter()
            .find(|d| d.key == "cmd-c")
            .expect("cmd-c must be a macOS default");
        assert_eq!(copy.action_name, "terminal_copy");
        assert_eq!(copy.context, Some("Terminal"));

        let paste = MACOS_ONLY_DEFAULTS
            .iter()
            .find(|d| d.key == "cmd-v")
            .expect("cmd-v must be a macOS default");
        assert_eq!(paste.action_name, "terminal_paste");
        assert_eq!(paste.context, Some("Terminal"));

        // Base DEFAULTS still hold the ctrl-shift-c/v entries — the macOS
        // bindings are ADDITIVE, not replacements.
        assert!(
            DEFAULTS
                .iter()
                .any(|d| d.key == "ctrl-shift-c" && d.action_name == "terminal_copy")
        );
        assert!(
            DEFAULTS
                .iter()
                .any(|d| d.key == "ctrl-shift-v" && d.action_name == "terminal_paste")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn us010_no_cmd_bindings_on_linux() {
        // AC5: Linux keyboards lack a cmd key by default; MACOS_ONLY_DEFAULTS
        // must be empty so the settings page and binding registry stay
        // clean.
        assert!(
            MACOS_ONLY_DEFAULTS.is_empty(),
            "Linux build should carry zero macOS-only defaults, got {} entries",
            MACOS_ONLY_DEFAULTS.len()
        );
    }

    #[test]
    fn us010_ctrl_c_never_bound_to_terminal_copy() {
        // AC4: plain `ctrl-c` (without shift) must never reach terminal_copy
        // on any platform — the PTY needs to receive it so running
        // processes still get SIGINT.
        let leaked_actions: Vec<&'static str> = DEFAULTS
            .iter()
            .chain(MACOS_ONLY_DEFAULTS.iter())
            .filter(|d| d.key == "ctrl-c")
            .map(|d| d.action_name)
            .collect();
        assert!(
            leaked_actions.is_empty(),
            "ctrl-c must not appear in defaults (SIGINT safety); bound to: {leaked_actions:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn us010_cmd_c_parses_as_binding() {
        // Defence-in-depth: the raw string we register must actually parse
        // into a valid KeyBinding on macOS. GPUI's Keystroke::parse treats
        // `cmd` as the platform modifier.
        let binding = make_binding("cmd-c", Box::new(TerminalCopy), Some("Terminal"));
        assert!(binding.is_some(), "cmd-c must parse as a valid KeyBinding");
    }

    // -- US-012 ---------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn us012_cmd_q_bound_to_quit() {
        // AC2/AC3: cmd-q must be a macOS default wired to the `quit` action,
        // so clicking PaneFlow > Quit PaneFlow or pressing ⌘Q quits the app.
        let quit = MACOS_ONLY_DEFAULTS
            .iter()
            .find(|d| d.key == "cmd-q")
            .expect("cmd-q must be a macOS default");
        assert_eq!(quit.action_name, "quit");
        assert_eq!(quit.context, None);
    }

    #[test]
    fn us012_quit_action_name_resolves() {
        // Cross-platform: `action_from_name` must resolve "quit" to a real
        // Action instance so MACOS_ONLY_DEFAULTS registration succeeds on
        // macOS and user config overrides like `"quit": "secondary-alt-q"`
        // work on any platform.
        assert!(action_from_name("quit").is_some());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn us012_no_cmd_q_on_linux() {
        // AC5: the menu bar is macOS-only, so `cmd-q` must not bleed into
        // Linux. MACOS_ONLY_DEFAULTS is an empty slice on Linux (US-010
        // already asserts this; re-checking specifically for cmd-q keeps
        // regressions localised to the right story).
        assert!(MACOS_ONLY_DEFAULTS.iter().all(|d| d.key != "cmd-q"));
    }
}
