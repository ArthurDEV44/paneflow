//! Terminal theming with 30 color slots compatible with Zed's terminal theme format.

use gpui::{Hsla, Rgba};

/// Terminal color theme with 30 slots:
/// 5 base colors + 24 ANSI colors (8 hues x 3 intensities) + cursor.
pub struct TerminalTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub bright_foreground: Hsla,
    pub dim_foreground: Hsla,
    pub ansi_background: Hsla,
    pub cursor: Hsla,
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

// ---------------------------------------------------------------------------
// Bundled themes
// ---------------------------------------------------------------------------

pub type ThemeEntry = (&'static str, fn() -> TerminalTheme);

pub static THEMES: &[ThemeEntry] = &[
    ("Catppuccin Mocha", catppuccin_mocha),
    ("One Dark", one_dark),
    ("Dracula", dracula),
    ("Gruvbox Dark", gruvbox_dark),
    ("Solarized Dark", solarized_dark),
];

pub fn catppuccin_mocha() -> TerminalTheme {
    TerminalTheme {
        background: h(0x1e1e2e),
        foreground: h(0xcdd6f4),
        bright_foreground: h(0xcdd6f4),
        dim_foreground: h(0x8b91a6),
        ansi_background: h(0x1e1e2e),
        cursor: h(0xf5e0dc),
        black: h(0x45475a),
        red: h(0xf38ba8),
        green: h(0xa6e3a1),
        yellow: h(0xf9e2af),
        blue: h(0x89b4fa),
        magenta: h(0xf5c2e7),
        cyan: h(0x94e2d5),
        white: h(0xbac2de),
        bright_black: h(0x585b70),
        bright_red: h(0xf38ba8),
        bright_green: h(0xa6e3a1),
        bright_yellow: h(0xf9e2af),
        bright_blue: h(0x89b4fa),
        bright_magenta: h(0xf5c2e7),
        bright_cyan: h(0x94e2d5),
        bright_white: h(0xa6adc8),
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

pub fn one_dark() -> TerminalTheme {
    TerminalTheme {
        background: h(0x282c34),
        foreground: h(0xabb2bf),
        bright_foreground: h(0xabb2bf),
        dim_foreground: h(0x5c6370),
        ansi_background: h(0x282c34),
        cursor: h(0x528bff),
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

/// Read the theme name from the PaneFlow config file.
/// Returns None if the file doesn't exist or has no theme set.
fn read_config_theme_name() -> Option<String> {
    paneflow_config::loader::load_config().theme
}

/// Get the active theme — reads from config if available, falls back to Catppuccin Mocha.
pub fn active_theme() -> TerminalTheme {
    if let Some(name) = read_config_theme_name() {
        if let Some(theme) = theme_by_name(&name) {
            return theme;
        }
        log::warn!("Unknown theme '{}', using default", name);
    }
    catppuccin_mocha()
}

/// Get the config file modification time for change detection.
pub fn config_mtime() -> Option<std::time::SystemTime> {
    let config_path = paneflow_config::loader::config_path()?;
    std::fs::metadata(config_path).ok()?.modified().ok()
}
