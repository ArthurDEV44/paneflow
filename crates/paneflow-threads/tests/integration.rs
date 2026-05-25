//! End-to-end integration tests for the `ThreadStore` API. Cover the
//! ACs spelled out in US-006 of `tasks/prd-agents-view.md`:
//! - create + load round-trip
//! - large message (~1 MB plaintext) compresses to <50% of raw bytes
//! - corrupted DB at startup is renamed and a fresh DB is opened
//! - list ordering: `updated_at DESC`
//! - concurrent writes from two threads both succeed without panic or
//!   data loss

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_in_result
)]

use agent_client_protocol::schema::{ContentBlock, TextContent};
use paneflow_threads::{Message, MessageRole, OpenOutcome, ThreadStore};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn open(tmp: &TempDir) -> (ThreadStore, OpenOutcome) {
    let path = tmp.path().join("threads.db");
    ThreadStore::open_at(path).expect("open threads.db")
}

#[test]
fn create_load_round_trip() {
    let tmp = TempDir::new().unwrap();
    let (store, outcome) = open(&tmp);
    assert!(matches!(outcome, OpenOutcome::Opened));

    let id = store
        .create_thread(Some("proj-1"), "claude-code")
        .expect("create_thread");

    let loaded = store.load_thread(&id).unwrap();
    assert!(
        loaded.is_empty(),
        "fresh thread has no messages, got {}",
        loaded.len()
    );

    store
        .append_message(&id, &Message::user_text("hello"))
        .unwrap();
    store
        .append_message(&id, &Message::assistant_text("hi there"))
        .unwrap();

    let loaded = store.load_thread(&id).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].role, MessageRole::User);
    assert_eq!(loaded[1].role, MessageRole::Assistant);
    match &loaded[0].content[0] {
        ContentBlock::Text(TextContent { text, .. }) => assert_eq!(text, "hello"),
        other => panic!("expected text content, got {other:?}"),
    }
}

#[test]
fn large_message_compresses_significantly() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let id = store.create_thread(None, "claude-code").unwrap();

    // ~1 MB of repetitive prose. zstd level 3 should crush this well
    // under 50% of raw bytes.
    let phrase = "The quick brown fox jumps over the lazy dog. ";
    let big = phrase.repeat(25_000);
    let raw_len = big.len();
    assert!(raw_len >= 1_000_000, "test fixture should be >=1 MB");

    store
        .append_message(&id, &Message::assistant_text(big))
        .unwrap();

    // Read the stored BLOB size out of the file directly.
    let conn = rusqlite::Connection::open(store.db_path()).unwrap();
    let blob_len: i64 = conn
        .query_row(
            "SELECT length(data) FROM threads WHERE id = ?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();

    let ratio = blob_len as f64 / raw_len as f64;
    assert!(
        ratio < 0.5,
        "expected compressed blob <50% of raw ({raw_len} B), got {blob_len} B (ratio {ratio:.3})"
    );

    // And confirm the data round-trips after compression.
    let loaded = store.load_thread(&id).unwrap();
    assert_eq!(loaded.len(), 1);
    match &loaded[0].content[0] {
        ContentBlock::Text(TextContent { text, .. }) => {
            assert_eq!(text.len(), raw_len);
        }
        other => panic!("expected text content, got {other:?}"),
    }
}

#[test]
fn corrupted_db_is_renamed_and_reopens_fresh() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("threads.db");

    // Write a wholly bogus payload as if the file had been mid-write
    // when the process crashed.
    std::fs::write(&path, b"not a sqlite database, this should fail").unwrap();

    let (store, outcome) = ThreadStore::open_at(&path).expect("must recover");
    let backup = match outcome {
        OpenOutcome::RecoveredFromCorruption { backup } => backup,
        OpenOutcome::Opened => panic!("expected RecoveredFromCorruption outcome"),
    };

    assert!(backup.exists(), "backup file must exist on disk");
    assert!(path.exists(), "fresh DB must be written to original path");
    let backup_name = backup.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        backup_name.starts_with("threads.db.corrupted-"),
        "got {backup_name}"
    );

    // The fresh store is functional: insert + read works.
    let id = store.create_thread(None, "claude-code").unwrap();
    store
        .append_message(&id, &Message::user_text("post-recovery"))
        .unwrap();
    let loaded = store.load_thread(&id).unwrap();
    assert_eq!(loaded.len(), 1);
}

#[test]
fn list_for_project_orders_by_updated_at_desc() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);

    let a = store.create_thread(Some("proj-A"), "claude-code").unwrap();
    // Sleep 1.1s between creates so the ISO timestamps differ by at
    // least one whole second (our `now_iso` is millisecond-resolution
    // but the table column is text and SQLite uses lexicographic order
    // on the string -- so millisecond drift is enough, but we use
    // 1.1s for slack on slow CI).
    thread::sleep(Duration::from_millis(1100));
    let b = store.create_thread(Some("proj-A"), "claude-code").unwrap();
    thread::sleep(Duration::from_millis(1100));
    let c = store.create_thread(Some("proj-A"), "claude-code").unwrap();

    // List should be [c, b, a] -- newest first.
    let listed = store.list_for_project(Some("proj-A")).unwrap();
    let ids: Vec<_> = listed.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids, vec![c.clone(), b.clone(), a.clone()]);

    // Append to the oldest -> it should jump to the front.
    thread::sleep(Duration::from_millis(1100));
    store
        .append_message(&a, &Message::user_text("touch"))
        .unwrap();
    let listed = store.list_for_project(Some("proj-A")).unwrap();
    let ids: Vec<_> = listed.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids, vec![a, c, b]);
}

#[test]
fn list_for_project_isolates_by_project_id() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);

    let a_thread = store.create_thread(Some("proj-A"), "claude-code").unwrap();
    let b_thread = store.create_thread(Some("proj-B"), "claude-code").unwrap();
    let no_proj = store.create_thread(None, "claude-code").unwrap();

    let a = store.list_for_project(Some("proj-A")).unwrap();
    let b = store.list_for_project(Some("proj-B")).unwrap();
    let unassigned = store.list_for_project(None).unwrap();

    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(unassigned.len(), 1);
    assert_eq!(a[0].id, a_thread);
    assert_eq!(b[0].id, b_thread);
    assert_eq!(unassigned[0].id, no_proj);
}

#[test]
fn delete_thread_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let id = store.create_thread(None, "claude-code").unwrap();

    store.delete_thread(&id).unwrap();
    // Second delete must NOT error.
    store.delete_thread(&id).unwrap();

    let err = store.load_thread(&id).unwrap_err();
    assert!(matches!(
        err,
        paneflow_threads::ThreadStoreError::NotFound(_)
    ));
}

#[test]
fn set_summary_bumps_updated_at_and_renames_row() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let id = store.create_thread(Some("proj"), "claude-code").unwrap();
    let before = &store.list_for_project(Some("proj")).unwrap()[0];
    assert_eq!(before.summary, "");

    thread::sleep(Duration::from_millis(1100));
    store.set_summary(&id, "What is 2 + 2?").unwrap();

    let after = &store.list_for_project(Some("proj")).unwrap()[0];
    assert_eq!(after.summary, "What is 2 + 2?");
    assert!(
        after.updated_at > before.updated_at,
        "updated_at must move forward: {} -> {}",
        before.updated_at,
        after.updated_at
    );
}

#[test]
fn concurrent_writers_do_not_corrupt_or_panic() {
    // Two threads append 50 messages each to two different rows. Both
    // must finish without panicking and both rows must end with
    // exactly 50 messages -- proving the Mutex<Connection> +
    // transaction model serialises correctly under contention.
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let store = Arc::new(store);

    let id_a = store.create_thread(None, "claude-code").unwrap();
    let id_b = store.create_thread(None, "claude-code").unwrap();

    let s_a = Arc::clone(&store);
    let s_b = Arc::clone(&store);
    let id_a_for_thread = id_a.clone();
    let id_b_for_thread = id_b.clone();
    let h_a = thread::spawn(move || {
        for i in 0..50 {
            s_a.append_message(&id_a_for_thread, &Message::user_text(format!("a{i}")))
                .unwrap();
        }
    });
    let h_b = thread::spawn(move || {
        for i in 0..50 {
            s_b.append_message(&id_b_for_thread, &Message::user_text(format!("b{i}")))
                .unwrap();
        }
    });
    h_a.join().expect("writer A panicked");
    h_b.join().expect("writer B panicked");

    let loaded_a = store.load_thread(&id_a).unwrap();
    let loaded_b = store.load_thread(&id_b).unwrap();
    assert_eq!(loaded_a.len(), 50, "writer A lost messages");
    assert_eq!(loaded_b.len(), 50, "writer B lost messages");
    // Spot-check that the messages are correctly ordered per writer
    // (Mutex serialises each row's transaction; within a single
    // writer, ordering is preserved by definition).
    for (i, msg) in loaded_a.iter().enumerate() {
        match &msg.content[0] {
            ContentBlock::Text(TextContent { text, .. }) => assert_eq!(text, &format!("a{i}")),
            other => panic!("expected text, got {other:?}"),
        }
    }
}

#[test]
fn concurrent_same_thread_appends_do_not_lose_messages() {
    // Regression test for the lost-update race in `append_message`:
    // two writers racing on the SAME ThreadId must observe all writes,
    // not a partial set. With the old non-atomic load+replace, two
    // concurrent appends could read the same base, both compute base+1,
    // and one write would silently overwrite the other.
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let store = Arc::new(store);
    let id = store.create_thread(None, "claude-code").unwrap();

    let writers = 4;
    let per_writer = 25;
    let total_expected = writers * per_writer;

    let handles: Vec<_> = (0..writers)
        .map(|w| {
            let store = Arc::clone(&store);
            let id = id.clone();
            thread::spawn(move || {
                for i in 0..per_writer {
                    store
                        .append_message(&id, &Message::user_text(format!("w{w}-i{i}")))
                        .unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("writer panicked");
    }

    let loaded = store.load_thread(&id).unwrap();
    assert_eq!(
        loaded.len(),
        total_expected,
        "all {total_expected} writes must persist (got {})",
        loaded.len()
    );
}

#[test]
fn data_survives_close_and_reopen() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("threads.db");
    let id = {
        let (store, _) = ThreadStore::open_at(&path).unwrap();
        let id = store.create_thread(Some("proj"), "claude-code").unwrap();
        store
            .append_message(&id, &Message::user_text("persistent"))
            .unwrap();
        store.set_summary(&id, "Persistent thread").unwrap();
        id
    };

    // Open a fresh ThreadStore on the same file -- must read the row.
    let (store, outcome) = ThreadStore::open_at(&path).unwrap();
    assert!(matches!(outcome, OpenOutcome::Opened));
    let loaded = store.load_thread(&id).unwrap();
    assert_eq!(loaded.len(), 1);
    let listed = store.list_for_project(Some("proj")).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].summary, "Persistent thread");
}

// =================================================================
// US-102 (prd-agent-ui-refactor-2026-Q3): draft store round-trip.
// =================================================================

fn text_block(text: &str) -> ContentBlock {
    ContentBlock::Text(TextContent::new(text))
}

#[test]
fn draft_round_trip_preserves_content_blocks() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let thread_id = store.create_thread(None, "claude-code").unwrap();

    assert!(
        store.read_draft(&thread_id).unwrap().is_none(),
        "fresh thread has no draft"
    );

    let blocks = vec![text_block("write a haiku"), text_block("about Rust")];
    store.write_draft(&thread_id, &blocks).unwrap();

    let loaded = store
        .read_draft(&thread_id)
        .unwrap()
        .expect("draft must round-trip");
    assert_eq!(loaded.len(), 2);
    match &loaded[0] {
        ContentBlock::Text(t) => assert_eq!(t.text, "write a haiku"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn write_draft_is_upsert_not_append() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let thread_id = store.create_thread(None, "claude-code").unwrap();

    store
        .write_draft(&thread_id, &[text_block("first take")])
        .unwrap();
    store
        .write_draft(&thread_id, &[text_block("second take")])
        .unwrap();

    let loaded = store.read_draft(&thread_id).unwrap().unwrap();
    assert_eq!(loaded.len(), 1, "second write must replace, not append");
    match &loaded[0] {
        ContentBlock::Text(t) => assert_eq!(t.text, "second take"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn delete_draft_clears_row() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let thread_id = store.create_thread(None, "claude-code").unwrap();

    store
        .write_draft(&thread_id, &[text_block("about to send")])
        .unwrap();
    assert!(store.read_draft(&thread_id).unwrap().is_some());

    store.delete_draft(&thread_id).unwrap();
    assert!(store.read_draft(&thread_id).unwrap().is_none());

    // AC #5: deleting a non-existent draft is not an error
    // (mirrors `delete_thread`'s idempotent behavior).
    store.delete_draft(&thread_id).unwrap();
}

#[test]
fn corrupted_draft_payload_falls_back_to_empty() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = open(&tmp);
    let thread_id = store.create_thread(None, "claude-code").unwrap();

    // Manually inject a malformed payload to simulate a crash mid-write
    // or a hand-edited DB. Going through rusqlite directly avoids the
    // public API's JSON-validating contract.
    let conn = rusqlite::Connection::open(store.db_path()).unwrap();
    conn.execute(
        "INSERT INTO drafts (thread_id, payload, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![thread_id.as_str(), b"\xff\xfenot json".to_vec(), "now"],
    )
    .unwrap();

    // AC #7: read returns Ok(None) -- composer opens empty, no panic.
    assert!(store.read_draft(&thread_id).unwrap().is_none());

    // The row is still present; a successful write_draft replaces it
    // cleanly without leaving the malformed payload behind.
    store
        .write_draft(&thread_id, &[text_block("recovered")])
        .unwrap();
    let loaded = store.read_draft(&thread_id).unwrap().unwrap();
    assert_eq!(loaded.len(), 1);
}

#[test]
fn draft_table_survives_reopen_with_existing_data() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("threads.db");
    let id_str: String = {
        let (store, _) = ThreadStore::open_at(&path).unwrap();
        let id = store.create_thread(None, "codex").unwrap();
        store
            .write_draft(&id, &[text_block("survives restart")])
            .unwrap();
        id.to_string()
    };

    // Re-open the same file. The schema migration must be idempotent
    // (`CREATE TABLE IF NOT EXISTS`) AND the draft row must still be
    // there.
    let (store, _) = ThreadStore::open_at(&path).unwrap();
    let id = paneflow_threads::store::ThreadId::from_string(id_str);
    let loaded = store.read_draft(&id).unwrap().unwrap();
    assert_eq!(loaded.len(), 1);
}
