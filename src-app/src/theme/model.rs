//! Terminal theme data model and UI palette.

use gpui::{Hsla, Rgba};

use crate::terminal::element::{MIN_APCA_CONTRAST, ensure_minimum_contrast};

/// Terminal color theme with 36 slots:
/// 5 base + cursor + selection + selection_foreground + scrollbar_thumb +
/// link_text + 2 title bar + 24 ANSI (8 hues x 3 intensities).
#[derive(Clone, Copy)]
pub struct TerminalTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub bright_foreground: Hsla,
    pub dim_foreground: Hsla,
    pub ansi_background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    /// US-007: foreground color for text inside the selection rect,
    /// guaranteed to satisfy APCA Lc ≥ `MIN_APCA_CONTRAST` against
    /// `selection`. Computed once at theme-load time by
    /// [`TerminalTheme::recompute_selection_foreground`] from the theme's
    /// regular `foreground`. Used by `build_layout` to override the
    /// per-cell `fg` for cells inside the selection.
    pub selection_foreground: Hsla,
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

impl TerminalTheme {
    /// US-007: recompute [`Self::selection_foreground`] from the current
    /// `foreground` and `selection` colors so APCA Lc(selection_fg, selection)
    /// ≥ [`MIN_APCA_CONTRAST`]. Called at theme-load time (and on every
    /// hot-reload). Reusing the same `ensure_minimum_contrast` algorithm as
    /// per-cell text guarantees consistent visual semantics — selected text
    /// is no harder to read than non-selected text on near-luminance themes.
    pub(crate) fn recompute_selection_foreground(&mut self) {
        // The `selection` slot's alpha represents how the selection blends
        // with the cell background underneath. For contrast purposes we
        // approximate the perceived selection background as the opaque
        // version of `selection` (alpha = 1.0). A future refinement could
        // alpha-composite against the actual cell `background` for a
        // tighter contrast estimate, but the simple opaque-bg model is
        // what `MIN_APCA_CONTRAST` was tuned for in the per-cell path.
        let selection_bg_opaque = Hsla {
            a: 1.0,
            ..self.selection
        };
        self.selection_foreground =
            ensure_minimum_contrast(self.foreground, selection_bg_opaque, MIN_APCA_CONTRAST);
    }
}

pub(super) fn apply_surface_overrides(mut theme: TerminalTheme) -> TerminalTheme {
    if is_light_theme(&theme) {
        // US-007: light themes skip surface overrides but still need their
        // selection_foreground populated; do it here so every theme exiting
        // this function has a valid value regardless of branch taken.
        theme.recompute_selection_foreground();
        return theme;
    }

    let chrome_bg = h(CHROME_BACKGROUND_HEX);
    let terminal_bg = h(TERMINAL_BACKGROUND_HEX);
    theme.title_bar_background = chrome_bg;
    theme.title_bar_inactive_background = chrome_bg;
    theme.background = terminal_bg;
    theme.ansi_background = terminal_bg;
    theme.recompute_selection_foreground();
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
    let colors = if is_light {
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
    };

    // DIAGNOSTIC B — fires once per process. Surfaces the actual Hsla
    // values resolved for the UI palette so we can rule in/out the
    // "text rendered but invisible" hypothesis (alpha=0 or
    // foreground == background after `Rgba` -> `Hsla` conversion on
    // the platform). If `text.a < 1.0` or `text` is numerically
    // identical to a background color, glyphs paint but cannot be
    // distinguished from the surface beneath them. Also logs
    // `is_light` and the source `theme.background` so a misclassified
    // light/dark branch is immediately visible.
    {
        use std::sync::Once;
        static LOG_ONCE: Once = Once::new();
        LOG_ONCE.call_once(|| {
            log::info!(
                "ui_colors diagnostic: is_light={} \
                 theme.background=(h={:.3},s={:.3},l={:.3},a={:.3}) \
                 text=(h={:.3},s={:.3},l={:.3},a={:.3}) \
                 muted=(h={:.3},s={:.3},l={:.3},a={:.3}) \
                 base=(h={:.3},s={:.3},l={:.3},a={:.3}) \
                 surface=(h={:.3},s={:.3},l={:.3},a={:.3})",
                is_light,
                theme.background.h,
                theme.background.s,
                theme.background.l,
                theme.background.a,
                colors.text.h,
                colors.text.s,
                colors.text.l,
                colors.text.a,
                colors.muted.h,
                colors.muted.s,
                colors.muted.l,
                colors.muted.a,
                colors.base.h,
                colors.base.s,
                colors.base.l,
                colors.base.a,
                colors.surface.h,
                colors.surface.s,
                colors.surface.l,
                colors.surface.a,
            );
        });
    }

    colors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::element::apca_contrast;
    use crate::theme::builtin::{
        catppuccin_mocha, dracula, gruvbox_dark, one_dark, paneflow_light, solarized_dark,
        theme_by_name,
    };

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

    /// US-007 invariant: for any theme exiting `apply_surface_overrides` or
    /// `theme_by_name`, the selection foreground must satisfy the same
    /// APCA Lc threshold the per-cell contrast pass uses. A near-luminance
    /// theme without this invariant would render selected text illegibly.
    fn assert_selection_invariant(theme: &TerminalTheme, label: &str) {
        let bg_opaque = Hsla {
            a: 1.0,
            ..theme.selection
        };
        let lc = apca_contrast(theme.selection_foreground, bg_opaque).abs();
        assert!(
            lc >= MIN_APCA_CONTRAST,
            "{label}: APCA Lc({lc}) < {MIN_APCA_CONTRAST} for selection_foreground vs selection"
        );
    }

    #[test]
    fn bundled_themes_satisfy_selection_contrast_invariant() {
        // All 6 bundled themes must produce a readable selection foreground.
        for (label, theme) in [
            (
                "Catppuccin Mocha",
                apply_surface_overrides(catppuccin_mocha()),
            ),
            ("PaneFlow Light", apply_surface_overrides(paneflow_light())),
            ("One Dark", apply_surface_overrides(one_dark())),
            ("Dracula", apply_surface_overrides(dracula())),
            ("Gruvbox Dark", apply_surface_overrides(gruvbox_dark())),
            ("Solarized Dark", apply_surface_overrides(solarized_dark())),
        ] {
            assert_selection_invariant(&theme, label);
        }
    }

    #[test]
    fn theme_by_name_returns_invariant_satisfying_themes() {
        // theme_by_name is the public entry point; users may call it without
        // going through apply_surface_overrides, so it must finalize on the
        // way out.
        for name in [
            "Catppuccin Mocha",
            "PaneFlow Light",
            "One Dark",
            "Dracula",
            "Gruvbox Dark",
            "Solarized Dark",
        ] {
            let theme = theme_by_name(name).expect("bundled theme not found");
            assert_selection_invariant(&theme, name);
        }
    }

    #[test]
    fn adversarial_selection_close_to_red_text_still_legible() {
        // Construct a synthetic theme whose `selection` background is a
        // strong red, the same hue as theme.foreground, then assert the
        // recomputed selection_foreground still satisfies the invariant.
        // This is the canonical "user picked a clashing selection color"
        // failure mode US-007 must guard against.
        let mut theme = catppuccin_mocha();
        theme.foreground = h(0xff0000); // bright red text
        theme.selection = ha(0xff0000, 0.4); // selection of the same hue
        theme.recompute_selection_foreground();
        assert_selection_invariant(&theme, "adversarial-red-on-red");
    }

    #[test]
    fn adversarial_selection_close_to_white_on_light_theme() {
        // Light theme + near-white selection background. The algorithm must
        // pick a dark foreground for legibility.
        let mut theme = paneflow_light();
        theme.foreground = h(0xeeeeee); // near-white text
        theme.selection = ha(0xf0f0f0, 0.5); // very pale selection
        theme.recompute_selection_foreground();
        assert_selection_invariant(&theme, "adversarial-white-on-light");
    }

    #[test]
    fn recompute_is_idempotent() {
        // Running the recompute twice must yield the same value — guards
        // against accidental mutation of `foreground` or `selection` during
        // the algorithm.
        let mut theme = catppuccin_mocha();
        theme.recompute_selection_foreground();
        let first = theme.selection_foreground;
        theme.recompute_selection_foreground();
        let second = theme.selection_foreground;
        assert_eq!(first, second);
    }
}
