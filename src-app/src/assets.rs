use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};
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
