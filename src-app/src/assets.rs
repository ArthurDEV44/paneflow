use std::borrow::Cow;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*"]
#[include = "fonts/**/*"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| {
                if p.starts_with(path) {
                    Some(SharedString::from(p.to_string()))
                } else {
                    None
                }
            })
            .collect())
    }
}

impl Assets {
    /// Register every `.ttf` under `assets/fonts/` with GPUI's text system
    /// in one batch. Mirrors Zed's `Assets::load_fonts` pattern at
    /// `zed/crates/assets/src/assets.rs:42-55`: iterating the embed registry
    /// means dropping a new `.ttf` into `assets/fonts/` is enough to ship
    /// it — no Rust edit, no name list to maintain, no cargo recompile of
    /// a `LazyLock` of hardcoded paths.
    ///
    /// Skips non-`.ttf` files so the `OFL.txt` / `LICENSE` companions sit
    /// alongside the font binaries without needing a separate include set.
    pub fn load_fonts(&self, cx: &App) -> Result<()> {
        let font_paths = self.list("fonts/")?;
        let mut embedded_fonts = Vec::with_capacity(font_paths.len());
        for path in &font_paths {
            // GPUI's text system accepts TTF and OTF; we only ship TTFs
            // today, but allow OTF too so a future swap doesn't need a
            // matching code change.
            let lower = path.to_lowercase();
            if !lower.ends_with(".ttf") && !lower.ends_with(".otf") {
                continue;
            }
            let data = self
                .load(path)?
                .ok_or_else(|| anyhow::anyhow!("embedded font {path} listed but not loadable"))?;
            embedded_fonts.push(data);
        }
        if embedded_fonts.is_empty() {
            log::warn!(
                "Assets::load_fonts: no .ttf/.otf found under fonts/ — \
                 the rust-embed include set may have drifted"
            );
            return Ok(());
        }
        let count = embedded_fonts.len();
        cx.text_system().add_fonts(embedded_fonts)?;
        log::info!("Assets::load_fonts: registered {count} embedded font file(s) with GPUI");
        Ok(())
    }
}

/// US-008 — embedded AI-hook binaries.
///
/// `paneflow-shim` (mapped at extraction time to `claude` + `codex`) and
/// `paneflow-ai-hook` are staged into `src-app/target/embed/bin/<target>/`
/// by `build.rs` before rust-embed's proc-macro expands. Entries look like
/// `bin/<target-triple>/paneflow-shim[.exe]` and
/// `bin/<target-triple>/paneflow-ai-hook[.exe]`.
///
/// Not cfg-gated on target_os: PaneFlow only builds for Linux, macOS, and
/// Windows per `CLAUDE.md` mandate. A compile failure on any other OS is
/// the correct outcome — there is no build path that would populate the
/// embed folder anyway. Gating here would only move the failure from
/// rust-embed (empty folder ⇒ panic) to `ai_hooks::extract` (missing
/// symbol ⇒ compile error), with no benefit.
///
/// Consumers: `ai_hooks::extract::ensure_binaries_extracted` at runtime
/// (wired into `terminal::pty_session::inject_ai_hook_env` by US-009).
#[derive(RustEmbed)]
#[folder = "target/embed/bin"]
#[prefix = "bin/"]
pub struct Bins;
