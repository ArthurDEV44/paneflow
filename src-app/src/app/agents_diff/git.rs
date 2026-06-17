//! Off-thread HEAD-relative diff build for the Agents dock.
//!
//! Shells the shared git pipeline ([`crate::diff::compute_head_diff`]) and turns
//! the result into the shared row model (unified + split) with syntax
//! highlighting, off the GPUI main thread. The product of this module is an
//! [`AgentsDiffBuilt`] that [`super::model::AgentsDiffData::loaded`] wraps in
//! `Rc`s back on the main thread.

use std::path::Path;

use crate::diff::{
    DiffSyntax, DisplayRow, RowKind, SplitRow, build_display_rows, build_split_rows,
    compute_head_diff,
};

/// Off-thread build result: the full (uncollapsed) display rows + anchors for
/// both view modes, plus the per-panel summary. Built in `smol::unblock` and
/// moved back to the main thread to seed an [`super::model::AgentsDiffData`].
pub(super) struct AgentsDiffBuilt {
    pub(super) unified: Vec<DisplayRow>,
    pub(super) split: Vec<SplitRow>,
    pub(super) anchors_unified: Vec<(String, usize)>,
    pub(super) anchors_split: Vec<(String, usize)>,
    pub(super) paths: Vec<String>,
    pub(super) file_count: usize,
    pub(super) added: u32,
    pub(super) removed: u32,
}

/// Off-thread builder: shell the HEAD-relative diff and turn it into the shared
/// row model (unified + split) with syntax highlighting. Mirrors the Review
/// view's column pipeline; safe to run in `smol::unblock`.
pub(super) fn build_agents_diff(
    cwd: &str,
    theme: crate::theme::TerminalTheme,
) -> Result<AgentsDiffBuilt, String> {
    let diff = compute_head_diff(Path::new(cwd));
    if let Some(e) = diff.error {
        return Err(e);
    }
    let syntax = DiffSyntax::from_theme(&theme);
    let (unified, _) = build_display_rows(&diff.files, Some(&syntax));
    let (split, _) = build_split_rows(&diff.files, Some(&syntax));
    // File path → header row index, in file order, so a body click can resolve
    // which file's header was hit (collapse toggle). Header rows are emitted one
    // per file in `diff.files` order, so zipping realigns them.
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
    let paths: Vec<String> = diff.files.iter().map(|f| f.path.clone()).collect();
    let (added, removed) = diff.files.iter().fold((0u32, 0u32), |(a, r), f| {
        let (fa, fr) = f.line_counts();
        (a + fa, r + fr)
    });
    Ok(AgentsDiffBuilt {
        unified,
        split,
        anchors_unified,
        anchors_split,
        file_count: diff.files.len(),
        paths,
        added,
        removed,
    })
}
