//! Keyboard handlers for the settings window — key-down routing + shortcut
//! capture.
//!
//! The dropdown typeahead path consumes keys when the font picker is open.
//! Otherwise, if we're on the Shortcuts tab and a row is recording, keys
//! flow to `handle_shortcut_recording` to be saved as a new binding.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{Context, KeyDownEvent, Window};

use crate::{config_writer, keybindings};

use super::window::{SettingsSection, SettingsWindow};

impl SettingsWindow {
    pub(crate) fn handle_settings_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

        if self.section == SettingsSection::Shortcuts {
            self.handle_shortcut_recording(event, cx);
        }
    }

    fn handle_shortcut_recording(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(idx) = self.recording_shortcut_idx else {
            return;
        };

        if keybindings::is_bare_modifier(&event.keystroke) {
            return;
        }

        if event.keystroke.key == "escape" {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        }

        let Some(action_name) = keybindings::action_name_at(idx) else {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        };

        let new_key = event.keystroke.to_string();
        config_writer::save_shortcut(&new_key, action_name);

        let config = paneflow_config::loader::load_config();
        keybindings::apply_keybindings(cx, &config.shortcuts);
        self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
        self.recording_shortcut_idx = None;
        cx.notify();
    }
}
