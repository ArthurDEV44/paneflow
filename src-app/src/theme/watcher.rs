//! Theme cache with mtime-based hot-reload from `paneflow.json`.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use super::builtin::{catppuccin_mocha, theme_by_name};
use super::model::{TerminalTheme, apply_surface_overrides};

/// Minimum interval between mtime checks (avoids stat() on every frame).
const THEME_CHECK_INTERVAL: Duration = Duration::from_millis(500);

struct CachedTheme {
    theme: TerminalTheme,
    mtime: Option<SystemTime>,
    last_check: Instant,
}

static THEME_CACHE: Mutex<Option<CachedTheme>> = Mutex::new(None);

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
