// US-018: Hot-reload via file watcher

use crate::loader::{config_path, load_config_from_path};
use crate::schema::PaneFlowConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Debounce window: accumulate file events for this duration before reloading.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(300);

/// Watches the PaneFlow config file for changes and triggers hot-reload.
///
/// The watcher monitors the parent directory (not the file directly) so that
/// editor save patterns involving delete+recreate (atomic saves) are captured.
/// File events are debounced at 300ms to coalesce rapid sequences of writes.
pub struct ConfigWatcher {
    callback: Arc<dyn Fn(PaneFlowConfig) + Send + Sync>,
    config_path: PathBuf,
}

impl ConfigWatcher {
    /// Creates a new `ConfigWatcher` that will invoke `callback` with the new
    /// configuration whenever the config file is successfully reloaded.
    ///
    /// Uses `config_path()` to determine which file to watch. Panics if no
    /// config directory can be determined (this should not happen on supported
    /// platforms).
    // Invariant: `config_path()` returns `Some` on every supported platform
    // (Linux/macOS: `dirs::config_dir()`; Windows: `%APPDATA%`). A `None`
    // here means the user's environment is so broken (e.g., unset `HOME`
    // AND `USERPROFILE`) that starting the app is meaningless. `expect` is
    // the right behavior — documented invariant per CLAUDE.md.
    #[allow(clippy::expect_used)]
    pub fn new(callback: Arc<dyn Fn(PaneFlowConfig) + Send + Sync>) -> Self {
        let config_path =
            config_path().expect("could not determine config path for the current platform");
        Self {
            callback,
            config_path,
        }
    }

    /// Creates a `ConfigWatcher` targeting a specific path — useful for testing.
    #[cfg(test)]
    fn new_with_path(path: PathBuf, callback: Arc<dyn Fn(PaneFlowConfig) + Send + Sync>) -> Self {
        Self {
            callback,
            config_path: path,
        }
    }

    /// Starts watching the config file's parent directory for changes.
    ///
    /// Spawns a background thread that:
    /// 1. Receives raw file-system events from `notify::RecommendedWatcher`
    /// 2. Debounces them over a 300ms window
    /// 3. Reloads and validates the config file
    /// 4. Calls the callback on success, or logs a warning on failure
    ///
    /// Returns `Ok(())` once the watcher is installed, or an error if the
    /// underlying OS watcher could not be created.
    pub fn start(&self) -> Result<(), notify::Error> {
        // Invariant: `self.config_path` is always a file path built from
        // `config_path()` (e.g., `/home/u/.config/paneflow/paneflow.json`),
        // so `.parent()` is guaranteed to be `Some`. `expect` is correct
        // here — documented invariant per CLAUDE.md.
        #[allow(clippy::expect_used)]
        let watch_dir = self
            .config_path
            .parent()
            .expect("config path has no parent directory")
            .to_path_buf();

        // notify can't watch a directory that doesn't exist yet — create it
        // on first run so hot-reload works even before the user writes a config.
        if !watch_dir.exists() {
            std::fs::create_dir_all(&watch_dir).map_err(notify::Error::io)?;
        }

        let config_path = self.config_path.clone();
        let callback = Arc::clone(&self.callback);

        // Channel for notify -> processing thread.
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

        // Create the OS file watcher. It sends events through `tx`.
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                // Best-effort send; if the receiver is gone the watcher is being dropped.
                let _ = tx.send(res);
            },
            notify::Config::default(),
        )?;

        // Watch the parent directory (non-recursive) to catch delete+recreate.
        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

        // Spawn the event-processing loop in a background thread.
        // The thread owns `watcher` to keep it alive.
        thread::spawn(move || {
            event_loop(rx, &config_path, &callback, &watcher);
        });

        info!(
            path = %self.config_path.display(),
            "config watcher started"
        );

        Ok(())
    }
}

/// Returns `true` if this event kind is relevant for config reload.
fn is_relevant_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Returns `true` if any path in the event matches the config file.
///
/// Matches by file name rather than full path: platforms rewrite watched
/// paths before emitting events (macOS FSEvents canonicalizes
/// `/var/folders/...` to `/private/var/folders/...`, Windows sometimes uses
/// UNC `\\?\C:\...` prefixes) so a full-path comparison is inherently
/// fragile. Because the watcher is installed `NonRecursive` on the parent
/// directory, every event we receive already belongs to that directory —
/// basename equality is sufficient and portable.
fn event_targets_config(event: &Event, config_path: &Path) -> bool {
    let target_name = config_path.file_name();
    target_name.is_some() && event.paths.iter().any(|p| p.file_name() == target_name)
}

/// The main event-processing loop running on the background thread.
///
/// `_watcher` is kept alive by moving it into this scope — dropping it would
/// stop the OS-level file watch.
fn event_loop(
    rx: mpsc::Receiver<notify::Result<Event>>,
    config_path: &PathBuf,
    callback: &Arc<dyn Fn(PaneFlowConfig) + Send + Sync>,
    _watcher: &RecommendedWatcher,
) {
    // The last config that was successfully loaded (starts as the current one).
    let mut current_config = load_config_from_path(config_path);
    let mut pending_reload: Option<Instant> = None;

    loop {
        // If we have a pending reload, wait only until the debounce window expires.
        // Otherwise block indefinitely for the next event.
        let event_result = if let Some(deadline) = pending_reload {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Debounce window expired — do the reload.
                pending_reload = None;
                attempt_reload(config_path, &mut current_config, callback);
                continue;
            }
            rx.recv_timeout(remaining)
        } else {
            // No pending reload — block for the next event.
            match rx.recv() {
                Ok(ev) => Ok(ev),
                Err(_) => break, // Channel closed — watcher was dropped.
            }
        };

        match event_result {
            Ok(Ok(event)) => {
                if is_relevant_event(&event.kind) && event_targets_config(&event, config_path) {
                    // Start (or extend) the debounce window.
                    pending_reload = Some(Instant::now() + DEBOUNCE_DURATION);
                }
            }
            Ok(Err(e)) => {
                warn!("file watcher error: {e}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Debounce window expired.
                pending_reload = None;
                attempt_reload(config_path, &mut current_config, callback);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break; // Channel closed.
            }
        }
    }
}

/// Attempt to reload the config file. On success, call the callback and update
/// `current_config`. On failure (file deleted or invalid), log a warning and
/// keep the old config.
fn attempt_reload(
    config_path: &PathBuf,
    current_config: &mut PaneFlowConfig,
    callback: &Arc<dyn Fn(PaneFlowConfig) + Send + Sync>,
) {
    if !config_path.exists() {
        warn!(
            path = %config_path.display(),
            "config file was deleted; keeping previous config and continuing to watch"
        );
        return;
    }

    let contents = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(
                path = %config_path.display(),
                error = %e,
                "failed to read config file during reload; keeping previous config"
            );
            return;
        }
    };

    match serde_json::from_str::<PaneFlowConfig>(&contents) {
        Ok(new_config) => {
            info!("config reloaded successfully");
            *current_config = new_config.clone();
            callback(new_config);
        }
        Err(e) => {
            warn!(
                error = %e,
                "config file has validation errors; keeping previous config"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    /// Helper: write a valid minimal config file.
    fn write_valid_config(path: &PathBuf) {
        fs::write(path, r#"{"default_shell": "/bin/bash", "commands": []}"#).unwrap();
    }

    /// Helper: write an updated config file with a different shell.
    fn write_updated_config(path: &PathBuf) {
        fs::write(path, r#"{"default_shell": "/bin/zsh", "commands": []}"#).unwrap();
    }

    /// Helper: write invalid JSON to the config path.
    fn write_invalid_config(path: &PathBuf) {
        fs::write(path, "this is not valid json {{{").unwrap();
    }

    /// Poll `condition` every 50ms until it returns `true` or `timeout` elapses.
    /// Why: macOS FSEvents on CI runners can take >1s to deliver file events vs
    /// near-instant inotify on Linux; a fixed sleep is inherently flaky across
    /// platforms, so we poll instead.
    fn wait_for<F: FnMut() -> bool>(mut condition: F, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        condition()
    }

    #[test]
    fn test_config_watcher_new_with_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        let cb = Arc::new(|_: PaneFlowConfig| {});
        let watcher = ConfigWatcher::new_with_path(path.clone(), cb);
        assert_eq!(watcher.config_path, path);
    }

    #[test]
    fn test_is_relevant_event() {
        use notify::event::*;

        assert!(is_relevant_event(&EventKind::Create(CreateKind::File)));
        assert!(is_relevant_event(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
        assert!(is_relevant_event(&EventKind::Remove(RemoveKind::File)));
        assert!(!is_relevant_event(&EventKind::Access(AccessKind::Read)));
        assert!(!is_relevant_event(&EventKind::Other));
    }

    #[test]
    fn test_event_targets_config() {
        let config_path = PathBuf::from("/tmp/paneflow/paneflow.json");

        let matching_event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/tmp/paneflow/paneflow.json")],
            attrs: Default::default(),
        };
        assert!(event_targets_config(&matching_event, &config_path));

        let non_matching_event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/tmp/paneflow/other.json")],
            attrs: Default::default(),
        };
        assert!(!event_targets_config(&non_matching_event, &config_path));
    }

    #[test]
    fn test_attempt_reload_missing_file_keeps_old_config() {
        let path = PathBuf::from("/nonexistent/path/config.json");
        let mut current = PaneFlowConfig {
            default_shell: Some("/bin/bash".to_string()),
            ..Default::default()
        };
        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |_| *called_clone.lock().unwrap() = true);

        attempt_reload(&path, &mut current, &cb);

        assert!(!*called.lock().unwrap(), "callback should not be called");
        assert_eq!(
            current.default_shell,
            Some("/bin/bash".to_string()),
            "old config should be preserved"
        );
    }

    #[test]
    fn test_attempt_reload_invalid_json_keeps_old_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_invalid_config(&path);

        let mut current = PaneFlowConfig {
            default_shell: Some("/bin/bash".to_string()),
            ..Default::default()
        };
        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |_| *called_clone.lock().unwrap() = true);

        attempt_reload(&path, &mut current, &cb);

        assert!(
            !*called.lock().unwrap(),
            "callback should not be called for invalid JSON"
        );
        assert_eq!(
            current.default_shell,
            Some("/bin/bash".to_string()),
            "old config should be preserved"
        );
    }

    #[test]
    fn test_attempt_reload_valid_config_calls_callback() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_valid_config(&path);

        let mut current = PaneFlowConfig::default();
        let received = Arc::new(Mutex::new(None::<PaneFlowConfig>));
        let received_clone = Arc::clone(&received);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |cfg| *received_clone.lock().unwrap() = Some(cfg));

        attempt_reload(&path, &mut current, &cb);

        let received_cfg = received
            .lock()
            .unwrap()
            .clone()
            .expect("callback should be called");
        assert_eq!(received_cfg.default_shell, Some("/bin/bash".to_string()));
        assert_eq!(current.default_shell, Some("/bin/bash".to_string()));
    }

    #[test]
    fn test_watcher_detects_file_change() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_valid_config(&path);

        let received = Arc::new(Mutex::new(Vec::<PaneFlowConfig>::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |cfg| received_clone.lock().unwrap().push(cfg));

        let watcher = ConfigWatcher::new_with_path(path.clone(), cb);
        watcher.start().expect("watcher should start");

        // Give the watcher time to initialize.
        thread::sleep(Duration::from_millis(100));

        write_updated_config(&path);

        let received_poll = Arc::clone(&received);
        let fired = wait_for(
            move || !received_poll.lock().unwrap().is_empty(),
            Duration::from_secs(5),
        );
        assert!(fired, "callback should have been invoked at least once");

        let configs = received.lock().unwrap();
        let last = configs.last().unwrap();
        assert_eq!(last.default_shell, Some("/bin/zsh".to_string()));
    }

    #[test]
    fn test_watcher_invalid_change_keeps_old() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_valid_config(&path);

        let received = Arc::new(Mutex::new(Vec::<PaneFlowConfig>::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |cfg| received_clone.lock().unwrap().push(cfg));

        let watcher = ConfigWatcher::new_with_path(path.clone(), cb);
        watcher.start().expect("watcher should start");

        thread::sleep(Duration::from_millis(100));

        // Write invalid JSON.
        write_invalid_config(&path);

        // Wait for debounce + processing.
        thread::sleep(Duration::from_millis(800));

        let configs = received.lock().unwrap();
        // Callback should NOT have been called (invalid JSON is rejected).
        assert!(
            configs.is_empty(),
            "callback should not be invoked for invalid config"
        );
    }

    #[test]
    fn test_watcher_survives_file_deletion_and_recreation() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_valid_config(&path);

        let received = Arc::new(Mutex::new(Vec::<PaneFlowConfig>::new()));
        let received_clone = Arc::clone(&received);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |cfg| received_clone.lock().unwrap().push(cfg));

        let watcher = ConfigWatcher::new_with_path(path.clone(), cb);
        watcher.start().expect("watcher should start");

        thread::sleep(Duration::from_millis(100));

        // Delete the file, then recreate with new content. macOS FSEvents may
        // coalesce both into a single event batch, so we only wait on the
        // post-recreation callback rather than pausing between steps.
        fs::remove_file(&path).unwrap();
        write_updated_config(&path);

        let received_poll = Arc::clone(&received);
        let fired = wait_for(
            move || {
                let guard = received_poll.lock().unwrap();
                guard
                    .last()
                    .is_some_and(|cfg| cfg.default_shell.as_deref() == Some("/bin/zsh"))
            },
            Duration::from_secs(5),
        );
        assert!(fired, "callback should fire after file recreation");
    }

    #[test]
    fn test_debounce_coalesces_rapid_writes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paneflow.json");
        write_valid_config(&path);

        let call_count = Arc::new(Mutex::new(0u32));
        let call_count_clone = Arc::clone(&call_count);
        let cb: Arc<dyn Fn(PaneFlowConfig) + Send + Sync> =
            Arc::new(move |_| *call_count_clone.lock().unwrap() += 1);

        let watcher = ConfigWatcher::new_with_path(path.clone(), cb);
        watcher.start().expect("watcher should start");

        thread::sleep(Duration::from_millis(100));

        // Rapid-fire writes within the debounce window.
        for i in 0..5 {
            let shell = format!("/bin/shell{i}");
            let json = format!(r#"{{"default_shell": "{shell}", "commands": []}}"#);
            fs::write(&path, json).unwrap();
            thread::sleep(Duration::from_millis(50));
        }

        // Wait for at least one callback to fire (up to 5s for macOS CI).
        let call_count_poll = Arc::clone(&call_count);
        let fired = wait_for(
            move || *call_count_poll.lock().unwrap() >= 1,
            Duration::from_secs(5),
        );
        assert!(fired, "at least one reload should have occurred");

        // Then settle for an extra second so any trailing debounce flushes.
        thread::sleep(Duration::from_secs(1));

        let count = *call_count.lock().unwrap();
        // With debouncing, we should see fewer callbacks than writes.
        // Typically 1 (all coalesced), but timing may cause 2.
        assert!(
            count <= 2,
            "debounce should coalesce rapid writes, got {count} callbacks for 5 writes"
        );
    }
}
