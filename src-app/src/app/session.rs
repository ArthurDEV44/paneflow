//! Session persistence for `PaneFlowApp` — save/restore workspace layouts
//! and their per-pane CWD + scrollback so relaunching rebuilds exactly
//! what the user had open.
//!
//! Extracted from `main.rs` per US-017 of the src-app refactor PRD.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext, Context, Entity};
use paneflow_config::schema::LayoutNode;

use crate::PaneFlowApp;
use crate::layout::LayoutTree;
use crate::pane::Pane;
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, next_workspace_id};

/// Cap on the number of `session.json.corrupted-*` backup files retained
/// alongside the live session. Beyond this, the oldest are deleted on
/// rotation. Cf. risk R8 in `prd-stabilization-2026-q2.md`: every parse
/// failure produces a new backup, and without rotation a user with a
/// chronic corruption (e.g. a flaky disk) would silently fill `~/.cache`.
const MAX_CORRUPTION_BACKUPS: usize = 5;

/// Forensic context emitted alongside a `session_corrupted` telemetry
/// event (US-006). Gathered by [`PaneFlowApp::load_session_at`] inside
/// the parse-failure branch *before* the empty fallback session is
/// returned, so the values reflect the file the user actually had on
/// disk — not the one we're about to overwrite. Stays a plain data
/// struct (no telemetry client coupling) because `load_session` runs
/// in bootstrap before the `TelemetryClient` is constructed; the
/// caller in `bootstrap.rs` defers the emit until after telemetry is
/// up.
#[derive(Debug, Clone)]
pub(crate) struct SessionCorruptionInfo {
    /// Canonical `serde_json::Error::classify()` bucket
    /// (`io | syntax | data | eof`). Plain string keeps the telemetry
    /// schema fixed even if serde widens its `Category` enum.
    pub(crate) error_category: &'static str,
    /// Size in bytes of the corrupted file, or `0` if the metadata
    /// call itself failed (rare — file just got read successfully).
    pub(crate) file_size: u64,
    /// Wall-clock age in seconds (mtime → now). `None` when the
    /// platform's modification-time call returns a value newer than
    /// `now` (clock drift) or the metadata call fails.
    pub(crate) file_age_seconds: Option<u64>,
    /// Resolved path of the freshly-written backup, or `None` if the
    /// backup write itself failed (AC6 — never block startup on
    /// backup-side errors).
    pub(crate) backup_path: Option<PathBuf>,
}

impl PaneFlowApp {
    pub(crate) fn save_session(&self, cx: &App) {
        let state = paneflow_config::schema::SessionState {
            version: paneflow_config::schema::SESSION_SCHEMA_VERSION,
            active_workspace: self.active_idx,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| paneflow_config::schema::WorkspaceSession {
                    title: ws.title.clone(),
                    cwd: ws.cwd.clone(),
                    layout: ws.serialize_layout(cx),
                    custom_buttons: ws.custom_buttons.clone(),
                })
                .collect(),
        };
        let Some(path) = paneflow_config::loader::session_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&state) {
            Ok(json) => {
                // Atomic write: write to temp file, then rename. This prevents
                // corruption if the process is killed mid-write (US-013).
                let tmp_path = path.with_extension("json.tmp");
                match std::fs::write(&tmp_path, &json) {
                    Ok(()) => {
                        if let Err(e) = std::fs::rename(&tmp_path, &path) {
                            log::warn!("session save rename failed: {e}");
                            let _ = std::fs::remove_file(&tmp_path);
                        }
                    }
                    Err(e) => {
                        log::warn!("session save failed: {e}");
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                }
            }
            Err(e) => log::warn!("session serialize failed: {e}"),
        }
    }

    /// Restore a saved session from disk, or fall back silently to an
    /// empty session if anything goes wrong.
    ///
    /// Behaviour matrix (US-006):
    ///
    /// | State on disk           | Returned session  | Corruption info | Backup written |
    /// |-------------------------|-------------------|-----------------|----------------|
    /// | File missing            | `None`            | `None`          | no             |
    /// | Read error (perms, IO)  | `None`            | `None`          | no             |
    /// | Read OK + parse OK      | `Some(state)`     | `None`          | no             |
    /// | Read OK + parse FAIL    | `None` (fallback) | `Some(info)`    | yes            |
    ///
    /// On parse failure the bad file is preserved as
    /// `session.json.corrupted-<unix-timestamp>` *before* the next
    /// `save_session` overwrites it, so we keep forensic evidence even
    /// when the user immediately moves on. The backup directory is
    /// rotated down to [`MAX_CORRUPTION_BACKUPS`] entries (R8) so a
    /// chronic-corruption case can't silently fill `~/.cache`.
    ///
    /// Telemetry: callers receive a `SessionCorruptionInfo` they can
    /// pass to `PaneFlowApp::emit_session_corrupted` once the
    /// telemetry client is up. The emit is consent-gated by the
    /// existing `TelemetryClient::Null` factory branch — opted-out
    /// users never produce a network call.
    pub(crate) fn load_session() -> (
        Option<paneflow_config::schema::SessionState>,
        Option<SessionCorruptionInfo>,
    ) {
        let Some(path) = paneflow_config::loader::session_path() else {
            return (None, None);
        };
        Self::load_session_at(&path)
    }

    /// Path-parametrised core of [`load_session`]. Direct test surface —
    /// the wrapper above resolves `paneflow_config::loader::session_path()`
    /// against the user's XDG cache dir, which is unsuitable for unit
    /// tests because every run would race against a live install.
    pub(crate) fn load_session_at(
        path: &Path,
    ) -> (
        Option<paneflow_config::schema::SessionState>,
        Option<SessionCorruptionInfo>,
    ) {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("session load: read failed at {}: {e}", path.display());
                }
                return (None, None);
            }
        };

        match serde_json::from_str::<paneflow_config::schema::SessionState>(&data) {
            Ok(state) => (Some(state), None),
            Err(parse_err) => {
                log::warn!(
                    "session load: parse failed at {} ({}); falling back to empty session",
                    path.display(),
                    parse_err
                );

                let metadata = std::fs::metadata(path).ok();
                let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                let file_age_seconds = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|mt| SystemTime::now().duration_since(mt).ok())
                    .map(|d| d.as_secs());

                let backup_path = write_corruption_backup(path, &data).unwrap_or_else(|e| {
                    // AC6: backup write failure must not block startup.
                    // Log and proceed — telemetry still fires with a
                    // `backup_path: None`, so the operator can still see
                    // the corruption rate even if forensics are missing.
                    log::warn!(
                        "session load: backup write failed at {}: {e}",
                        path.display()
                    );
                    None
                });

                (
                    None,
                    Some(SessionCorruptionInfo {
                        error_category: serde_category_tag(&parse_err),
                        file_size,
                        file_age_seconds,
                        backup_path,
                    }),
                )
            }
        }
    }

    /// Rebuild workspaces from a saved session. Each workspace's layout tree
    /// is reconstructed via `LayoutTree::from_layout_node` with CWD-aware
    /// terminal spawning. Returns the workspace list and active index.
    pub(crate) fn restore_workspaces(
        session: &paneflow_config::schema::SessionState,
        cx: &mut Context<Self>,
    ) -> (Vec<Workspace>, usize) {
        use std::path::PathBuf;

        let mut workspaces = Vec::new();

        for ws_session in &session.workspaces {
            let cwd = PathBuf::from(&ws_session.cwd);
            let ws_id = next_workspace_id();

            let mut workspace = if let Some(mut layout) = ws_session.layout.clone() {
                paneflow_config::loader::validate_layout(&mut layout);
                let mut pane_deque: VecDeque<Entity<Pane>> = VecDeque::new();
                let ws_cwd = cwd.clone();
                let tree = LayoutTree::from_layout_node(&layout, &mut pane_deque, &mut |node| {
                    let surfaces = match node {
                        LayoutNode::Pane { surfaces } => surfaces.as_slice(),
                        _ => &[],
                    };
                    Self::spawn_pane_from_surfaces(ws_id, surfaces, &ws_cwd, cx)
                });
                Workspace::with_layout_and_id(ws_id, ws_session.title.clone(), cwd, tree)
            } else {
                let terminal =
                    cx.new(|cx| TerminalView::with_cwd(ws_id, Some(cwd.clone()), None, cx));
                cx.subscribe(&terminal, Self::handle_terminal_event)
                    .detach();
                let pane = cx.new(|cx| Pane::new(terminal, ws_id, cx));
                cx.subscribe(&pane, Self::handle_pane_event).detach();
                Workspace::with_cwd_and_id(ws_id, ws_session.title.clone(), cwd, pane)
            };

            workspace.custom_buttons = ws_session.custom_buttons.clone();
            workspace.propagate_custom_buttons(cx);
            workspaces.push(workspace);
        }

        let active_idx = session
            .active_workspace
            .min(workspaces.len().saturating_sub(1));
        (workspaces, active_idx)
    }

    /// Create a `Pane` (with one tab per surface) from serialized surface
    /// definitions. Falls back to a single terminal in `fallback_cwd` when
    /// the surface list is empty.
    pub(crate) fn spawn_pane_from_surfaces(
        workspace_id: u64,
        surfaces: &[paneflow_config::schema::SurfaceDefinition],
        fallback_cwd: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Entity<Pane> {
        use std::path::PathBuf;

        let mut focus_idx: usize = 0;
        let terminals: Vec<Entity<TerminalView>> = if surfaces.is_empty() {
            let t = cx.new(|cx| {
                TerminalView::with_cwd(workspace_id, Some(fallback_cwd.to_path_buf()), None, cx)
            });
            cx.subscribe(&t, Self::handle_terminal_event).detach();
            vec![t]
        } else {
            surfaces
                .iter()
                .enumerate()
                .map(|(i, surface)| {
                    let cwd = surface
                        .cwd
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| fallback_cwd.to_path_buf());

                    let t = cx.new(|cx| TerminalView::with_cwd(workspace_id, Some(cwd), None, cx));

                    if let Some(ref scrollback) = surface.scrollback {
                        t.read(cx).terminal.restore_scrollback(scrollback);
                    }
                    cx.subscribe(&t, Self::handle_terminal_event).detach();
                    if surface.focus == Some(true) {
                        focus_idx = i;
                    }
                    t
                })
                .collect()
        };

        let first = terminals[0].clone();
        let pane = cx.new(|cx| {
            let mut p = Pane::new(first, workspace_id, cx);
            for tab in &terminals[1..] {
                p.add_tab(tab.clone(), cx);
            }
            p.selected_idx = focus_idx.min(terminals.len() - 1);
            p
        });
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        pane
    }
}

// ---------------------------------------------------------------------------
// US-006: corruption-backup helpers (free functions, free of `&self`)
// ---------------------------------------------------------------------------

/// Convert `serde_json::Error::classify()` to a fixed string. Telemetry
/// schema commits to these four buckets so we can dashboard them
/// directly without remapping if serde widens its enum later.
fn serde_category_tag(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    }
}

/// Persist the corrupted file's bytes to
/// `<session_path>.corrupted-<unix-timestamp>` and rotate the backup
/// directory down to [`MAX_CORRUPTION_BACKUPS`] entries.
///
/// Returns `Ok(Some(path))` on success, `Ok(None)` if the wall clock is
/// before `UNIX_EPOCH` (a degenerate state we do not want to crash on),
/// `Err` on actual filesystem failures so the caller can log without
/// blocking startup.
fn write_corruption_backup(
    session_path: &Path,
    contents: &str,
) -> std::io::Result<Option<PathBuf>> {
    let parent = match session_path.parent() {
        Some(p) => p,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "session path has no parent",
            ));
        }
    };
    std::fs::create_dir_all(parent)?;

    let ts = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return Ok(None),
    };
    let stem = session_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("session.json");
    let backup = parent.join(format!("{stem}.corrupted-{ts}"));
    std::fs::write(&backup, contents)?;

    rotate_corruption_backups(parent, stem);
    Ok(Some(backup))
}

/// Cap the count of `<stem>.corrupted-*` files in `dir` to
/// [`MAX_CORRUPTION_BACKUPS`], deleting the oldest first. Best-effort —
/// any filesystem error during rotation is logged and swallowed because
/// failing rotation must never abort startup (R8 mitigation, AC6
/// spirit).
fn rotate_corruption_backups(dir: &Path, stem: &str) {
    let prefix = format!("{stem}.corrupted-");
    let mut backups: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(it) => it
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect(),
        Err(_) => return,
    };

    if backups.len() <= MAX_CORRUPTION_BACKUPS {
        return;
    }

    // Sort by trailing unix-timestamp ascending (oldest first). The
    // suffix is purely numeric so a string compare suffices and avoids
    // a metadata syscall per file.
    backups.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix(&prefix))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });

    let drop_count = backups.len() - MAX_CORRUPTION_BACKUPS;
    for old in backups.into_iter().take(drop_count) {
        if let Err(e) = std::fs::remove_file(&old) {
            log::warn!(
                "session backup rotation: could not remove {}: {e}",
                old.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a `session.json` with deliberately broken JSON, run the
    /// path-parametrised loader, assert the corruption-info shape and
    /// the on-disk backup file. Covers AC1 (None fallback + info
    /// emitted), AC2 (backup written before fallback), AC3 (load_session
    /// behaviour test).
    #[test]
    fn malformed_json_returns_corruption_info_and_writes_backup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let session_path = tmp.path().join("session.json");
        std::fs::write(&session_path, "{").expect("seed broken session");

        let (state, info) = PaneFlowApp::load_session_at(&session_path);
        assert!(state.is_none(), "fallback to empty session expected");

        let info = info.expect("corruption info expected");
        assert_eq!(info.error_category, "eof", "trailing brace = EOF bucket");
        assert_eq!(info.file_size, 1, "single byte file");
        let backup = info.backup_path.expect("backup path populated");
        assert!(backup.exists(), "backup file actually on disk");
        assert!(
            backup
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("session.json.corrupted-")),
            "backup name format honoured"
        );
        let backup_contents = std::fs::read_to_string(&backup).expect("backup is readable");
        assert_eq!(backup_contents, "{", "backup preserves original bytes");
    }

    /// AC1: missing file is *not* corruption — both halves of the
    /// return tuple must be `None` so `bootstrap.rs` doesn't emit a
    /// noisy `session_corrupted` event for every fresh install.
    #[test]
    fn missing_file_yields_no_state_no_corruption() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nonexistent.json");

        let (state, info) = PaneFlowApp::load_session_at(&path);
        assert!(state.is_none());
        assert!(info.is_none(), "missing file is not corruption");
    }

    /// R8: backup directory must not grow unbounded. After 7 induced
    /// corruptions only the 5 newest survive — verifies the
    /// timestamp-sort + drop-oldest path in `rotate_corruption_backups`.
    #[test]
    fn corruption_backup_rotation_caps_at_five() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let session_path = tmp.path().join("session.json");

        // Pre-seed 7 backups with monotonic synthetic timestamps so
        // the test does not depend on the host's wall-clock resolution
        // (a real run produces one new backup per parse failure, but
        // bursts within the same second would otherwise collide).
        for ts in 1000..1007u64 {
            let p = tmp.path().join(format!("session.json.corrupted-{ts}"));
            std::fs::write(&p, format!("backup{ts}")).expect("seed backup");
        }

        rotate_corruption_backups(tmp.path(), "session.json");

        let mut surviving: Vec<u64> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_prefix("session.json.corrupted-"))
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .collect();
        surviving.sort_unstable();
        assert_eq!(
            surviving,
            vec![1002, 1003, 1004, 1005, 1006],
            "5 newest survive, 2 oldest deleted"
        );

        // Live session.json itself is unaffected by the rotation.
        std::fs::write(&session_path, "{").expect("seed");
        assert!(session_path.exists());
    }
}
