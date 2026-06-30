//! Files sidebar live filesystem watch + per-workspace expansion persistence
//! (PRD `prd-files-tree-sidebar-2026-Q3`, EP-002).
//!
//! `spawn_files_hydration` reads the tree + registers the recursive `notify`
//! watch OFF the render thread (US-018; US-005 wiring, degrading gracefully on
//! failure per US-006); the background drain loop in `bootstrap` debounces +
//! coalesces events and calls `refresh_files_dirs` for the targeted
//! per-directory re-read. `sync_files_expansion` mirrors the live expansion into
//! the active `Workspace` so it persists to `session.json` (US-007). Split out
//! of `mod.rs` to keep each file under the 250-line budget.

use std::path::{Path, PathBuf};

use gpui::Context;

use crate::PaneFlowApp;
use crate::app::files_tree;

impl PaneFlowApp {
    /// Mirror the live tree's expansion into the active workspace (excluding
    /// the implicit root) so it survives close/reopen and persists to
    /// `session.json` (US-007).
    pub(super) fn sync_files_expansion(&mut self) {
        let root = self.files_tree.root.clone();
        let mut expanded: Vec<PathBuf> = self
            .files_tree
            .expanded
            .iter()
            .filter(|p| **p != root)
            .cloned()
            .collect();
        expanded.sort();
        if let Some(ws) = self.workspaces.get_mut(self.active_idx) {
            ws.files_expanded = expanded;
        }
    }

    /// US-018: hydrate the Files tree and install the recursive watcher OFF the
    /// GPUI main thread.
    ///
    /// A recursive `notify` watch walks the entire subtree at registration
    /// (inotify adds one watch per directory - ~23k for a repo carrying a
    /// `target/`), which previously froze the render thread ("not responding"
    /// on Wayland). Both the directory reads ([`FilesTreeState::hydrated`]) and
    /// the watch registration now run on a background task; results are
    /// re-injected only if the sidebar is still open on the SAME root (it may
    /// close or re-root during the walk - EP-003 identity guard). A root shell
    /// renders immediately so the panel never looks frozen.
    pub(crate) fn spawn_files_hydration(
        &mut self,
        root: PathBuf,
        persisted: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        // Drop the previous watch + channel immediately (cheap), and show a
        // root shell so the panel paints this frame while the reads run.
        self.files_watcher = None;
        self.files_event_rx = None;
        self.files_tree = files_tree::FilesTreeState::root_shell(root.clone());

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                // Stage 1 - directory reads (fast): inject the populated tree first
                // so content appears before the (slower) recursive watch walk ends.
                let tree = smol::unblock({
                    let root = root.clone();
                    let persisted = persisted.clone();
                    move || files_tree::FilesTreeState::hydrated(root, &persisted)
                })
                .await;
                let still_current = this
                    .update(cx, |app, cx| {
                        if app.files_sidebar_open && app.files_tree.root == root {
                            app.files_tree = tree;
                            app.sync_files_expansion();
                            app.clamp_files_selection();
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !still_current {
                    return;
                }

                // Stage 2 - recursive watch registration (the slow walk): inject the
                // handles when ready so live updates start, still off the render thread.
                let built = smol::unblock({
                    let root = root.clone();
                    move || build_files_watcher(&root)
                })
                .await;
                let _ = this.update(cx, |app, _cx| {
                    if app.files_sidebar_open
                        && app.files_tree.root == root
                        && let Some((watcher, rx)) = built
                    {
                        app.files_watcher = Some(watcher);
                        app.files_event_rx = Some(rx);
                    }
                });
            },
        )
        .detach();
    }

    /// Apply a debounced, prefix-coalesced batch of changed directories
    /// (US-005), called from the background drain loop in `bootstrap`. Re-reads
    /// only the cached (expanded) directories among the affected parents - a
    /// change under a collapsed/uncached dir is ignored until it's expanded
    /// (then read fresh by `toggle_dir`). `rescan` (a notify overflow/Rescan
    /// signal, US-006 AC3) forces a root re-read. Never walks the whole tree.
    pub(crate) fn refresh_files_dirs(
        &mut self,
        mut dirs: Vec<PathBuf>,
        rescan: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.files_sidebar_open {
            return;
        }
        let root = self.files_tree.root.clone();
        if rescan {
            dirs.push(root.clone());
        }
        let mut changed = false;
        for dir in files_tree::coalesce_by_prefix(dirs) {
            // AC4: only re-read directories we've already cached (expanded).
            if let std::collections::hash_map::Entry::Occupied(mut e) =
                self.files_tree.children.entry(dir.clone())
            {
                e.insert(files_tree::read_dir_sorted(&root, &dir));
                changed = true;
            }
        }
        if changed {
            self.clamp_files_selection();
            cx.notify();
        }
    }
}

/// US-018: build a recursive `notify` watch on `root`, returning the watcher +
/// its event channel, or `None` (logged) on failure - notably Linux `ENOSPC`
/// when a large repo exhausts `fs.inotify.max_user_watches` (default often
/// 8192). The caller falls back to on-expand reads (US-006); raise the kernel
/// limit with `sudo sysctl fs.inotify.max_user_watches=524288` (persist in
/// `/etc/sysctl.d/`). macOS/Windows use FSEvents / ReadDirectoryChangesW and
/// don't hit the limit.
///
/// Runs on a background thread (the recursive registration walk is the freeze
/// US-018 fixes); the caller re-injects the returned handles.
#[allow(clippy::type_complexity)]
fn build_files_watcher(
    root: &Path,
) -> Option<(
    notify::RecommendedWatcher,
    std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
)> {
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("files watcher unavailable: {e}; falling back to on-expand reads");
            return None;
        }
    };
    if let Err(e) = watcher.watch(root, notify::RecursiveMode::Recursive) {
        log::warn!(
            "files watcher: failed to watch {} ({e}); falling back to on-expand reads",
            root.display()
        );
        return None;
    }
    Some((watcher, rx))
}
