//! Diff load lifecycle + per-column generation guards for the Review view
//! (US-004 code-motion). See [`super`] for the `DiffView` definition.

use super::*;

impl DiffView {
    /// (Re)load every visible column's diff off the main thread. One background
    /// task per column — a slow worktree never blocks the others; the
    /// `generation` guard discards results superseded by a newer load (US-016
    /// keeps the task count bounded to the visible columns).
    pub(super) fn start_loading(&mut self, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, _)| i)
            .collect();
        self.start_loading_columns(&indices, cx);
    }

    /// (Re)load a specific set of columns' diffs off the main thread. The full
    /// [`Self::start_loading`] passes every visible column; US-016
    /// [`Self::revalidate`] passes only the columns whose git fingerprint moved
    /// while the surface was hidden. One background task per column — a slow
    /// worktree never blocks the others; the `generation` guard discards results
    /// superseded by a newer load (US-007 last-write-wins).
    pub(super) fn start_loading_columns(&mut self, indices: &[usize], cx: &mut Context<Self>) {
        let shared_base = self.base_ref.clone();
        // Snapshot the active theme on the main thread; `TerminalTheme` is `Copy`
        // so each column's background task gets its own copy to derive syntax
        // colors from, without touching the theme cache off-thread.
        let theme = crate::theme::active_theme();
        log::debug!(
            "diff: start_loading base={shared_base:?} ({} of {} columns)",
            indices.len(),
            self.columns.len()
        );
        for &i in indices {
            // Bump THIS column's generation + resolve its effective base (per-column
            // override, else the shared base) under one `get_mut`. Per-column gen so
            // a subset reload (e.g. `revalidate`) never discards an in-flight load of
            // the OTHER columns. Do NOT blank an already-loaded column to `Loading`
            // on a refresh — keep its content until the new diff swaps in (no flash).
            let (generation, base, path, branch) = match self.columns.get_mut(i) {
                Some(col) if col.visible => {
                    col.generation = col.generation.wrapping_add(1);
                    let base = col
                        .base_override
                        .clone()
                        .unwrap_or_else(|| shared_base.clone());
                    (col.generation, base, col.path.clone(), col.branch.clone())
                }
                _ => continue,
            };
            // No base resolved (no develop/main/master, or the user cleared it):
            // prompt instead of spawning a diff against a non-existent ref.
            if base.is_empty() {
                if let Some(col) = self.columns.get_mut(i) {
                    col.state = ColumnState::Failed("Select a base branch".to_string());
                }
                continue;
            }
            log::debug!("diff: col {i} ({branch}) task SPAWNED (gen={generation})");
            cx.spawn(async move |this, cx| {
                // The whole pipeline — git diff, row building, AND the syntect
                // pass — runs off the GPUI main thread; only the `Rc` wrap +
                // assignment happen back on it (NFR: 0 ms main-thread git/diff).
                log::debug!("diff: col {i} ({branch}) task STARTED (polled)");
                let bc = branch.clone();
                let built = smol::unblock(move || {
                    // US-016: snapshot the fingerprint BEFORE reading the tree, so a
                    // commit landing mid-build makes the stored fingerprint LAG the
                    // rows — `revalidate` then sees HEAD moved and reloads (a harmless
                    // extra reload) rather than matching a stale fingerprint and
                    // showing pre-commit rows as current (the unsafe direction).
                    let fingerprint = super::super::git::column_fingerprint(&path, &base);
                    let t0 = Instant::now();
                    let diff = super::super::git::compute_worktree_diff(&path, &base);
                    log::debug!(
                        "diff: col {i} ({bc}) computed {} files in {:?} (error={:?})",
                        diff.files.len(),
                        t0.elapsed(),
                        diff.error
                    );
                    if let Some(e) = diff.error {
                        return Built::Failed(e);
                    }
                    let t1 = Instant::now();
                    let syntax = SYNTAX_HIGHLIGHT_ENABLED
                        .then(|| super::super::syntax::DiffSyntax::from_theme(&theme));
                    let (unified, _) = build_display_rows(&diff.files, syntax.as_ref());
                    let (split, _) = build_split_rows(&diff.files, syntax.as_ref());
                    // File path -> header row index, in file order, so a sidebar
                    // click can scroll the body to that file. Header rows are
                    // emitted one per file in `diff.files` order, so zipping
                    // realigns them (the zip naturally truncates if the row cap
                    // dropped trailing headers).
                    let anchors_unified: Vec<(String, usize)> = diff
                        .files
                        .iter()
                        .map(|f| f.path.clone())
                        .zip(
                            unified
                                .iter()
                                .enumerate()
                                .filter(|(_, r)| r.kind == RowKind::FileHeader)
                                .map(|(i, _)| i),
                        )
                        .collect();
                    let anchors_split: Vec<(String, usize)> = diff
                        .files
                        .iter()
                        .map(|f| f.path.clone())
                        .zip(
                            split
                                .iter()
                                .enumerate()
                                .filter(|(_, r)| matches!(r, SplitRow::Header(_)))
                                .map(|(i, _)| i),
                        )
                        .collect();
                    // US-008: lightweight per-file summary for the git panel,
                    // built here (off-thread) from the same FileDiffs.
                    let files = diff
                        .files
                        .iter()
                        .map(|f| {
                            let (added, removed) = f.line_counts();
                            FileEntry {
                                path: f.path.clone(),
                                change: f.change,
                                old_path: f.old_path.clone(),
                                added,
                                removed,
                                is_binary: f.is_binary,
                            }
                        })
                        .collect();
                    log::debug!(
                        "diff: col {i} ({bc}) built {} unified / {} split rows in {:?}",
                        unified.len(),
                        split.len(),
                        t1.elapsed()
                    );
                    // EP-004 US-014: match local agent sessions to this worktree
                    // in the SAME off-thread pass (no second async round-trip).
                    // Enrichment only — a parse miss yields an empty Vec and the
                    // diff is unaffected.
                    let cwd = path.to_string_lossy();
                    let attribution =
                        crate::agent_sessions::attribution_for_column(&cwd, &bc);
                    Built::Loaded {
                        unified,
                        split,
                        file_count: diff.files.len(),
                        files,
                        anchors_unified,
                        anchors_split,
                        // Move the raw FileDiffs out for copy/review (US-001..005);
                        // every `&diff.files` consumer above has finished borrowing.
                        files_full: diff.files,
                        fingerprint: Box::new(fingerprint),
                        attribution,
                    }
                })
                .await;
                log::debug!("diff: col {i} ({branch}) off-thread done, applying on main thread");
                cx.update(|cx| {
                    let _ = this.update(cx, |view: &mut Self, cx| {
                        let Some(col) = view.columns.get_mut(i) else {
                            return;
                        };
                        if col.generation != generation {
                            // Not an error: a newer load (bootstrap + watcher
                            // overlap, base-branch switch, resize) bumped this
                            // column's generation while we were off-thread, so
                            // last-write-wins drops the stale result. Trace it
                            // at debug — a WARN here just cried wolf on the
                            // race guard doing its job.
                            log::debug!(
                                "diff: col {i} ({branch}) superseded — task gen={generation} != col gen={}",
                                col.generation
                            );
                            return; // superseded by a newer load of this column
                        }
                        let new_state = match built {
                            Built::Failed(e) => {
                                log::warn!("diff: col {i} ({branch}) FAILED: {e}");
                                ColumnState::Failed(e)
                            }
                            Built::Loaded {
                                unified,
                                split,
                                file_count,
                                files,
                                anchors_unified,
                                anchors_split,
                                files_full,
                                fingerprint,
                                attribution,
                            } => {
                                log::debug!("diff: col {i} ({branch}) LOADED ({file_count} files)");
                                // US-016: stamp the fingerprint these rows were
                                // built against, for warm-resume revalidation.
                                col.fingerprint = Some(*fingerprint);
                                // EP-004 US-014: cache the matched sessions on the
                                // column (re-fetched only on re-diff).
                                col.attribution = attribution;
                                ColumnState::Loaded {
                                    unified: Rc::new(unified),
                                    split: Rc::new(split),
                                    file_count,
                                    files: Rc::new(files),
                                    anchors_unified: Rc::new(anchors_unified),
                                    anchors_split: Rc::new(anchors_split),
                                    files_full: Rc::new(files_full),
                                }
                            }
                        };
                        col.state = new_state;
                        // Rebuild the collapse-filtered views from the fresh rows
                        // (carries any per-file collapse across the reload).
                        col.recompute_display();
                        // A reload can reorder or drop entries in this column's
                        // `files_full`, which an open body context menu indexes by
                        // position. Drop a menu targeting this column so a menu
                        // action can never land on the wrong file after a live
                        // refresh.
                        if view.body_menu.as_ref().is_some_and(|m| m.col_idx == i) {
                            view.body_menu = None;
                        }
                        cx.notify();
                    });
                });
            })
            .detach();
        }
        // Repaint now so any column set to `Failed` (empty base) above shows its
        // prompt immediately; loaded columns also repaint when their task applies.
        cx.notify();
    }

    /// Per-branch changed-file lists for the multi-branch diff sidebar: one entry
    /// per visible column as `(branch, column index, worktree path, file-list
    /// state)`. The worktree path is the stable, globally-unique key the sidebar
    /// uses for per-section collapse state — branch NAMES collide across repos in
    /// Multi-project scope (every repo has a `main`). Reads the same `Rc`-shared
    /// file vecs, so it is allocation-cheap per frame.
    pub fn column_file_lists(&self) -> Vec<(String, usize, PathBuf, FileListState)> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, c)| {
                let state = match &c.state {
                    ColumnState::Loading => FileListState::Loading,
                    ColumnState::Failed(e) => FileListState::Failed(e.clone()),
                    ColumnState::Loaded { files, .. } => FileListState::Loaded(files.clone()),
                };
                (c.branch.clone(), i, c.path.clone(), state)
            })
            .collect()
    }

    /// Index of the column whose file list currently drives the sidebar/diffstat
    /// (so the sidebar can mark the active branch's section).
    pub fn selected_column(&self) -> usize {
        self.selected_column
    }

    /// Select `col_idx` (focus its file list) AND scroll its body to `path`.
    /// Used by the multi-branch sidebar so clicking a file in ANY branch section
    /// focuses that branch and lands on the file — `jump_to_file` keys off the
    /// just-set `selected_column`.
    pub fn select_and_jump(
        &mut self,
        col_idx: usize,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_column(col_idx, cx);
        self.jump_to_file(path, window, cx);
    }
}
