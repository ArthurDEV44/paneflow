//! Terminal theming with 35 color slots compatible with Zed's terminal theme format.

mod builtin;
mod model;
mod watcher;

pub use builtin::{THEMES, ThemeEntry, one_dark, paneflow_light, theme_by_name};
pub use model::{TerminalTheme, UiColors, ui_colors};
pub use watcher::{ThemeWatcher, active_theme, config_mtime, invalidate_theme_cache};
