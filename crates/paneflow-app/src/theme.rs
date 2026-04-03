// US-014: Centralized Color Theme
//
// All UI colors flow from UiTheme. No hardcoded Color::from_rgb() in view methods.
// The accent color is configurable via paneflow.json `accent_color` field.

use iced::Color;

/// Centralized UI color theme for all PaneFlow surfaces.
#[derive(Debug, Clone)]
pub struct UiTheme {
    /// Primary accent color (default: #0091FF cmux blue)
    pub accent: Color,
    /// Sidebar background
    pub sidebar_bg: Color,
    /// Main content area background
    pub content_bg: Color,
    /// Primary text (titles, active labels)
    pub text_primary: Color,
    /// Secondary text (subtitles, counts)
    pub text_secondary: Color,
    /// Muted text (section headers, placeholders)
    pub text_muted: Color,
    /// Border color (panels, inputs)
    pub border: Color,
    /// Split divider color
    pub divider: Color,
    /// Button/item background (add button, palette items)
    pub surface: Color,
    /// Command palette / overlay background
    pub overlay_bg: Color,
    /// Scrim color for modal overlays
    pub scrim: Color,
    /// Focus border color (derived from accent)
    pub focus_border: Color,
    /// Terminal pane background
    pub terminal_bg: Color,
}

impl UiTheme {
    /// Build the default dark theme, optionally overriding the accent color.
    pub fn new(accent_hex: Option<&str>) -> Self {
        let accent = accent_hex
            .and_then(parse_hex_color)
            .unwrap_or(Color::from_rgb(0.0, 0.569, 1.0)); // #0091FF

        Self {
            accent,
            sidebar_bg: Color::from_rgb(0.110, 0.110, 0.129), // #1C1C21
            content_bg: Color::from_rgb(0.059, 0.059, 0.078), // #0F0F14
            text_primary: Color::from_rgb(0.851, 0.851, 0.867), // #D9D9DD
            text_secondary: Color::from_rgb(0.502, 0.502, 0.525), // #808086
            text_muted: Color::from_rgb(0.290, 0.290, 0.322), // #4A4A52
            border: Color::from_rgb(0.165, 0.165, 0.196),     // #2A2A32
            divider: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
            surface: Color::from_rgb(0.149, 0.149, 0.180), // #26262E
            overlay_bg: Color::from_rgb(0.118, 0.118, 0.145), // #1E1E25
            scrim: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
            focus_border: accent,
            terminal_bg: Color::from_rgb(0.059, 0.059, 0.078), // same as content_bg
        }
    }

    /// Accent color with reduced opacity (for badges on selected items).
    pub fn accent_muted(&self) -> Color {
        Color::from_rgba(self.accent.r, self.accent.g, self.accent.b, 0.25)
    }

    /// Divider color on hover (brighter). Used in US-012.
    #[allow(dead_code)]
    pub fn divider_hover(&self) -> Color {
        Color::from_rgba(1.0, 1.0, 1.0, 0.2)
    }
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::new(None)
    }
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    Some(Color::from_rgb(r, g, b))
}
