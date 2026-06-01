//! Shared types and helpers for the AI-agent session readers
//! (`claude_sessions`, `codex_sessions`, `opencode_sessions`). Each reader
//! sources sessions from its agent's native store — JSONL transcripts on
//! disk for Claude / Codex, `opencode session list --format json` shell-out
//! for OpenCode (whose backing SQLite schema is intentionally not exposed
//! as a stable contract; see US-001 spike notes in
//! `tasks/prd-opencode-sessions-decisions.md`). All three normalise to
//! the unified [`SessionMeta`] below so the popover UI can render rows
//! with a single template.

/// Which AI agent created the session. Drives the row icon, the
/// `--resume` command shape, and the popover tab the row sits under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionAgent {
    Claude,
    Codex,
    OpenCode,
}

/// US-017 (audit P2-5): module-level mtime-keyed cache for the
/// session readers. The popover scan currently re-walks the on-disk
/// JSONL store on every workspace switch -- a 100-session project
/// pays the parse cost each time. The cache stores the last
/// successful scan keyed by `(agent, cwd)` plus the directory mtime
/// observed at scan time. A subsequent scan with an unchanged mtime
/// returns the cached vector directly.
///
/// Only the Claude reader uses this in v1 because its on-disk layout
/// (`~/.claude/projects/<slug>/*.jsonl`) is flat -- adding or
/// removing a session file changes the parent directory mtime
/// reliably. Codex stores sessions under a `YYYY/MM/DD/` partitioned
/// tree where the root mtime does NOT reflect leaf-file changes;
/// caching Codex correctly needs per-leaf-dir mtimes (deferred).
/// OpenCode runs an external CLI (`opencode session list`) and
/// cannot be invalidated via filesystem mtime at all.
pub mod cache {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, SystemTime};

    use super::{SessionAgent, SessionMeta};

    /// US-025 (cli-hardening-followup-2026-Q3): LRU capacity cap.
    /// The session-cache HashMap was process-global with no upper
    /// bound; switching across 20+ project directories (a common
    /// workflow for someone juggling client repos) used to grow
    /// the map monotonically. 10 entries is sized for the typical
    /// "active set" of recently-opened projects -- past that the
    /// least-recently-accessed entry is evicted on the next store.
    pub const MAX_CACHE_ENTRIES: usize = 10;

    /// US-025: monotonically-increasing access stamp. Bumped on
    /// every `lookup` hit and every `store_result` write so the
    /// smallest stamp identifies the LRU entry to evict.
    fn next_access_seq() -> u64 {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        SEQ.fetch_add(1, Ordering::Relaxed)
    }

    /// Maximum forward drift between the cached mtime and a newly
    /// observed mtime before we treat the cache as stale. APFS samples
    /// mtimes at nanosecond precision but its sampling jitter can
    /// produce drift of a few hundred nanoseconds even when no file
    /// actually changed -- a strict `==` would spuriously re-scan on
    /// every popover open (apenwarr, "mtime comparison considered
    /// harmful", 2018). 1ms is small enough that any real human-
    /// induced write (always seconds apart) is still caught and large
    /// enough to absorb all known filesystem-internal jitter.
    const MTIME_FUZZ: Duration = Duration::from_millis(1);

    struct Entry {
        mtime: SystemTime,
        sessions: Vec<SessionMeta>,
        /// US-025: monotonic stamp of the last access (lookup hit
        /// or store_result write). Used by `store_result` to pick
        /// the LRU victim when the cache hits its cap.
        access_seq: u64,
    }

    fn store() -> &'static Mutex<HashMap<(SessionAgent, String), Entry>> {
        static CACHE: OnceLock<Mutex<HashMap<(SessionAgent, String), Entry>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Read the directory mtime, returning `None` when the path does
    /// not exist or its metadata is unreadable. Both cases skip the
    /// cache (caller falls through to the scan and does not store the
    /// result).
    fn dir_mtime(dir: &Path) -> Option<SystemTime> {
        std::fs::metadata(dir).ok().and_then(|m| m.modified().ok())
    }

    /// True when `observed` has not advanced past `cached` by more
    /// than [`MTIME_FUZZ`]. mtimes only ever move forward, so the
    /// `Err` branch of `duration_since` (clock went backwards, NTP
    /// correction, DST transition) is treated as stale -- conservative
    /// safe-direction failure.
    fn within_fuzz(cached: SystemTime, observed: SystemTime) -> bool {
        match observed.duration_since(cached) {
            Ok(delta) => delta < MTIME_FUZZ,
            Err(_) => false,
        }
    }

    /// Try to read a fresh `Vec<SessionMeta>` from the cache. Returns
    /// `Some` only when the dir's mtime is within `MTIME_FUZZ` of the
    /// cached snapshot's mtime -- catches real writes (seconds apart)
    /// without spurious invalidation on filesystem-internal jitter.
    pub fn lookup(agent: SessionAgent, cwd: &str, project_dir: &Path) -> Option<Vec<SessionMeta>> {
        let observed = dir_mtime(project_dir)?;
        let mut guard = match store().lock() {
            Ok(g) => g,
            // US-008 (cli-hardening-followup-2026-Q3): a poisoned
            // session cache mutex means a prior holder panicked
            // mid-write. The recovered state may be partially-written
            // -- callers see a possibly-stale `Vec<SessionMeta>` once,
            // then `store_result` replaces it on the next scan. Log
            // so the previous panic is not hidden (matches
            // `lock_with_poison_log` in agent_terminal.rs).
            Err(p) => {
                tracing::warn!(
                    target: "paneflow_app::agent_sessions",
                    "session cache mutex poisoned on lookup; using potentially stale data \
                     (a previous thread panicked while holding the lock)"
                );
                p.into_inner()
            }
        };
        // US-025 (cli-hardening-followup-2026-Q3): bump the access
        // stamp on a cache hit so subsequent stores see this entry
        // as recently-used and pick a colder entry to evict.
        let entry = guard.get_mut(&(agent, cwd.to_string()))?;
        if within_fuzz(entry.mtime, observed) {
            entry.access_seq = next_access_seq();
            Some(entry.sessions.clone())
        } else {
            None
        }
    }

    /// Store the result of a fresh scan. The mtime is captured AFTER
    /// the scan to avoid the race where a write lands between the
    /// pre-scan mtime read and the post-scan write -- using the
    /// post-scan mtime means a follow-up write also invalidates the
    /// entry.
    pub fn store_result(
        agent: SessionAgent,
        cwd: &str,
        project_dir: &Path,
        sessions: &[SessionMeta],
    ) {
        let Some(mtime) = dir_mtime(project_dir) else {
            return;
        };
        let mut guard = match store().lock() {
            Ok(g) => g,
            // US-008 (cli-hardening-followup-2026-Q3): log poison
            // recovery on the write path too. The `insert` overwrites
            // the entry so the data is restored to a consistent state
            // on the next scan, but the previous panic deserves a
            // breadcrumb.
            Err(p) => {
                tracing::warn!(
                    target: "paneflow_app::agent_sessions",
                    "session cache mutex poisoned on store_result; overwriting entry \
                     (a previous thread panicked while holding the lock)"
                );
                p.into_inner()
            }
        };
        let key = (agent, cwd.to_string());
        // US-025 (cli-hardening-followup-2026-Q3): when the cache
        // is at capacity AND this key would be a NEW entry (not an
        // overwrite of an existing one), drop the LRU entry first.
        // The linear scan is O(N) on N=10 -- cheap enough versus
        // pulling in an `lru` crate dependency for one call site.
        if guard.len() >= MAX_CACHE_ENTRIES
            && !guard.contains_key(&key)
            && let Some((victim_key, victim_seq)) = guard
                .iter()
                .map(|(k, v)| (k.clone(), v.access_seq))
                .min_by_key(|(_, seq)| *seq)
        {
            tracing::debug!(
                target: "paneflow_app::agent_sessions",
                "session cache LRU eviction: (agent={:?}, cwd={}) seq={}",
                victim_key.0, victim_key.1, victim_seq,
            );
            guard.remove(&victim_key);
        }
        guard.insert(
            key,
            Entry {
                mtime,
                sessions: sessions.to_vec(),
                access_seq: next_access_seq(),
            },
        );
    }

    /// Drop everything; used by tests to reset state between cases.
    #[cfg(test)]
    pub fn clear() {
        let cache = store();
        match cache.lock() {
            Ok(mut g) => g.clear(),
            Err(p) => p.into_inner().clear(),
        }
        cache.clear_poison();
    }

    #[cfg(test)]
    mod tests {
        use super::{MTIME_FUZZ, within_fuzz};
        use std::time::{Duration, SystemTime};
        use tracing_test::traced_test;

        /// Cache-stateful tests share the process-global `store()`
        /// Mutex. `poisoned_session_cache_logs_warning` deliberately
        /// poisons it from a spawned thread; run concurrently with
        /// `session_cache_evicts_lru` (which locks `store()` directly
        /// and asserts exact contents) it made the latter flaky -- the
        /// LRU test would observe the poison mid-run and panic. cargo
        /// runs tests in parallel within one process, so serialize the
        /// two so each owns the shared cache exclusively. The guard is
        /// itself poison-tolerant for the same reason.
        fn serial() -> std::sync::MutexGuard<'static, ()> {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            LOCK.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// EP-004 review follow-up: a strict `==` mtime check would
        /// spuriously invalidate on APFS nanosecond jitter. With the
        /// fuzz band, observed - cached < 1ms reads as fresh.
        #[test]
        fn within_fuzz_accepts_subms_drift() {
            let cached = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
            // 500 microseconds forward drift -- well inside the fuzz band.
            let observed = cached + Duration::from_micros(500);
            assert!(
                within_fuzz(cached, observed),
                "{:?} sub-ms drift should be tolerated",
                MTIME_FUZZ,
            );
        }

        /// EP-004 review follow-up: a real human-induced write lands
        /// seconds after the cached scan and MUST invalidate the
        /// cache. A 5 ms forward drift already exceeds the 1 ms fuzz
        /// band -- representative of any real file mutation.
        #[test]
        fn within_fuzz_rejects_real_change() {
            let cached = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
            let observed = cached + Duration::from_millis(5);
            assert!(
                !within_fuzz(cached, observed),
                "5 ms drift (well past {:?}) should invalidate",
                MTIME_FUZZ,
            );
        }

        /// Identical mtime is the no-write hot path. Must still match.
        #[test]
        fn within_fuzz_accepts_exact_match() {
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
            assert!(within_fuzz(t, t));
        }

        /// Backwards drift (NTP correction, DST transition) is treated
        /// as stale -- safe-direction failure since mtimes only ever
        /// move forward and a backwards observation means something
        /// underneath us is unreliable.
        #[test]
        fn within_fuzz_rejects_backwards_drift() {
            let cached = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
            let observed = cached - Duration::from_millis(10);
            assert!(!within_fuzz(cached, observed));
        }

        /// US-025 (cli-hardening-followup-2026-Q3): when the cache
        /// is at capacity, inserting a new distinct (agent, cwd)
        /// must evict the least-recently-accessed existing entry.
        ///
        /// Review 2026-05-28: switched from inlining the eviction
        /// logic into the test (which risked drifting away from the
        /// production path in `store_result`) to calling
        /// `store_result` directly with a tempdir, so the test now
        /// exercises the real production code path. The earlier
        /// shape was kept only because `store_result` reads
        /// `dir_mtime`; supplying a tempdir satisfies that without
        /// duplicating the eviction branch.
        #[test]
        fn session_cache_evicts_lru() {
            use super::super::SessionAgent;
            use super::Entry;
            // Serialize against `poisoned_session_cache_logs_warning`,
            // which poisons the shared `store()` Mutex from a parallel
            // thread; without this the direct lock below sees a
            // PoisonError and panics (CI flake, aarch64 scheduling).
            let _serial = serial();
            // Isolate from any sibling test's cache state.
            super::clear();
            // Real on-disk dir for `store_result`'s `dir_mtime` call.
            let dir = tempfile::tempdir().expect("tempdir");
            // Seed the cache directly to MAX_CACHE_ENTRIES so the
            // 11th insert exercises the production eviction path.
            // We bypass `store_result` for the seeding step because
            // `dir_mtime` returns the SAME mtime for the SAME path,
            // and we want each entry to have a distinct
            // (agent, cwd) key for the LRU comparison.
            {
                let mut guard = super::store().lock().expect("lock");
                for i in 0..super::MAX_CACHE_ENTRIES {
                    let key = (SessionAgent::Claude, format!("/proj-{i}"));
                    guard.insert(
                        key,
                        Entry {
                            mtime: SystemTime::UNIX_EPOCH,
                            sessions: Vec::new(),
                            access_seq: super::next_access_seq(),
                        },
                    );
                }
                assert_eq!(guard.len(), super::MAX_CACHE_ENTRIES);
                // /proj-0 is the oldest entry by access_seq.
                assert!(guard.contains_key(&(SessionAgent::Claude, "/proj-0".to_string())));
            }
            // Insert the 11th distinct entry via the REAL production
            // path: `store_result` enforces the cap, picks the LRU
            // victim, evicts it, then inserts. This catches any
            // future drift in the eviction branch (line 179-192).
            super::store_result(SessionAgent::Claude, "/proj-N", dir.path(), &[]);
            {
                let guard = super::store().lock().expect("lock");
                assert_eq!(
                    guard.len(),
                    super::MAX_CACHE_ENTRIES,
                    "cache must stay at cap after store_result eviction"
                );
                assert!(
                    guard.contains_key(&(SessionAgent::Claude, "/proj-N".to_string())),
                    "new entry must be present"
                );
                assert!(
                    !guard.contains_key(&(SessionAgent::Claude, "/proj-0".to_string())),
                    "LRU victim (proj-0) must have been evicted"
                );
            }
            super::clear();
        }

        /// US-008 (cli-hardening-followup-2026-Q3): poison recovery
        /// must leave a log breadcrumb. The cache can still recover
        /// with `PoisonError::into_inner`, but the previous panic is
        /// operationally relevant and should not disappear.
        #[test]
        #[traced_test]
        fn poisoned_session_cache_logs_warning() {
            use super::super::SessionAgent;

            // See `serial()` -- this test poisons the shared cache, so
            // it must not overlap `session_cache_evicts_lru`.
            let _serial = serial();
            super::clear();
            let _ = std::thread::spawn(|| {
                let _guard = super::store().lock().expect("lock cache for poison");
                panic!("force session cache poison");
            })
            .join();

            let dir = tempfile::tempdir().expect("tempdir");
            super::store_result(SessionAgent::Claude, "/poisoned", dir.path(), &[]);

            assert!(
                logs_contain("session cache mutex poisoned on store_result"),
                "poison recovery warning should be emitted"
            );
            super::clear();
        }
    }
}

/// Read user config and return the agents whose tab-bar button is currently
/// visible, in display order (Claude → Codex → OpenCode). Both the popover
/// tab strip and the on-open session scans filter through this so a hidden
/// agent never appears in the UI and we don't pay the I/O cost of a scan
/// the user can't see. An unset field is treated as visible (matches the
/// behaviour in `pane.rs` where the buttons render by default).
pub fn enabled_session_agents() -> Vec<SessionAgent> {
    let cfg = paneflow_config::loader::load_config();
    let mut agents = Vec::with_capacity(3);
    if cfg.claude_code_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::Claude);
    }
    if cfg.codex_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::Codex);
    }
    if cfg.opencode_button_visible.unwrap_or(true) {
        agents.push(SessionAgent::OpenCode);
    }
    agents
}

/// Unified session metadata. Anything the UI needs to render a row +
/// resume the session is here; the heavier message payload stays on disk.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    /// Which CLI created the session — drives row routing and the resume
    /// command (`claude --resume <id>` vs `codex resume <id>`).
    pub agent: SessionAgent,
    pub session_id: String,
    /// ISO 8601 timestamp from the first event. Used for sorting (lexical
    /// sort matches chronological order for ISO 8601).
    pub timestamp: String,
    /// `cwd` recorded on the first line. Files where the first line lacks
    /// `cwd` are skipped, so this is always populated.
    pub cwd: String,
    /// Git branch — empty string when the session was outside a git repo
    /// (Claude Code) or when the agent doesn't record one (Codex CLI).
    /// Retained in the data contract (the scans still populate it) but no
    /// longer surfaced in the sidebar UI.
    #[allow(dead_code)]
    pub git_branch: String,
    /// Human-readable session label. Sourced from an LLM-generated title
    /// when available, falling back to the cleaned first user message
    /// otherwise. `None` if neither could be extracted.
    pub summary: Option<String>,
}

/// Format an ISO 8601 timestamp into a short relative label. Pure string
/// math (no `chrono` dep) — parses `YYYY-MM-DDTHH:MM:SS` and computes the
/// delta against `std::time::SystemTime::now()` via a calendar-free
/// approximation good enough for "Xm ago" / "Xh ago" / "Xd ago" labels.
///
/// Falls back to the date prefix (`YYYY-MM-DD`) when parsing fails.
pub fn format_relative_time(iso8601: &str) -> String {
    // US-009 (cli-hardening-followup-2026-Q3): if the system clock
    // is before UNIX_EPOCH (impossible in practice, but
    // `duration_since` returns `Err` and the previous `unwrap_or(0)`
    // silently mapped every session to "30+ days ago"), drop to the
    // ISO-date prefix fallback instead. Future NTP step-backwards
    // beyond the cached `parse_iso8601_to_unix_secs` is already
    // safe-bounded by `saturating_sub`.
    //
    // US-026 (cli-hardening-followup-2026-Q3): the fallback path
    // additionally trims to the first 10 characters defensively --
    // a malformed JSONL field containing a newline or
    // `<script>alert(1)</script>` will not blow up the sidebar
    // layout. Well-formed `YYYY-MM-DDTHH:MM:SS` and
    // `YYYY-MM-DD` inputs are already <= 10 chars after the
    // `split('T').next()` so the clamp is a no-op for them.
    let now_secs = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return iso8601_safe_fallback(iso8601),
    };

    match parse_iso8601_to_unix_secs(iso8601) {
        Some(ts_secs) => {
            let delta = now_secs.saturating_sub(ts_secs);
            relative_label(delta)
        }
        None => iso8601_safe_fallback(iso8601),
    }
}

/// US-026 (cli-hardening-followup-2026-Q3): defensive ISO-date
/// fallback. Takes the substring before the first `T` (the typical
/// `YYYY-MM-DD` prefix of an ISO-8601 timestamp), then clamps to
/// the first 10 chars on a `chars()` boundary (UTF-8 safe).
fn iso8601_safe_fallback(iso8601: &str) -> String {
    iso8601
        .split('T')
        .next()
        .unwrap_or(iso8601)
        .chars()
        .take(10)
        .collect()
}

fn relative_label(delta_secs: i64) -> String {
    if delta_secs < 60 {
        return "just now".to_string();
    }
    if delta_secs < 3_600 {
        return format!("{}m ago", delta_secs / 60);
    }
    if delta_secs < 86_400 {
        return format!("{}h ago", delta_secs / 3_600);
    }
    if delta_secs < 30 * 86_400 {
        return format!("{}d ago", delta_secs / 86_400);
    }
    if delta_secs < 365 * 86_400 {
        return format!("{}mo ago", delta_secs / (30 * 86_400));
    }
    format!("{}y ago", delta_secs / (365 * 86_400))
}

/// Minimal ISO 8601 → Unix-seconds parser. Accepts
/// `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`. Treats the timestamp as UTC.
/// Calendar math via Howard Hinnant's "days from civil" algorithm; an
/// off-by-one on leap-second boundaries is acceptable for a relative-time
/// UI label.
fn parse_iso8601_to_unix_secs(iso: &str) -> Option<i64> {
    let (date, rest) = iso.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let time = rest
        .split_once(['Z', '+', '-'])
        .map(|(t, _)| t)
        .unwrap_or(rest);
    let time = time.split('.').next().unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146_097 + doe - 719_468;

    Some(days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_label_under_minute() {
        assert_eq!(relative_label(15), "just now");
    }

    #[test]
    fn relative_label_minutes() {
        assert_eq!(relative_label(125), "2m ago");
    }

    #[test]
    fn relative_label_hours() {
        assert_eq!(relative_label(7_400), "2h ago");
    }

    #[test]
    fn relative_label_days() {
        assert_eq!(relative_label(3 * 86_400 + 100), "3d ago");
    }

    #[test]
    fn iso8601_parses_z() {
        let secs = parse_iso8601_to_unix_secs("2025-01-15T12:30:45Z").unwrap();
        assert_eq!(secs, 1_736_944_245);
    }

    #[test]
    fn iso8601_parses_fractional_seconds() {
        let secs = parse_iso8601_to_unix_secs("2025-01-15T12:30:45.123Z").unwrap();
        assert_eq!(secs, 1_736_944_245);
    }

    /// US-026 (cli-hardening-followup-2026-Q3): the fallback now
    /// also clamps to 10 chars, so an unparseable input is truncated.
    /// The original assertion is updated accordingly; the storage
    /// shape didn't change (still ISO-date prefix shape), only its
    /// length is bounded.
    #[test]
    fn iso8601_unparseable_falls_back_to_date_prefix() {
        let label = format_relative_time("not a real timestamp");
        // "not a real timestamp" has no 'T', so `split('T').next()`
        // returns the whole string; the 10-char clamp then trims it.
        assert_eq!(label, "not a real");
    }

    /// US-026 (cli-hardening-followup-2026-Q3): the fallback must
    /// be safe against malformed inputs that would otherwise blow
    /// up the sidebar layout (newlines, oversize strings, multi-byte).
    #[test]
    fn iso8601_date_trims_to_10_chars() {
        // Well-formed: behaves as before.
        let well = format_relative_time("definitely-not-a-timestamp-2025");
        assert_eq!(well.chars().count(), 10);

        // Newline-injected malformed input: only the date prefix
        // (or the first 10 chars in the absence of 'T') is kept.
        let malicious = format_relative_time("2026-05-28\n<script>alert(1)</script>");
        assert!(!malicious.contains('\n'));
        assert!(malicious.chars().count() <= 10);

        // Multi-byte safety: a `cafétimestamp` truncates on a char
        // boundary (chars().take(10) -- not bytes().take(10)).
        let multi = format_relative_time("café-timestamp-very-long");
        assert_eq!(multi.chars().count(), 10);
    }
}
