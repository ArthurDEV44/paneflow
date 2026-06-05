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
