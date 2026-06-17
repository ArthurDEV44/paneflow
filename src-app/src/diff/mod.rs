//! Multi-worktree diff viewer (prd-multi-worktree-diff-2026-Q3.md).
//!
//! EP-001 scaffold (US-003): stands up the `DiffView` GPUI entity and its tab
//! plumbing only — it renders an empty/placeholder state seeded with the
//! sibling worktrees of one repo. The diff engine (EP-002), side-by-side
//! render (EP-003), and N-column live view + base selector (EP-004) fill in
//! `DiffView` with real hunk data on top of this host.
//!
//! `DiffView` is the exact structural analog of `markdown::MarkdownView`: an
//! `Entity` implementing `Render + Focusable`, hosted in a pane via the new
//! `TabContent::Diff` variant. It is ephemeral — never persisted to
//! `session.json` (like markdown tabs, dropped by `layout/serde.rs`).

mod align;
mod arrange;
mod element;
mod engine;
mod extract;
mod git;
mod highlighter;
mod hit_test;
mod multi_view;
mod review_terminal;
mod rows;
mod scope;
mod scope_header;
mod syntax;
mod view;
mod worddiff;

// Only the host view + its seed type are consumed outside this module
// (`pane::TabContent::Diff`, `event_handlers::open_multi_diff_for_repo`). The
// engine / git / rows types stay crate-internal, reached via `super::` paths.
pub use git::{FileChange, list_repo_worktrees};
pub use multi_view::MultiRepoDiffView;
pub use scope::{DiffScope, RepoGroup};
pub use view::{
    DiffView, DiffViewEvent, DiffWorktree, FileEntry, FileListState, aggregate_file_lists,
};

// EP-001 (prd-review-redesign-2026-Q3.md, US-001/US-002): the Agents diff dock
// (`crate::app::agents_diff`) renders through the SAME `DiffElement` + git
// pipeline + row model as the Review view, so these are exposed crate-internally
// rather than re-implemented. Kept `pub(crate)` (not `pub`) so the unification
// surface stays inside the binary.
pub(crate) use element::{DiffBody, DiffElement};
pub(crate) use git::compute_head_diff;
pub(crate) use hit_test::row_at_offset;
pub(crate) use rows::{
    DisplayRow, RowKind, SplitRow, apply_collapse_split, apply_collapse_unified,
    build_display_rows, build_split_rows, palette, split_max_line_no, split_offsets,
    unified_max_line_no, unified_offsets,
};
pub(crate) use syntax::DiffSyntax;
