//! Files sidebar live filesystem watch + per-workspace expansion persistence
//! (PRD `prd-files-tree-sidebar-2026-Q3`, EP-002).
//!
//! `install_files_watcher` registers the recursive `notify` watch (US-005,
//! degrading gracefully on failure per US-006); the background drain loop in
//! `bootstrap` debounces + coalesces events and calls `refresh_files_dirs` for
//! the targeted per-directory re-read. `sync_files_expansion` mirrors the live
//! expansion into the active `Workspace` so it persists to `session.json`
//! (US-007). Split out of `mod.rs` to keep each file under the 250-line budget.

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
        let expanded: Vec<PathBuf> = self
            .files_tree
            .expanded
            .iter()
            .filter(|p| **p != root)
            .cloned()
            .collect();
        if let Some(ws) = self.workspaces.get_mut(self.active_idx) {
            ws.files_expanded = expanded;
        }
    }

    /// Install a recursive `notify` watcher on the tree root (US-005),
    /// replacing any previous handle. On failure — notably Linux `ENOSPC` when
    /// a large repo exhausts `fs.inotify.max_user_watches` (default often
    /// 8192) — it logs and leaves the watcher unset; the tree still renders and
    /// expands, and `toggle_dir` re-reads on every expand so manual navigation
    /// stays correct without live push (US-006). Raise the kernel limit with:
    ///   `sudo sysctl fs.inotify.max_user_watches=524288` (persist in
    ///   `/etc/sysctl.d/`). The limit is Linux-specific; macOS/Windows use
    ///   FSEvents / ReadDirectoryChangesW and don't hit it.
    pub(super) fn install_files_watcher(&mut self, root: &Path) {
        use notify::Watcher;
        // Drop the previous watch + channel first so we never hold two
        // recursive watches at once (US-005: re-root drops the old handle).
        self.files_watcher = None;
        self.files_event_rx = None;

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("files watcher unavailable: {e}; falling back to on-expand reads");
                return;
            }
        };
        if let Err(e) = watcher.watch(root, notify::RecursiveMode::Recursive) {
            log::warn!(
                "files watcher: failed to watch {} ({e}); falling back to on-expand reads",
                root.display()
            );
            return;
        }
        self.files_watcher = Some(watcher);
        self.files_event_rx = Some(rx);
    }

    /// Apply a debounced, prefix-coalesced batch of changed directories
    /// (US-005), called from the background drain loop in `bootstrap`. Re-reads
    /// only the cached (expanded) directories among the affected parents — a
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
            cx.notify();
        }
    }
}
