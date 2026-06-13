//! "General" settings page — the default landing section.
//!
//! Hosts the default-shell preference (`default_shell`): a curated set of
//! platform shells as selectable chips. Empty = fall back to `$SHELL` / the
//! platform default. Persists through [`PaneFlowApp::persist_setting`]
//! (cache-mutate + repaint + off-thread write), the same path every other
//! settings page uses.
//!
//! (The old "Window mode" / `window_decorations` radio cards were removed: the
//! setting is a no-op on macOS/Windows — PaneFlow always draws its own chrome —
//! and only meaningful on Linux, where it stays editable via `paneflow.json`.)

use gpui::{
    ClickEvent, Context, CursorStyle, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};

use crate::PaneFlowApp;
use crate::settings::components::{section_header, setting_card, setting_text};

impl PaneFlowApp {
    pub(crate) fn render_general_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let config = &self.cached_config;

        // ── Default shell — chips ───────────────────────────────────────
        // Order mirrors `terminal::shell`'s resolver preference (pwsh 7 before
        // Windows PowerShell 5.1 before cmd); `bash.exe` covers Git-Bash/MSYS,
        // which `terminal::shell` already supports on Windows. The list is a
        // curated set of presets — any other value still works via config.
        #[cfg(target_os = "windows")]
        let shells: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe", "bash.exe"];
        #[cfg(not(target_os = "windows"))]
        let shells: &[&str] = &["/bin/zsh", "/bin/bash", "/bin/sh", "/usr/bin/fish"];

        let current_shell = config.default_shell.clone().unwrap_or_default();

        let mut chips = div().flex().flex_row().flex_wrap().gap(px(8.));
        for shell in shells {
            // Match by basename so the chip still highlights when `default_shell`
            // holds a full path equivalent to a bare-name preset (e.g.
            // C:\…\pwsh.exe, or /usr/bin/zsh vs the /bin/zsh chip).
            let is_active = shell_basename_eq(&current_shell, shell);
            let shell_owned = (*shell).to_string();
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("shell-{shell}")))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .cursor(CursorStyle::PointingHand)
                    .when(is_active, |d| {
                        d.bg(ui.accent)
                            .text_color(gpui::white())
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!is_active, |d| {
                        d.bg(ui.subtle)
                            .text_color(ui.text)
                            .hover(|s| s.bg(ui.overlay))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.persist_setting(
                            false,
                            "default_shell",
                            serde_json::Value::String(shell_owned.clone()),
                            cx,
                        );
                    }))
                    .child((*shell).to_string()),
            );
        }

        let shell_card = setting_card(ui).child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .px(px(12.))
                .py(px(10.))
                .child(setting_text(
                    ui,
                    "Default shell",
                    "Program launched in every new terminal. Falls back to \
                     $SHELL (or the platform default) when unset.",
                ))
                .child(chips),
        );

        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(section_header(ui, "Shell"))
            .child(shell_card)
    }
}

/// Case-insensitive basename comparison for the shell chips. Normalizes the
/// same way `terminal::shell` resolves a shell (strip the directory, drop a
/// trailing `.exe`, lowercase) so a stored full path matches its bare-name
/// preset. An empty `stored` (unset `default_shell`) matches nothing.
fn shell_basename_eq(stored: &str, chip: &str) -> bool {
    fn stem(s: &str) -> String {
        let base = s
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(s)
            .to_ascii_lowercase();
        base.trim_end_matches(".exe").to_string()
    }
    !stored.is_empty() && stem(stored) == stem(chip)
}
