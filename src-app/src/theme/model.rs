//! Terminal theme data model and UI palette.

use gpui::{Hsla, Rgba};

/// Terminal color theme with 35 slots:
/// 5 base + cursor + selection + scrollbar_thumb + link_text + 2 title bar + 24 ANSI (8 hues x 3 intensities).
#[derive(Clone, Copy)]
pub struct TerminalTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub bright_foreground: Hsla,
    pub dim_foreground: Hsla,
    pub ansi_background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub scrollbar_thumb: Hsla,
    /// Color for hyperlink underline and text on Ctrl+hover.
    pub link_text: Hsla,
    pub title_bar_background: Hsla,
    pub title_bar_inactive_background: Hsla,
    // 8 hues x 3 intensities = 24 ANSI colors
    pub black: Hsla,
    pub red: Hsla,
    pub green: Hsla,
    pub yellow: Hsla,
    pub blue: Hsla,
    pub magenta: Hsla,
    pub cyan: Hsla,
    pub white: Hsla,
    pub bright_black: Hsla,
    pub bright_red: Hsla,
    pub bright_green: Hsla,
    pub bright_yellow: Hsla,
    pub bright_blue: Hsla,
    pub bright_magenta: Hsla,
    pub bright_cyan: Hsla,
    pub bright_white: Hsla,
    pub dim_black: Hsla,
    pub dim_red: Hsla,
    pub dim_green: Hsla,
    pub dim_yellow: Hsla,
    pub dim_blue: Hsla,
    pub dim_magenta: Hsla,
    pub dim_cyan: Hsla,
    pub dim_white: Hsla,
}

pub(super) fn h(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Hsla::from(Rgba { r, g, b, a: 1.0 })
}

pub(super) fn ha(hex: u32, alpha: f32) -> Hsla {
    let mut color = h(hex);
    color.a = alpha;
    color
}

const CHROME_BACKGROUND_HEX: u32 = 0x141414;
const TERMINAL_BACKGROUND_HEX: u32 = 0x181818;

fn is_light_theme(theme: &TerminalTheme) -> bool {
    theme.background.l > 0.5
}

pub(super) fn apply_surface_overrides(mut theme: TerminalTheme) -> TerminalTheme {
    if is_light_theme(&theme) {
        return theme;
    }

    let chrome_bg = h(CHROME_BACKGROUND_HEX);
    let terminal_bg = h(TERMINAL_BACKGROUND_HEX);
    theme.title_bar_background = chrome_bg;
    theme.title_bar_inactive_background = chrome_bg;
    theme.background = terminal_bg;
    theme.ansi_background = terminal_bg;
    theme
}

// ---------------------------------------------------------------------------
// UI color palette — derived from the active terminal theme
// ---------------------------------------------------------------------------

/// Colors for the app chrome (sidebar, settings, badges, etc.).
#[derive(Clone, Copy)]
pub struct UiColors {
    pub base: Hsla,       // deepest background (settings sidebar, app bg)
    pub surface: Hsla,    // card/panel background
    pub overlay: Hsla,    // dropdown/popover bg
    pub border: Hsla,     // borders, dividers
    pub subtle: Hsla,     // hover bg, badge bg
    pub muted: Hsla,      // secondary text, labels
    pub text: Hsla,       // primary text
    pub accent: Hsla,     // active indicator, highlighted items
    pub preview_bg: Hsla, // preview box background
}

/// Derive UI colors from the active terminal theme.
///
/// Light themes get light UI; dark themes get dark UI.
pub fn ui_colors() -> UiColors {
    let theme = super::watcher::active_theme();
    let is_light = is_light_theme(&theme);
    if is_light {
        UiColors {
            base: h(0xefefef),
            surface: h(0xf8f8f8),
            overlay: h(0xffffff),
            border: h(0xd4d4d8),
            subtle: h(0xe4e4e7),
            muted: h(0x71717a),
            text: h(0x27272a),
            accent: h(0x4078f2),
            preview_bg: h(0xfafafa),
        }
    } else {
        UiColors {
            base: h(0x181818),
            surface: h(0x212121),
            overlay: h(0x141414),
            border: h(0x333333),
            subtle: h(0x2a2a2a),
            muted: h(0x888888),
            text: h(0xffffff),
            accent: h(0x89b4fa),
            preview_bg: h(0x141414),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin::{catppuccin_mocha, paneflow_light};

    #[test]
    fn light_theme_keeps_light_surfaces_after_overrides() {
        let theme = apply_surface_overrides(paneflow_light());

        assert!(theme.background.l > 0.5);
        assert!(theme.ansi_background.l > 0.5);
        assert!(theme.title_bar_background.l > 0.5);
    }

    #[test]
    fn dark_theme_still_uses_dark_surface_overrides() {
        let theme = apply_surface_overrides(catppuccin_mocha());

        assert_eq!(theme.background.l, h(TERMINAL_BACKGROUND_HEX).l);
        assert_eq!(theme.ansi_background.l, h(TERMINAL_BACKGROUND_HEX).l);
        assert_eq!(theme.title_bar_background.l, h(CHROME_BACKGROUND_HEX).l);
    }
}
