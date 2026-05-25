//! The [`ThreadStore`] API: a thin, thread-safe wrapper around a single
//! SQLite connection.
//!
//! Concurrency model:
//! - One process-wide `Arc<Mutex<Connection>>` serialises every write.
//!   This is enough for Paneflow's single-process Agents view; the WAL
//!   mode is set so that future multi-process scenarios (debug tooling,
//!   import scripts) don't corrupt the file.
//! - `ThreadStore: Clone` clones the `Arc`, not the connection -- two
//!   clones share state. This is the pattern the UI uses to hand the
//!   store to background ACP tasks.
//!
//! Failure handling:
//! - The mutex is poison-tolerant ([`recover_lock`]). A poisoned mutex
//!   means a previous panic happened while holding the lock; the
//!   subsequent operation might see a partially-written transaction,
//!   but SQLite's own atomicity guarantees prevent file-level damage.
//! - On open, a `PRAGMA quick_check` runs. If it fails, the file is
//!   moved aside (`threads.db.corrupted-<unix-ts>`) and a fresh one is
//!   created -- US-006 AC #8.

use crate::blob::{compress_items, compress_messages, decompress_items, decompress_messages};
use crate::error::ThreadStoreError;
use crate::item::PersistedThreadItem;
use crate::message::Message;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Opaque, stable identifier for a persisted thread. Wraps a UUIDv4
/// rendered as a hyphenated lowercase string -- matches the SQL `TEXT
/// PRIMARY KEY` shape from US-006 AC #1.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(String);

impl ThreadId {
    /// Generate a new random ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wrap an already-existing string (e.g. loaded from session.json
    /// via US-007). No validation -- the SQL layer treats it as opaque.
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Sidebar view of a thread (no body). Returned by
/// [`ThreadStore::list_for_project`] and ordered `updated_at DESC` so
/// the most recently active conversations float to the top of the
/// list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadMetadata {
    pub id: ThreadId,
    pub summary: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Tells the caller whether [`ThreadStore::open_at`] returned a fresh
/// DB or one that had to be recovered from a corrupted file. The UI
/// uses this to render the "Threads database was corrupted, backed up
/// to disk" banner (US-006 AC #8).
#[derive(Clone, Debug)]
pub enum OpenOutcome {
    /// The DB opened cleanly.
    Opened,
    /// The pre-existing DB failed `PRAGMA quick_check`. It has been
    /// renamed to `backup` and a fresh DB sits at the original path.
    RecoveredFromCorruption { backup: PathBuf },
}

/// The thread store. Cheap to clone (clones share the underlying
/// connection via `Arc<Mutex<_>>`). Open it once per process and pass
/// clones to any task that needs persistence.
#[derive(Clone)]
pub struct ThreadStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl ThreadStore {
    /// Open the store at the canonical path
    /// (`~/.local/share/paneflow/threads/threads.db` and platform
    /// equivalents). Returns `Err(ThreadStoreError::NoDataDir)` if the
    /// platform helper cannot resolve a local data directory.
    pub fn open_default() -> Result<(Self, OpenOutcome), ThreadStoreError> {
        let path = crate::paths::default_db_path().ok_or(ThreadStoreError::NoDataDir)?;
        Self::open_at(path)
    }

    /// Open the store at a caller-chosen path. Creates the parent
    /// directory if missing. Honours the corruption-recovery flow.
    pub fn open_at(path: impl Into<PathBuf>) -> Result<(Self, OpenOutcome), ThreadStoreError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let (conn, outcome) = match open_and_init(&path) {
            Ok(conn) => (conn, OpenOutcome::Opened),
            Err(open_err) => {
                // The file is present but unusable. Try to move it
                // aside and start fresh so the user is not locked out.
                if !path.exists() {
                    // Not corruption, real error -- bubble it up.
                    return Err(open_err);
                }
                let backup = corrupted_backup_path(&path);
                tracing::warn!(
                    "paneflow-threads: {} failed integrity check ({open_err}); \
                     renaming to {} and starting fresh",
                    path.display(),
                    backup.display()
                );
                std::fs::rename(&path, &backup)?;
                let conn = open_and_init(&path)?;
                (conn, OpenOutcome::RecoveredFromCorruption { backup })
            }
        };

        Ok((
            Self {
                db_path: path,
                conn: Arc::new(Mutex::new(conn)),
            },
            outcome,
        ))
    }

    /// Path to the underlying SQLite file. Stable for the lifetime of
    /// the store. Mostly useful in tests and diagnostics.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Insert a brand-new thread row with an empty message blob. The
    /// returned [`ThreadId`] is the row's primary key. `summary`
    /// defaults to an empty string -- the UI should set a real title
    /// via [`Self::set_summary`] once it has one (typically derived
    /// from the first user prompt).
    pub fn create_thread(
        &self,
        project_id: Option<&str>,
        agent_id: &str,
    ) -> Result<ThreadId, ThreadStoreError> {
        let id = ThreadId::new();
        let now = now_iso();
        let empty_blob = compress_messages(&[])?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO threads \
               (id, summary, agent_id, project_id, created_at, updated_at, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id.as_str(), "", agent_id, project_id, now, now, empty_blob],
        )?;
        Ok(id)
    }

    /// Append one message to the thread. The blob is decompressed
    /// into the full timeline (so tool calls + reasoning cards are
    /// preserved), the message is pushed, the result is recompressed,
    /// and the row is written -- all inside a single SQLite
    /// transaction held under the connection mutex, so two concurrent
    /// appends to the same `ThreadId` serialise correctly without
    /// lost updates. Prefer [`Self::save_items`] for new code; this
    /// helper stays as a thin convenience for legacy callers.
    pub fn append_message(&self, id: &ThreadId, message: &Message) -> Result<(), ThreadStoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let blob: Vec<u8> = tx
            .query_row(
                "SELECT data FROM threads WHERE id = ?1",
                params![id.as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => ThreadStoreError::NotFound(id.clone()),
                other => ThreadStoreError::Sqlite(other),
            })?;
        let mut items = decompress_items(&blob)?;
        items.push(PersistedThreadItem::Message(message.clone()));
        let new_blob = compress_items(&items)?;
        let now = now_iso();
        let changed = tx.execute(
            "UPDATE threads SET data = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_blob, now, id.as_str()],
        )?;
        if changed == 0 {
            return Err(ThreadStoreError::NotFound(id.clone()));
        }
        tx.commit()?;
        Ok(())
    }

    /// Replace the entire message vector. Used by the fork flow
    /// (US-020) and by atomic compaction. `updated_at` is bumped to
    /// "now". Returns `NotFound` if the row does not exist.
    pub fn replace_messages(
        &self,
        id: &ThreadId,
        messages: &[Message],
    ) -> Result<(), ThreadStoreError> {
        let blob = compress_messages(messages)?;
        let now = now_iso();
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE threads SET data = ?1, updated_at = ?2 WHERE id = ?3",
            params![blob, now, id.as_str()],
        )?;
        if changed == 0 {
            return Err(ThreadStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Update only the human-readable summary (sidebar title). Does
    /// not touch `data`. Bumps `updated_at` so the sidebar reorders.
    pub fn set_summary(&self, id: &ThreadId, summary: &str) -> Result<(), ThreadStoreError> {
        let now = now_iso();
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE threads SET summary = ?1, updated_at = ?2 WHERE id = ?3",
            params![summary, now, id.as_str()],
        )?;
        if changed == 0 {
            return Err(ThreadStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Load the full message history (legacy convenience). Filters the
    /// timeline down to `Message` entries; callers that need tool
    /// calls or reasoning cards should use [`Self::load_items`]
    /// instead.
    pub fn load_thread(&self, id: &ThreadId) -> Result<Vec<Message>, ThreadStoreError> {
        let conn = self.lock();
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT data FROM threads WHERE id = ?1",
                params![id.as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => ThreadStoreError::NotFound(id.clone()),
                other => ThreadStoreError::Sqlite(other),
            })?;
        drop(conn); // release the mutex before the (potentially slow) zstd decode
        Ok(decompress_messages(&blob)?)
    }

    /// Load the full persisted timeline (messages + standalone tool
    /// calls + reasoning cards). Handles the legacy `messages`-only
    /// blob shape transparently via [`crate::blob::decompress_items`].
    /// `NotFound` if the row is gone.
    pub fn load_items(&self, id: &ThreadId) -> Result<Vec<PersistedThreadItem>, ThreadStoreError> {
        let conn = self.lock();
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT data FROM threads WHERE id = ?1",
                params![id.as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => ThreadStoreError::NotFound(id.clone()),
                other => ThreadStoreError::Sqlite(other),
            })?;
        drop(conn);
        Ok(decompress_items(&blob)?)
    }

    /// Replace the entire persisted timeline with `items`. Mirrors
    /// [`Self::replace_messages`] but covers tool calls and reasoning
    /// cards as well. Used by the ThreadView's event-driven snapshot
    /// trigger so every mutation re-writes the canonical blob. Bumps
    /// `updated_at`. Returns `NotFound` if the row does not exist.
    pub fn save_items(
        &self,
        id: &ThreadId,
        items: &[PersistedThreadItem],
    ) -> Result<(), ThreadStoreError> {
        let blob = compress_items(items)?;
        let now = now_iso();
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE threads SET data = ?1, updated_at = ?2 WHERE id = ?3",
            params![blob, now, id.as_str()],
        )?;
        if changed == 0 {
            return Err(ThreadStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// List every thread for a project (or unassigned threads when
    /// `project_id == None`), most recently updated first.
    pub fn list_for_project(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<ThreadMetadata>, ThreadStoreError> {
        let conn = self.lock();
        // Two query shapes because SQLite treats `= NULL` as never-true
        // -- we have to use `IS NULL` for the "no project" case.
        let mut stmt = match project_id {
            Some(_) => conn.prepare(
                "SELECT id, summary, agent_id, project_id, created_at, updated_at \
                   FROM threads \
                  WHERE project_id = ?1 \
                  ORDER BY updated_at DESC",
            )?,
            None => conn.prepare(
                "SELECT id, summary, agent_id, project_id, created_at, updated_at \
                   FROM threads \
                  WHERE project_id IS NULL \
                  ORDER BY updated_at DESC",
            )?,
        };
        let rows = match project_id {
            Some(p) => stmt.query_map([p], row_to_metadata)?.collect::<Vec<_>>(),
            None => stmt.query_map([], row_to_metadata)?.collect::<Vec<_>>(),
        };
        rows.into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Hard-delete the row. Idempotent: deleting a missing thread is
    /// not an error (so the UI can call this from a confirmation
    /// dialog without worrying about double-clicks).
    pub fn delete_thread(&self, id: &ThreadId) -> Result<(), ThreadStoreError> {
        let conn = self.lock();
        conn.execute("DELETE FROM threads WHERE id = ?1", params![id.as_str()])?;
        Ok(())
    }

    /// US-102: load a thread's pending composer draft, if any. Returns
    /// `Ok(None)` when no row exists OR when the payload fails to
    /// deserialise (malformed JSON from a crash, hand-edited DB, ...)
    /// — the latter case logs a `warn!` line and returns `None` so the
    /// composer opens empty rather than panicking. The payload is a
    /// JSON-encoded `Vec<ContentBlock>`; preserving the structured
    /// form keeps mentions / attachments alive across the round-trip.
    pub fn read_draft(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Option<Vec<crate::ContentBlock>>, ThreadStoreError> {
        let conn = self.lock();
        let payload: Option<Vec<u8>> = conn
            .query_row(
                "SELECT payload FROM drafts WHERE thread_id = ?1",
                params![thread_id.as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map(Some)
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(ThreadStoreError::Sqlite(other)),
            })?;
        drop(conn);
        let Some(bytes) = payload else {
            return Ok(None);
        };
        match serde_json::from_slice::<Vec<crate::ContentBlock>>(&bytes) {
            Ok(blocks) => Ok(Some(blocks)),
            Err(err) => {
                tracing::warn!(
                    target: "paneflow_threads::drafts",
                    thread_id = thread_id.as_str(),
                    "draft payload was unreadable, opening composer empty: {err}",
                );
                Ok(None)
            }
        }
    }

    /// US-102: upsert the in-progress composer draft for `thread_id`.
    /// Replaces the previous payload wholesale (drafts are last-writer-
    /// wins by design). Bumps `updated_at` for diagnostics.
    pub fn write_draft(
        &self,
        thread_id: &ThreadId,
        blocks: &[crate::ContentBlock],
    ) -> Result<(), ThreadStoreError> {
        let payload = serde_json::to_vec(blocks).map_err(ThreadStoreError::Json)?;
        let now = now_iso();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO drafts (thread_id, payload, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(thread_id) DO UPDATE SET payload = excluded.payload, \
             updated_at = excluded.updated_at",
            params![thread_id.as_str(), payload, now],
        )?;
        Ok(())
    }

    /// US-102: remove the draft row for `thread_id`. Idempotent —
    /// deleting a missing draft is not an error so the composer can
    /// call this from the send path without worrying about whether
    /// the draft was actually persisted yet.
    pub fn delete_draft(&self, thread_id: &ThreadId) -> Result<(), ThreadStoreError> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM drafts WHERE thread_id = ?1",
            params![thread_id.as_str()],
        )?;
        Ok(())
    }

    /// Return all threads grouped by `project_id`. Helper for app
    /// startup (rebuilding the sidebar in one query, ordered).
    pub fn list_all(&self) -> Result<Vec<ThreadMetadata>, ThreadStoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, summary, agent_id, project_id, created_at, updated_at \
               FROM threads \
              ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_metadata)?.collect::<Vec<_>>();
        rows.into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        recover_lock(self.conn.lock())
    }
}

fn row_to_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadMetadata> {
    Ok(ThreadMetadata {
        id: ThreadId::from_string(row.get::<_, String>(0)?),
        summary: row.get(1)?,
        agent_id: row.get(2)?,
        project_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn open_and_init(path: &Path) -> Result<Connection, ThreadStoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    // PRAGMA quick_check returns "ok" on a healthy DB. Any other value
    // (or an error) means the file is corrupt.
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(ThreadStoreError::Sqlite)?;
    if check != "ok" {
        return Err(ThreadStoreError::Sqlite(
            rusqlite::Error::ExecuteReturnedResults,
        ));
    }

    // PRAGMAs that must run on every open (WAL persists, the others
    // are connection-scoped). `wal_autocheckpoint = 1000` caps the WAL
    // at ~1000 pages so it cannot grow unbounded under heavy write
    // (PRD Risk #7). `auto_vacuum = INCREMENTAL` lets the file shrink
    // when threads are deleted without forcing a full VACUUM.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 500_i64)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "wal_autocheckpoint", 1000_i64)?;
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;

    // Schema. `CREATE TABLE IF NOT EXISTS` makes the open path
    // idempotent for both fresh and pre-existing DBs. The index lives
    // on `(project_id, updated_at DESC)` to match the most frequent
    // sidebar query.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS threads (
             id          TEXT PRIMARY KEY,
             summary     TEXT NOT NULL,
             agent_id    TEXT NOT NULL,
             project_id  TEXT,
             created_at  TEXT NOT NULL,
             updated_at  TEXT NOT NULL,
             data        BLOB NOT NULL
         );
         CREATE INDEX IF NOT EXISTS threads_project_updated_idx
             ON threads (project_id, updated_at DESC);
         CREATE TABLE IF NOT EXISTS drafts (
             thread_id   TEXT PRIMARY KEY,
             payload     BLOB NOT NULL,
             updated_at  TEXT NOT NULL
         );",
    )?;

    // The `archived` column was added to this table by an earlier
    // migration and removed once we dropped the archive feature. Old
    // DBs may still carry the column; SELECTs name explicit columns,
    // so the dead column is silently ignored. No schema downgrade is
    // attempted to keep the open path branch-free.

    Ok(conn)
}

fn corrupted_backup_path(path: &Path) -> PathBuf {
    let unix_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "threads.db".to_string());
    path.with_file_name(format!("{file_name}.corrupted-{unix_ts}"))
}

fn now_iso() -> String {
    // RFC 3339 in UTC. Without a chrono dep we build it from
    // SystemTime + a tiny formatter -- this avoids dragging chrono in
    // (~80 KB) for one timestamp string. Format: "2026-05-22T14:30:00.123Z".
    let secs_total = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i128)
        .unwrap_or(0);
    let secs = (secs_total / 1000) as i64;
    let ms = (secs_total % 1000).unsigned_abs() as u32;
    format_iso8601(secs, ms)
}

fn format_iso8601(unix_secs: i64, ms: u32) -> String {
    // Civil-from-days algorithm (Howard Hinnant). Avoids the chrono
    // dep while still producing valid RFC 3339 timestamps for any
    // year > 1.
    let (z, sod) = {
        let d = unix_secs.div_euclid(86_400);
        let s = unix_secs.rem_euclid(86_400);
        (d, s)
    };
    let (year, month, day) = civil_from_days(z);
    let hour = (sod / 3600) as u32;
    let minute = ((sod % 3600) / 60) as u32;
    let second = (sod % 60) as u32;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z",)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

fn recover_lock<'a, T>(
    result: Result<MutexGuard<'a, T>, std::sync::PoisonError<MutexGuard<'a, T>>>,
) -> MutexGuard<'a, T> {
    match result {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_8601_known_value() {
        // 2024-01-01 00:00:00 UTC == 1_704_067_200 unix seconds
        // (cross-check with `date -d @1704067200 -u`).
        let s = format_iso8601(1_704_067_200, 0);
        assert_eq!(s, "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn iso_8601_with_ms() {
        // 2026-05-22 00:00:00 UTC == 1_779_408_000 unix seconds.
        let s = format_iso8601(1_779_408_000, 123);
        assert_eq!(s, "2026-05-22T00:00:00.123Z");
    }

    #[test]
    fn iso_8601_epoch_boundary() {
        // Sanity check for the Hinnant algorithm at z=0.
        assert_eq!(format_iso8601(0, 0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn now_iso_parses_round_shape() {
        // We can't compare against a fixed value (it's "now"); we just
        // verify the shape parses and ends with the millisecond+Z.
        let s = now_iso();
        assert_eq!(s.len(), 24, "got {s}");
        assert!(s.ends_with('Z'));
        assert!(s.contains('T'));
    }

    #[test]
    fn corrupted_backup_path_has_ts_suffix() {
        let p = corrupted_backup_path(Path::new("/tmp/threads.db"));
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("threads.db.corrupted-"), "got {name}");
        let suffix = name.trim_start_matches("threads.db.corrupted-");
        assert!(suffix.chars().all(|c| c.is_ascii_digit()), "got {suffix}");
    }
}
