//! Terminal theming with 30 color slots compatible with Zed's terminal theme format.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

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

fn h(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Hsla::from(Rgba { r, g, b, a: 1.0 })
}

fn ha(hex: u32, alpha: f32) -> Hsla {
    let mut color = h(hex);
    color.a = alpha;
    color
}

const CHROME_BACKGROUND_HEX: u32 = 0x141414;
const TERMINAL_BACKGROUND_HEX: u32 = 0x181818;

fn is_light_theme(theme: &TerminalTheme) -> bool {
    theme.background.l > 0.5
}

fn apply_surface_overrides(mut theme: TerminalTheme) -> TerminalTheme {
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
// Bundled themes
// ---------------------------------------------------------------------------

pub type ThemeEntry = (&'static str, fn() -> TerminalTheme);

pub static THEMES: &[ThemeEntry] = &[
    ("Catppuccin Mocha", catppuccin_mocha),
    ("PaneFlow Light", paneflow_light),
    ("One Dark", one_dark),
    ("Dracula", dracula),
    ("Gruvbox Dark", gruvbox_dark),
    ("Solarized Dark", solarized_dark),
];

pub fn catppuccin_mocha() -> TerminalTheme {
    TerminalTheme {
        background: h(0x212121),
        foreground: h(0xcdd6f4),
        bright_foreground: h(0xcdd6f4),
        dim_foreground: h(0x8b91a6),
        ansi_background: h(0x212121),
        cursor: h(0xf5e0dc),
        selection: ha(0x89b4fa, 0.3),
        scrollbar_thumb: ha(0xcdd6f4, 0.4),
        link_text: h(0x89b4fa),
        title_bar_background: h(0x181818),
        title_bar_inactive_background: h(0x181818),
        black: h(0x45475a),
        red: h(0xf38ba8),
        green: h(0xa6e3a1),
        yellow: h(0xf9e2af),
        blue: h(0x89b4fa),
        magenta: h(0xf5c2e7),
        cyan: h(0x94e2d5),
        white: h(0xa6adc8), // Subtext0 (matches Ghostty palette 7)
        bright_black: h(0x585b70),
        bright_red: h(0xf37799),     // distinct bright variant
        bright_green: h(0x89d88b),   // distinct bright variant
        bright_yellow: h(0xebd391),  // distinct bright variant
        bright_blue: h(0x74a8fc),    // distinct bright variant
        bright_magenta: h(0xf2aede), // distinct bright variant
        bright_cyan: h(0x6bd7ca),    // distinct bright variant
        bright_white: h(0xbac2de),   // Subtext1 (matches Ghostty palette 15)
        dim_black: h(0x353544),
        dim_red: h(0xb3677e),
        dim_green: h(0x7baa78),
        dim_yellow: h(0xb9a882),
        dim_blue: h(0x6687ba),
        dim_magenta: h(0xb590ab),
        dim_cyan: h(0x6ea99e),
        dim_white: h(0x8b91a6),
    }
}

pub fn paneflow_light() -> TerminalTheme {
    TerminalTheme {
        background: h(0xf5f5f5),
        foreground: h(0x383a42),
        bright_foreground: h(0x383a42),
        dim_foreground: h(0x9ca0b0),
        ansi_background: h(0xf5f5f5),
        cursor: h(0x526fff),
        selection: ha(0x4078f2, 0.25),
        scrollbar_thumb: ha(0x383a42, 0.35),
        link_text: h(0x4078f2),
        title_bar_background: h(0xe8e8e8),
        title_bar_inactive_background: h(0xededed),
        black: h(0x383a42),
        red: h(0xe45649),
        green: h(0x50a14f),
        yellow: h(0xc18401),
        blue: h(0x4078f2),
        magenta: h(0xa626a4),
        cyan: h(0x0184bc),
        white: h(0xa0a1a7),
        bright_black: h(0x696c77),
        bright_red: h(0xd23d2d),
        bright_green: h(0x3e8a3e),
        bright_yellow: h(0xa67200),
        bright_blue: h(0x2e64d4),
        bright_magenta: h(0x8b1b8b),
        bright_cyan: h(0x016e9e),
        bright_white: h(0x383a42),
        dim_black: h(0xb0b1b5),
        dim_red: h(0xc9887f),
        dim_green: h(0x8ab88a),
        dim_yellow: h(0xb5a06a),
        dim_blue: h(0x7da0d6),
        dim_magenta: h(0xb881b8),
        dim_cyan: h(0x6aa9c0),
        dim_white: h(0x9ca0b0),
    }
}

pub fn one_dark() -> TerminalTheme {
    TerminalTheme {
        background: h(0x282c34),
        foreground: h(0xabb2bf),
        bright_foreground: h(0xabb2bf),
        dim_foreground: h(0x5c6370),
        ansi_background: h(0x282c34),
        cursor: h(0x528bff),
        selection: ha(0x528bff, 0.3),
        scrollbar_thumb: ha(0xabb2bf, 0.4),
        link_text: h(0x61afef),
        title_bar_background: h(0x21252b),
        title_bar_inactive_background: h(0x1b1f23),
        black: h(0x3f4451),
        red: h(0xe06c75),
        green: h(0x98c379),
        yellow: h(0xe5c07b),
        blue: h(0x61afef),
        magenta: h(0xc678dd),
        cyan: h(0x56b6c2),
        white: h(0xabb2bf),
        bright_black: h(0x4f5666),
        bright_red: h(0xe06c75),
        bright_green: h(0x98c379),
        bright_yellow: h(0xe5c07b),
        bright_blue: h(0x61afef),
        bright_magenta: h(0xc678dd),
        bright_cyan: h(0x56b6c2),
        bright_white: h(0xdcdfe4),
        dim_black: h(0x2c313a),
        dim_red: h(0xa85059),
        dim_green: h(0x72935b),
        dim_yellow: h(0xab905c),
        dim_blue: h(0x4983b3),
        dim_magenta: h(0x955aa6),
        dim_cyan: h(0x418992),
        dim_white: h(0x808690),
    }
}

pub fn dracula() -> TerminalTheme {
    TerminalTheme {
        background: h(0x282a36),
        foreground: h(0xf8f8f2),
        bright_foreground: h(0xf8f8f2),
        dim_foreground: h(0x6272a4),
        ansi_background: h(0x282a36),
        cursor: h(0xf8f8f2),
        selection: ha(0xbd93f9, 0.3),
        scrollbar_thumb: ha(0xf8f8f2, 0.4),
        link_text: h(0x8be9fd),
        title_bar_background: h(0x21222c),
        title_bar_inactive_background: h(0x191a21),
        black: h(0x21222c),
        red: h(0xff5555),
        green: h(0x50fa7b),
        yellow: h(0xf1fa8c),
        blue: h(0xbd93f9),
        magenta: h(0xff79c6),
        cyan: h(0x8be9fd),
        white: h(0xf8f8f2),
        bright_black: h(0x6272a4),
        bright_red: h(0xff6e6e),
        bright_green: h(0x69ff94),
        bright_yellow: h(0xffffa5),
        bright_blue: h(0xd6acff),
        bright_magenta: h(0xff92df),
        bright_cyan: h(0xa4ffff),
        bright_white: h(0xffffff),
        dim_black: h(0x191a21),
        dim_red: h(0xbf4040),
        dim_green: h(0x3cbb5c),
        dim_yellow: h(0xb5bb6a),
        dim_blue: h(0x8e6ebb),
        dim_magenta: h(0xbf5b95),
        dim_cyan: h(0x68afbe),
        dim_white: h(0xbabab5),
    }
}

pub fn gruvbox_dark() -> TerminalTheme {
    TerminalTheme {
        background: h(0x282828),
        foreground: h(0xebdbb2),
        bright_foreground: h(0xfbf1c7),
        dim_foreground: h(0xa89984),
        ansi_background: h(0x282828),
        cursor: h(0xebdbb2),
        selection: ha(0x458588, 0.35),
        scrollbar_thumb: ha(0xebdbb2, 0.4),
        link_text: h(0x83a598),
        title_bar_background: h(0x1d2021),
        title_bar_inactive_background: h(0x171819),
        black: h(0x282828),
        red: h(0xcc241d),
        green: h(0x98971a),
        yellow: h(0xd79921),
        blue: h(0x458588),
        magenta: h(0xb16286),
        cyan: h(0x689d6a),
        white: h(0xa89984),
        bright_black: h(0x928374),
        bright_red: h(0xfb4934),
        bright_green: h(0xb8bb26),
        bright_yellow: h(0xfabd2f),
        bright_blue: h(0x83a598),
        bright_magenta: h(0xd3869b),
        bright_cyan: h(0x8ec07c),
        bright_white: h(0xebdbb2),
        dim_black: h(0x1d2021),
        dim_red: h(0x9d0006),
        dim_green: h(0x79740e),
        dim_yellow: h(0xb57614),
        dim_blue: h(0x076678),
        dim_magenta: h(0x8f3f71),
        dim_cyan: h(0x427b58),
        dim_white: h(0x7c6f64),
    }
}

pub fn solarized_dark() -> TerminalTheme {
    TerminalTheme {
        background: h(0x002b36),
        foreground: h(0x839496),
        bright_foreground: h(0xeee8d5),
        dim_foreground: h(0x586e75),
        ansi_background: h(0x002b36),
        cursor: h(0x839496),
        selection: ha(0x268bd2, 0.3),
        scrollbar_thumb: ha(0x839496, 0.4),
        link_text: h(0x268bd2),
        title_bar_background: h(0x00252e),
        title_bar_inactive_background: h(0x001e26),
        black: h(0x073642),
        red: h(0xdc322f),
        green: h(0x859900),
        yellow: h(0xb58900),
        blue: h(0x268bd2),
        magenta: h(0xd33682),
        cyan: h(0x2aa198),
        white: h(0xeee8d5),
        bright_black: h(0x586e75),
        bright_red: h(0xcb4b16),
        bright_green: h(0x859900),
        bright_yellow: h(0xb58900),
        bright_blue: h(0x268bd2),
        bright_magenta: h(0x6c71c4),
        bright_cyan: h(0x2aa198),
        bright_white: h(0xfdf6e3),
        dim_black: h(0x002b36),
        dim_red: h(0xa5261f),
        dim_green: h(0x647300),
        dim_yellow: h(0x886700),
        dim_blue: h(0x1c699e),
        dim_magenta: h(0x9e2862),
        dim_cyan: h(0x207974),
        dim_white: h(0xbbb6a3),
    }
}

/// Look up a bundled theme by name (case-insensitive).
pub fn theme_by_name(name: &str) -> Option<TerminalTheme> {
    let name_lower = name.to_lowercase();
    THEMES
        .iter()
        .find(|(n, _)| n.to_lowercase() == name_lower)
        .map(|(_, f)| f())
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
    let theme = active_theme();
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

/// Read the theme name from the PaneFlow config file.
/// Returns None if the file doesn't exist or has no theme set.
fn read_config_theme_name() -> Option<String> {
    paneflow_config::loader::load_config().theme
}

/// Resolve the theme from config, falling back to Catppuccin Mocha.
fn resolve_theme() -> TerminalTheme {
    if let Some(name) = read_config_theme_name() {
        if let Some(theme) = theme_by_name(&name) {
            return apply_surface_overrides(theme);
        }
        log::warn!("Unknown theme '{}', using default", name);
    }
    apply_surface_overrides(catppuccin_mocha())
}

// ---------------------------------------------------------------------------
// Theme cache — avoids disk I/O on every frame
// ---------------------------------------------------------------------------

/// Minimum interval between mtime checks (avoids stat() on every frame).
const THEME_CHECK_INTERVAL: Duration = Duration::from_millis(500);

struct CachedTheme {
    theme: TerminalTheme,
    mtime: Option<SystemTime>,
    last_check: Instant,
}

static THEME_CACHE: Mutex<Option<CachedTheme>> = Mutex::new(None);

/// Invalidate the theme cache so the next `active_theme()` call re-reads from disk.
pub fn invalidate_theme_cache() {
    let mut cache = THEME_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *cache = None;
}

/// Get the config file modification time for change detection.
pub fn config_mtime() -> Option<SystemTime> {
    let config_path = paneflow_config::loader::config_path()?;
    std::fs::metadata(config_path).ok()?.modified().ok()
}

/// Get the active theme. Caches the parsed theme and only re-reads from disk
/// when the config file's mtime has changed (checked at most every 500ms).
/// If the config is corrupted or missing, the last valid theme is used.
pub fn active_theme() -> TerminalTheme {
    let mut cache = THEME_CACHE.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(cached) = cache.as_ref() {
        // Throttle: only check mtime if enough time has passed
        if cached.last_check.elapsed() < THEME_CHECK_INTERVAL {
            return cached.theme;
        }
    }

    let current_mtime = config_mtime();
    let needs_reload = match (&*cache, current_mtime) {
        (None, _) => true,
        // Config file missing/unreadable — always reload to pick up recovery
        (_, None) => true,
        (Some(cached), Some(_)) => cached.mtime != current_mtime,
    };

    if needs_reload {
        let theme = resolve_theme();
        *cache = Some(CachedTheme {
            theme,
            mtime: current_mtime,
            last_check: Instant::now(),
        });
        theme
    } else {
        // mtime unchanged — update last_check and return cached theme
        let cached = cache.as_mut().unwrap();
        cached.last_check = Instant::now();
        cached.theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
