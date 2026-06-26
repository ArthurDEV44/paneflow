//! Settings lifecycle + persistence + key handlers for `PaneFlowApp`.
//!
//! The settings *UI* - the Codex-style nav rail, the content panel, and the
//! per-section bodies - lives in `crate::settings` (`chrome` + `tabs::*`). This
//! module owns the glue on `PaneFlowApp`:
//! - [`PaneFlowApp::open_settings_window`] / [`PaneFlowApp::close_settings`] -
//!   toggle the embedded settings (set/clear `settings_section`).
//! - [`PaneFlowApp::persist_setting`] - the shared cache-mutate + repaint +
//!   off-thread write used by every settings control.
//! - [`PaneFlowApp::handle_settings_key_down`] /
//!   [`PaneFlowApp::handle_shortcut_recording`] - key routing for the font-picker
//!   typeahead, Escape handling, and shortcut capture.

use gpui::{Context, KeyDownEvent, ScrollHandle, Window, prelude::*};

use crate::{PaneFlowApp, SettingsSection, config_writer, keybindings};

impl PaneFlowApp {
    /// Open the embedded settings (Codex-style). The Settings button and the
    /// title-bar / macOS menu route here; it sets `settings_section`, and
    /// `main.rs` then swaps the left rail for the settings nav and the content
    /// area for the section panel. The name is kept for call-site compatibility
    /// there is no separate settings *window* anymore.
    pub(crate) fn open_settings_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_menu_open = None;
        self.profile_menu_open = None;
        self.agents_view.sidebar_actions_menu_open = false;
        self.agents_view.sidebar_mode_picker_open = false;
        self.settings_section = Some(SettingsSection::General);
        self.reset_settings_scroll();
        self.terminal_dropdown = None;
        self.general_dropdown = None;
        self.font_dropdown_open = false;
        self.font_search.clear();
        // Clear any stale nav search so the forced `General` landing row is
        // always visible (a leftover query could filter the nav to a section
        // that doesn't match the displayed page).
        self.clear_settings_search(cx);
        // Warm the MCP bridge status off-thread so the MCP page can render its
        // button label without ever doing config I/O during a frame.
        self.refresh_mcp_status(cx);
        self.settings_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.profile_menu_open = None;
        self.font_dropdown_open = false;
        self.font_search.clear();
        self.terminal_dropdown = None;
        self.general_dropdown = None;
        self.clear_settings_search(cx);
        if self.recording_shortcut_idx.is_some() {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
        }
    }

    /// Drop stale scroll geometry when the settings surface is remounted,
    /// changes page, or the window is resized. GPUI repopulates the handle from
    /// the next `track_scroll` layout pass.
    pub(crate) fn reset_settings_scroll(&mut self) {
        self.settings_scroll = ScrollHandle::new();
        self.settings_drag = None;
    }

    /// Reset the nav search box. Shared by open/close so a reopened settings
    /// page always shows the full, unfiltered section list.
    fn clear_settings_search(&mut self, cx: &mut Context<Self>) {
        self.settings_search_input.update(cx, |inp, cx| {
            inp.content = gpui::SharedString::default();
            inp.selected_range = 0..0;
            cx.notify();
        });
    }

    /// Apply a settings-control change. Mutates the render cache in memory for
    /// instant feedback, repaints, then persists the field to disk off the GPUI
    /// main thread (`smol::unblock`). `nested` routes into the `terminal` block;
    /// a `Null` value clears the field.
    pub(crate) fn persist_setting(
        &mut self,
        nested: bool,
        key: &'static str,
        value: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.cached_config =
            config_writer::with_field(&self.cached_config, nested, key, value.clone());
        cx.notify();
        cx.background_spawn(async move {
            smol::unblock(move || {
                let ok = if nested {
                    config_writer::save_terminal_field(key, value);
                    true
                } else {
                    config_writer::save_config_value_checked(key, value)
                };
                if !ok {
                    log::warn!(
                        "settings: failed to persist {key}; choice is in-memory only this session"
                    );
                }
            })
            .await;
        })
        .detach();
    }

    pub(crate) fn handle_settings_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Font dropdown typeahead (Terminal page).
        if self.font_dropdown_open {
            let key = event.keystroke.key.as_str();
            match key {
                "escape" => {
                    self.font_dropdown_open = false;
                    self.font_search.clear();
                    cx.notify();
                }
                "backspace" => {
                    self.font_search.pop();
                    cx.notify();
                }
                _ => {
                    if let Some(ch) = &event.keystroke.key_char
                        && !ch.is_empty()
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.platform
                    {
                        self.font_search.push_str(ch);
                        cx.notify();
                    }
                }
            }
            return;
        }

        // Escape (outside active shortcut recording): close an open Terminal-page
        // dropdown first, otherwise leave settings. During recording, Escape
        // falls through to `handle_shortcut_recording`, which cancels capture.
        if event.keystroke.key == "escape" && self.recording_shortcut_idx.is_none() {
            if self.terminal_dropdown.is_some() {
                self.terminal_dropdown = None;
            } else if self.general_dropdown.is_some() {
                self.general_dropdown = None;
            } else {
                self.close_settings(cx);
            }
            cx.notify();
            return;
        }

        // Shortcut recording (only on the Shortcuts page).
        if self.settings_section == Some(SettingsSection::Shortcuts) {
            self.handle_shortcut_recording(event, window, cx);
        }
    }

    pub(crate) fn handle_shortcut_recording(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.recording_shortcut_idx else {
            return;
        };

        // Ignore bare modifier presses (Shift alone, Ctrl alone, etc.)
        if keybindings::is_bare_modifier(&event.keystroke) {
            return;
        }

        // Escape cancels recording.
        if event.keystroke.key == "escape" {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        }

        // Resolve the action by the row's stable identity, NOT by indexing
        // `DEFAULTS` (the displayed list chains macOS-only defaults, skips
        // unbound rows, and appends user-only actions, so a positional index
        // would rebind the wrong action and corrupt `paneflow.json`).
        let Some(action_name) = self.effective_shortcuts.get(idx).map(|e| e.action_name) else {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        };

        // Format keystroke to a GPUI string (e.g. "ctrl-shift-d") and save it.
        let new_key = event.keystroke.to_string();
        config_writer::save_shortcut(&new_key, action_name);

        // Re-apply keybindings from the updated config.
        let config = paneflow_config::loader::load_config();
        keybindings::apply_keybindings(cx, &config.shortcuts);
        self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
        self.recording_shortcut_idx = None;
        cx.notify();
    }
}
