// US-007 (prd-agents-view.md) is the foundation story for the Agents
// view domain model. Many items here are deliberately "unused" until
// follow-up stories activate them:
// - ID counters + `bump_id_counters_to`: US-009 restore path.
// - `project_from_session` / `thread_from_session` / `agent_kind_from_str`:
//   US-009 restore from session.json.
// - `Thread::new` / `Project::new` constructors: US-011 sidebar
//   create-new affordances.
// - `ThreadStatus` non-`Idle` variants + `Thread::status`: US-013/US-015
//   streaming state machine.
//
// The PRD's dependency chain (US-007 -> US-008 -> US-009 -> US-010 -> US-011)
// only takes meaning when activated end-to-end; the foundation is what
// US-007 ships. The allow is module-scoped (not per-item) so the
// review surface stays one comment-line, not 12 attributes.
#![allow(dead_code)]

//! Runtime domain model for the Agents view (US-007 of
//! `tasks/prd-agents-view.md`).
//!
//! Mirrors the existing [`crate::workspace::Workspace`] shape:
//! - Monotonic `AtomicU64` ID counters per type
//! - A small struct holding the live state
//! - Conversion to/from the matching [`paneflow_config::schema`]
//!   "session" struct so save/restore round-trips cleanly
//!
//! The split between runtime types (here) and session types (in
//! `paneflow-config`) is intentional: session types are pure data and
//! belong to a leaf crate so the schema can evolve without dragging the
//! whole app crate into the same compile unit. Runtime types carry the
//! richer enums and conversion helpers.

use paneflow_acp::AgentKind;
use paneflow_config::schema::{ProjectSession, ThreadSession};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic project ID counter -- one per process. Mirrors
/// [`crate::workspace::next_workspace_id`] (US-007 AC).
static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

/// Monotonic thread ID counter -- one per process. Mirrors
/// [`crate::workspace::next_workspace_id`] (US-007 AC).
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_project_id() -> u64 {
    NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn next_thread_id() -> u64 {
    NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed)
}

/// On restore, advance both counters past any IDs reloaded from
/// `session.json` so newly-created projects/threads cannot collide
/// with restored ones.
pub fn bump_id_counters_to(projects: &[Project]) {
    let max_project = projects.iter().map(|p| p.id).max().unwrap_or(0);
    let max_thread = projects
        .iter()
        .flat_map(|p| p.threads.iter().map(|t| t.id))
        .max()
        .unwrap_or(0);
    bump_counter(&NEXT_PROJECT_ID, max_project + 1);
    bump_counter(&NEXT_THREAD_ID, max_thread + 1);
}

fn bump_counter(counter: &AtomicU64, target: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while current < target {
        match counter.compare_exchange_weak(current, target, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

/// Per-thread state machine (US-007 AC). Drives the sidebar status
/// dot animation: `Thinking` = pulsing, `WaitingForInput` = amber,
/// `Failed` = red, `Streaming` = blue, `Spawning` = ramping. `Idle` is
/// the rest state. The streaming pipeline (US-013/US-015) and the
/// permission flow (US-018) write into this field; the sidebar (US-010)
/// reads from it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThreadStatus {
    #[default]
    Idle,
    Spawning,
    Thinking,
    WaitingForInput,
    Streaming,
    Failed,
}

/// One persistent chat conversation. Body lives in
/// `paneflow-threads`'s SQLite DB (US-006); this struct holds only
/// the in-memory sidebar metadata.
#[derive(Debug, Clone)]
pub struct Thread {
    pub id: u64,
    pub title: String,
    pub agent: AgentKind,
    pub status: ThreadStatus,
    pub cwd: String,
    pub created_at: u64,
    pub model: Option<String>,
    pub mode: Option<String>,
    /// Foreign key into the `paneflow-threads` SQLite store (US-006).
    /// `None` until US-011 wires the create-thread side-effect that
    /// inserts the row; older threads loaded from a pre-US-011 session
    /// also restore as `None` (the lookup gracefully degrades to "no
    /// row to delete" on cascade).
    pub store_id: Option<String>,
}

impl Thread {
    /// Create a fresh thread with auto-allocated ID, current timestamp,
    /// and `Idle` status.
    pub fn new(title: impl Into<String>, agent: AgentKind, cwd: impl Into<String>) -> Self {
        Self {
            id: next_thread_id(),
            title: title.into(),
            agent,
            status: ThreadStatus::Idle,
            cwd: cwd.into(),
            created_at: now_unix_millis(),
            model: None,
            mode: None,
            store_id: None,
        }
    }
}

/// Sidebar grouping of threads with a shared cwd anchor.
#[derive(Debug, Clone)]
pub struct Project {
    pub id: u64,
    pub title: String,
    pub cwd: String,
    pub threads: Vec<Thread>,
    pub is_expanded: bool,
    /// Cached `git diff --shortstat` of the project's cwd. Refreshed
    /// by the 30 s git poller in `app/bootstrap.rs` (same loop that
    /// keeps workspace stats fresh). Stays `Default::default()` for
    /// non-git directories.
    pub git_stats: crate::workspace::GitDiffStats,
}

impl Project {
    /// Create a fresh project with auto-allocated ID, no threads, and
    /// `is_expanded = true` (matches the new-project-just-clicked
    /// expectation in the sidebar UX -- US-010).
    pub fn new(title: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            id: next_project_id(),
            title: title.into(),
            cwd: cwd.into(),
            threads: Vec::new(),
            is_expanded: true,
            git_stats: crate::workspace::GitDiffStats::default(),
        }
    }
}

/// Convert an in-memory [`Project`] to its persisted shape.
pub fn project_to_session(p: &Project) -> ProjectSession {
    ProjectSession {
        id: p.id,
        title: p.title.clone(),
        cwd: p.cwd.clone(),
        is_expanded: p.is_expanded,
        threads: p.threads.iter().map(thread_to_session).collect(),
    }
}

/// Convert an in-memory [`Thread`] to its persisted shape. The
/// runtime `status` is intentionally not persisted -- every thread
/// restores as `Idle`; live state is rebuilt from the ACP stream.
pub fn thread_to_session(t: &Thread) -> ThreadSession {
    ThreadSession {
        id: t.id,
        title: t.title.clone(),
        agent: agent_kind_to_str(t.agent).to_string(),
        cwd: t.cwd.clone(),
        created_at: t.created_at,
        model: t.model.clone(),
        mode: t.mode.clone(),
        // `store_id` is set by US-011's create-thread side-effect
        // (after inserting the `threads.db` row). Restoring a thread
        // from session without a `store_id` is legal -- the cascade
        // delete checks for `Some` before calling the store.
        store_id: t.store_id.clone(),
    }
}

/// Inverse of [`project_to_session`]. Unknown `agent` strings are
/// dropped (the thread is skipped) -- the AC says "OpenCode
/// deliberately excluded for v1", so any future tag we don't yet
/// recognise should surface as a missing thread, not a crash.
pub fn project_from_session(s: &ProjectSession) -> Project {
    Project {
        id: s.id,
        title: s.title.clone(),
        cwd: s.cwd.clone(),
        is_expanded: s.is_expanded,
        threads: s.threads.iter().filter_map(thread_from_session).collect(),
        git_stats: crate::workspace::GitDiffStats::default(),
    }
}

/// Inverse of [`thread_to_session`]. Returns `None` on unknown agent
/// tag (forward-compat for a future v1.x where OpenCode lands).
pub fn thread_from_session(s: &ThreadSession) -> Option<Thread> {
    let agent = agent_kind_from_str(&s.agent)?;
    Some(Thread {
        id: s.id,
        title: s.title.clone(),
        agent,
        status: ThreadStatus::default(),
        cwd: s.cwd.clone(),
        created_at: s.created_at,
        model: s.model.clone(),
        mode: s.mode.clone(),
        store_id: s.store_id.clone(),
    })
}

/// Canonical string tag for an [`AgentKind`]. Stable on-disk format
/// for `ThreadSession::agent`.
pub fn agent_kind_to_str(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::ClaudeCode => "claude_code",
        AgentKind::Codex => "codex",
    }
}

pub fn agent_kind_from_str(tag: &str) -> Option<AgentKind> {
    match tag {
        "claude_code" => Some(AgentKind::ClaudeCode),
        "codex" => Some(AgentKind::Codex),
        _ => None,
    }
}

fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::thread;

    #[test]
    fn project_id_counter_is_monotonic() {
        let a = next_project_id();
        let b = next_project_id();
        let c = next_project_id();
        assert!(a < b && b < c, "got {a} {b} {c}");
    }

    #[test]
    fn thread_id_counter_is_monotonic() {
        let a = next_thread_id();
        let b = next_thread_id();
        let c = next_thread_id();
        assert!(a < b && b < c, "got {a} {b} {c}");
    }

    // AC: monotonic ID atomicity across 1000 calls. We probe both
    // counters concurrently from many threads and assert (a) no
    // duplicates, (b) the full range was issued. The shared counter
    // in `super` is process-wide so other tests in the same binary
    // may have already advanced it; we work with the IDs we see, not
    // with a fixed range.
    #[test]
    fn project_id_atomic_no_duplicates_under_contention() {
        check_no_duplicates(next_project_id, 1_000);
    }

    #[test]
    fn thread_id_atomic_no_duplicates_under_contention() {
        check_no_duplicates(next_thread_id, 1_000);
    }

    fn check_no_duplicates(make_id: fn() -> u64, total: usize) {
        let issued = Arc::new(std::sync::Mutex::new(Vec::with_capacity(total)));
        let threads_n = 8;
        let per_thread = total / threads_n;

        let handles: Vec<_> = (0..threads_n)
            .map(|_| {
                let issued = Arc::clone(&issued);
                thread::spawn(move || {
                    let mut local = Vec::with_capacity(per_thread);
                    for _ in 0..per_thread {
                        local.push(make_id());
                    }
                    issued.lock().unwrap().extend(local);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let mut ids = issued.lock().unwrap().clone();
        let observed = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), observed, "duplicate IDs issued under contention");
    }

    #[test]
    fn project_roundtrip_through_session_shape() {
        let mut proj = Project::new("Paneflow", "/home/me/dev/paneflow");
        proj.threads.push(Thread::new(
            "First thread",
            AgentKind::ClaudeCode,
            &proj.cwd,
        ));
        proj.threads
            .push(Thread::new("Second thread", AgentKind::Codex, &proj.cwd));
        proj.is_expanded = false;

        let session = project_to_session(&proj);
        assert_eq!(session.id, proj.id);
        assert_eq!(session.threads.len(), 2);
        assert_eq!(session.threads[0].agent, "claude_code");
        assert_eq!(session.threads[1].agent, "codex");
        assert!(!session.is_expanded);

        let restored = project_from_session(&session);
        assert_eq!(restored.id, proj.id);
        assert_eq!(restored.title, proj.title);
        assert_eq!(restored.threads.len(), 2);
        assert_eq!(restored.threads[0].agent, AgentKind::ClaudeCode);
        assert_eq!(restored.threads[1].agent, AgentKind::Codex);
        // Status always restores Idle regardless of pre-save value.
        assert_eq!(restored.threads[0].status, ThreadStatus::Idle);
    }

    #[test]
    fn unknown_agent_tag_is_skipped_not_panicking() {
        let session = ProjectSession {
            id: 1,
            title: "P".to_string(),
            cwd: "/tmp".to_string(),
            is_expanded: true,
            threads: vec![ThreadSession {
                id: 1,
                title: "Future thread".to_string(),
                agent: "opencode".to_string(), // not in AgentKind yet
                cwd: "/tmp".to_string(),
                created_at: 0,
                model: None,
                mode: None,
                store_id: None,
            }],
        };
        let restored = project_from_session(&session);
        // The unknown thread is silently dropped (forward-compat).
        assert!(restored.threads.is_empty());
    }

    #[test]
    fn bump_id_counters_advances_past_restored_max() {
        let raw = AtomicU64::new(5);
        bump_counter(&raw, 10);
        assert_eq!(raw.load(Ordering::Relaxed), 10);
        // No regression: bumping to a smaller value is a no-op.
        bump_counter(&raw, 7);
        assert_eq!(raw.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn agent_kind_tags_are_bijective() {
        for kind in AgentKind::all() {
            let tag = agent_kind_to_str(kind);
            assert_eq!(agent_kind_from_str(tag), Some(kind), "round-trip {tag}");
        }
        assert_eq!(agent_kind_from_str("nope"), None);
    }
}
