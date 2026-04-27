//! `PaneFlowApp::new()` — the application constructor.
//!
//! Wires the title bar, IPC server, config watcher, git-dir watcher, update
//! checker, and all background tickers (50 ms IPC poll, 30 s git fallback,
//! 30 s stale-PID sweep). Restores a saved session or creates a fresh
//! single-workspace state.
//!
//! Extracted from `main.rs` per US-027 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged.

use gpui::{AppContext, Context};
use notify::Watcher;

use crate::pane::Pane;
use crate::telemetry;
use crate::terminal::TerminalView;
use crate::terminal::blink::{BlinkPhase, BlinkPhaseGlobal, CURSOR_BLINK_INTERVAL};
use crate::window_chrome::title_bar;
use crate::workspace::{Workspace, next_workspace_id};
use crate::{PaneFlowApp, ipc, keybindings, update};

impl PaneFlowApp {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let title_bar = cx.new(title_bar::TitleBar::new);
        cx.subscribe(&title_bar, Self::handle_title_bar_event)
            .detach();
        let ipc_rx = ipc::start_server();

        // US-006 — install the shared cursor-blink phase as a GPUI global
        // before any `TerminalView` is constructed. Each `TerminalView`
        // reads the global in `with_cwd` and observes the entity, so all
        // visible cursors blink in phase. One bootstrap-spawned loop
        // toggles `phase.visible` every 530 ms — replaces N per-terminal
        // `smol::Timer` loops with a single ticker for the whole app.
        let blink_phase = cx.new(|_| BlinkPhase::default());
        cx.set_global(BlinkPhaseGlobal(blink_phase.clone()));
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(CURSOR_BLINK_INTERVAL).await;
                    // Read the entity fresh from the global on every tick
                    // to keep this loop consistent with the existing
                    // git-watcher / IPC-poll patterns in this file: all of
                    // them go through `this.update(cx, |app, cx| ...)` and
                    // pull whatever they need from `cx`/`app` inside the
                    // closure rather than capturing it. Capturing the
                    // entity once would also be safe (the App owns the
                    // strong ref via the global; clones at app teardown
                    // are dropped together) — consistency wins.
                    let result = cx.update(|cx| {
                        this.update(cx, |_app: &mut Self, cx: &mut Context<Self>| {
                            let phase = cx.global::<BlinkPhaseGlobal>().0.clone();
                            phase.update(cx, |p, cx| {
                                p.visible = !p.visible;
                                cx.notify();
                            });
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // ConfigWatcher: background thread detects file changes (300ms debounce),
        // stores parsed config in a shared slot for the 50ms poll loop to pick up.
        // Note: `start()` moves the OS watcher into a background thread, so the
        // `ConfigWatcher` struct itself can be safely dropped after starting.
        let pending_config = std::sync::Arc::new(std::sync::Mutex::new(
            None::<paneflow_config::schema::PaneFlowConfig>,
        ));
        let pending_config_writer = std::sync::Arc::clone(&pending_config);
        let _config_watcher = paneflow_config::watcher::ConfigWatcher::new(std::sync::Arc::new(
            move |cfg: paneflow_config::schema::PaneFlowConfig| {
                *pending_config_writer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(cfg);
            },
        ));
        if let Err(e) = _config_watcher.start() {
            log::warn!("config watcher failed to start: {e}; config hot-reload disabled");
        }

        // Background update check (startup-only, non-blocking)
        let pending_update = update::checker::spawn_check();

        // Restore session or create a single default workspace
        let (workspaces, active_idx) = if let Some(session) = Self::load_session() {
            log::info!(
                "restoring session: {} workspace(s)",
                session.workspaces.len()
            );
            Self::restore_workspaces(&session, cx)
        } else {
            let ws_id = next_workspace_id();
            let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
            cx.subscribe(&terminal, Self::handle_terminal_event)
                .detach();
            let pane = cx.new(|cx| Pane::new(terminal, ws_id, cx));
            cx.subscribe(&pane, Self::handle_pane_event).detach();
            let dir_name = std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "Terminal 1".into());
            let ws = Workspace::with_id(ws_id, dir_name, pane);
            (vec![ws], 0)
        };

        // Setup notify file watcher for .git directories
        let (git_event_tx, git_event_rx) = std::sync::mpsc::channel();
        let mut git_watcher = match notify::recommended_watcher(git_event_tx) {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("git file watcher unavailable: {e}. Falling back to polling.");
                None
            }
        };
        let mut git_watch_counts = std::collections::HashMap::new();
        // Watch all workspaces' .git directories
        if let Some(ref mut watcher) = git_watcher {
            for ws in &workspaces {
                if let Some(ref git_dir) = ws.git_dir {
                    if let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive) {
                        log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
                    } else {
                        *git_watch_counts.entry(git_dir.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        // Poll git watcher events with 300ms debounce.
        // Filter: only HEAD and index matter. NonRecursive mode limits events to
        // top-level entries of .git/ so no subdirectory false positives.
        // On debounce fire, run git probes off main thread and apply results.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let debounce = std::time::Duration::from_millis(300);
                let mut last_event = std::time::Instant::now() - debounce;
                let mut pending = false;
                let mut pending_git_dirs = std::collections::HashSet::<std::path::PathBuf>::new();

                loop {
                    smol::Timer::after(std::time::Duration::from_millis(200)).await;

                    // Drain events from the watcher channel, collect affected .git dirs
                    let new_dirs = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                            let mut dirs = Vec::new();
                            while let Ok(event) = app.git_event_rx.try_recv() {
                                if let Ok(ref ev) = event {
                                    for p in &ev.paths {
                                        if matches!(
                                            p.file_name().and_then(|n| n.to_str()),
                                            Some("HEAD" | "index")
                                        ) && let Some(parent) = p.parent()
                                        {
                                            dirs.push(parent.to_path_buf());
                                        }
                                    }
                                }
                            }
                            dirs
                        })
                    });

                    match new_dirs {
                        Ok(dirs) if !dirs.is_empty() => {
                            pending_git_dirs.extend(dirs);
                            last_event = std::time::Instant::now();
                            pending = true;
                        }
                        Ok(_) => {}
                        Err(_) => break, // app shutting down
                    }

                    // Debounce: fire after 300ms of quiet
                    if pending && last_event.elapsed() >= debounce {
                        pending = false;
                        let affected_dirs = std::mem::take(&mut pending_git_dirs);
                        log::debug!(
                            "git watcher: debounced event fired for {} dir(s)",
                            affected_dirs.len()
                        );

                        // Collect CWDs of affected workspaces (main thread)
                        let cwds = cx.update(|cx| {
                            this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                                app.workspaces
                                    .iter()
                                    .filter(|ws| {
                                        ws.git_dir
                                            .as_ref()
                                            .is_some_and(|gd| affected_dirs.contains(gd))
                                    })
                                    .map(|ws| ws.cwd.clone())
                                    .collect::<Vec<String>>()
                            })
                        });

                        let cwds = match cwds {
                            Ok(c) => c,
                            Err(_) => break,
                        };

                        if cwds.is_empty() {
                            continue;
                        }

                        // Run git probes off main thread
                        let results = smol::unblock(move || {
                            cwds.into_iter()
                                .map(|cwd| {
                                    let (branch, is_repo) = crate::workspace::detect_branch(&cwd);
                                    let stats = crate::workspace::GitDiffStats::from_cwd(&cwd);
                                    (cwd, branch, is_repo, stats)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await;

                        // Apply results to matching workspaces (main thread)
                        let apply = cx.update(|cx| {
                            this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                                let mut changed = false;
                                for (cwd, branch, is_repo, stats) in &results {
                                    for ws in &mut app.workspaces {
                                        if ws.cwd != *cwd {
                                            continue;
                                        }
                                        if ws.git_branch != *branch || ws.is_git_repo != *is_repo {
                                            ws.git_branch = branch.clone();
                                            ws.is_git_repo = *is_repo;
                                            changed = true;
                                        }
                                        if ws.git_stats != *stats {
                                            ws.git_stats = stats.clone();
                                            changed = true;
                                        }
                                    }
                                }
                                if changed {
                                    cx.notify();
                                }
                            })
                        });
                        if apply.is_err() {
                            break;
                        }
                    }
                }
            },
        )
        .detach();

        // Poll IPC requests + config changes every 50ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(50)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            app.process_ipc_requests(cx);
                            app.process_config_changes(cx);
                            app.process_update_check(cx);
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Config hot-reload is now driven by ConfigWatcher (notify crate, 300ms debounce).
        // Changes are picked up in the 50ms IPC poll loop below via process_config_changes().

        // Fallback: poll git metadata for all workspaces every 30s.
        // Primary detection is event-driven (US-003 notify watcher above).
        // This timer catches edge cases where file system events are missed.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(30)).await;

                    // Phase 1: collect CWDs (cheap, main thread)
                    let cwds = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                            app.workspaces
                                .iter()
                                .map(|ws| ws.cwd.clone())
                                .collect::<Vec<String>>()
                        })
                    });
                    let cwds = match cwds {
                        Ok(c) => c,
                        Err(_) => break,
                    };

                    // Phase 2: run git probes off main thread
                    let results = smol::unblock(move || {
                        cwds.into_iter()
                            .map(|cwd| {
                                let (branch, is_repo) = crate::workspace::detect_branch(&cwd);
                                let stats = crate::workspace::GitDiffStats::from_cwd(&cwd);
                                (cwd, branch, is_repo, stats)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;

                    // Phase 3: apply results (cheap, main thread)
                    let apply = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let mut changed = false;
                            for (cwd, branch, is_repo, stats) in &results {
                                for ws in &mut app.workspaces {
                                    if ws.cwd != *cwd {
                                        continue;
                                    }
                                    if ws.git_branch != *branch || ws.is_git_repo != *is_repo {
                                        ws.git_branch = branch.clone();
                                        ws.is_git_repo = *is_repo;
                                        changed = true;
                                    }
                                    if ws.git_stats != *stats {
                                        ws.git_stats = stats.clone();
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                cx.notify();
                            }
                        })
                    });
                    if apply.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Stale PID sweep: every 30s, probe registered AI agent PIDs with
        // kill(pid, 0) to detect crashed processes and clean up sidebar state.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(30)).await;
                    if cx
                        .update(|cx| {
                            this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                                app.sweep_stale_pids(cx);
                            })
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            },
        )
        .detach();

        // Port scanning and CWD detection are now event-driven:
        // - TerminalEvent::ActivityBurst → schedule_port_scan()
        // - TerminalEvent::CwdChanged → handle_cwd_change()
        // See handle_terminal_event() for the push-based implementation.

        // US-008 — classify the install source once, then hand off to the
        // install-method hygiene migrations. Migrations are Linux-only and
        // the module itself is gated behind `#[cfg(target_os = "linux")]`,
        // so the call site needs the matching gate. On macOS / Windows the
        // tar.gz → rpm/deb crossover doesn't exist, so the helper isn't
        // compiled in at all.
        let install_method = update::install_method::detect();
        #[cfg(target_os = "linux")]
        update::migrations::run_startup_migrations(&install_method);

        // US-010/US-012 — resolve the anonymous telemetry_id (creates it
        // on first launch) and build the consent-gated capture client. The
        // `is_first_run` flag is reused below as a property on the
        // `app_started` event; a second filesystem probe would race with the
        // persistence we just did.
        let (telemetry_distinct_id, is_first_run_for_telemetry) =
            telemetry::id::telemetry_id_with_first_run();
        // Compile-time env vars: the PostHog project key is injected by the
        // release pipeline; the host defaults to EU Cloud so a build that
        // omits the override still honours the PRD's EU-residency constraint.
        let posthog_api_key = option_env!("POSTHOG_API_KEY").unwrap_or("");
        let posthog_host = option_env!("POSTHOG_HOST").unwrap_or("https://eu.i.posthog.com");
        let telemetry_config_snapshot = paneflow_config::loader::load_config();
        let telemetry_enabled_last = telemetry_config_snapshot
            .telemetry
            .as_ref()
            .and_then(|t| t.enabled);
        let telemetry = std::sync::Arc::new(telemetry::client::TelemetryClient::from_config(
            &telemetry_config_snapshot,
            posthog_api_key,
            posthog_host,
            &telemetry_distinct_id,
        ));
        // Background flusher: every 5 s the client inspects its queue and
        // posts when the size or age threshold is met. Runs off the GPUI
        // main thread — ureq blocks inside `post_batch` but never on the
        // renderer — via `cx.background_spawn` + `smol::unblock`.
        let telemetry_flusher = std::sync::Arc::clone(&telemetry);
        cx.background_spawn(async move {
            loop {
                smol::Timer::after(std::time::Duration::from_secs(5)).await;
                let client = std::sync::Arc::clone(&telemetry_flusher);
                smol::unblock(move || client.poll_flush()).await;
            }
        })
        .detach();

        // US-009 — coexistence detection + one-time advisory toast. Runs
        // strictly after the US-008 icon migration so a same-session
        // upgrade→cleanup→toast chain stays in order. Detection is always
        // logged (AC: "helper is still called for logging") so duplicate
        // installs remain visible in debug transcripts even after the
        // marker has muted the toast.
        #[cfg(target_os = "linux")]
        if let Some(report) = update::migrations::detect_coexistent_install(&install_method) {
            log::info!(
                "paneflow: coexistent install detected — running from {} (this install); other install at {} (installed via {})",
                report.running_path.display(),
                report.other_path.display(),
                report.other_method_label,
            );
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                let marker_path = update::migrations::coexistence_marker_path(&home);
                if !marker_path.exists() {
                    // Build the toast payload up front so the spawn closure
                    // captures owned strings, not borrowed locals.
                    let message = format!(
                        "Two PaneFlow installs detected. Running from {} (this install); other install at {} (installed via {}). Remove the unused install to avoid version drift.",
                        report.running_path.display(),
                        report.other_path.display(),
                        report.other_method_label,
                    );
                    let actions = vec![crate::ToastAction::OpenReleasesPage(
                        "https://paneflow.dev/download#multiple-installs".to_string(),
                    )];
                    let hold_ms = crate::TOAST_HOLD_MS * 4;
                    // `push_toast` needs `&mut Self` + `&mut Context<Self>`,
                    // but `Self` doesn't exist yet at this point in `new()`.
                    // Defer via `cx.spawn` — the first `Timer::after` yield
                    // lets the ctor finish and hands control back with a
                    // resolvable `WeakEntity<Self>`. Matches the established
                    // spawn pattern in this file (see git-watcher, port-scan,
                    // stale-PID sweep above).
                    cx.spawn(
                        async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                            smol::Timer::after(std::time::Duration::from_millis(1)).await;
                            let pushed = cx
                                .update(|cx| {
                                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                                        app.push_toast(message, actions, hold_ms, cx);
                                    })
                                })
                                .is_ok();
                            // Only persist the marker if the toast actually
                            // went out — a failed update means the app window
                            // is tearing down, in which case letting the toast
                            // recur next session is the right behaviour.
                            if pushed {
                                update::migrations::write_coexistence_marker(&marker_path);
                            }
                        },
                    )
                    .detach();
                }
            }
        }

        let app = Self {
            workspaces,
            active_idx,
            renaming_idx: None,
            rename_text: String::new(),
            pending_config,
            ipc_rx,
            title_bar,
            git_watcher,
            git_event_rx,
            git_watch_counts,
            settings_section: None,
            home_dir: std::env::var("HOME").unwrap_or_default(),
            effective_shortcuts: keybindings::effective_shortcuts(
                &paneflow_config::loader::load_config().shortcuts,
            ),
            recording_shortcut_idx: None,
            settings_focus: cx.focus_handle(),
            mono_font_names: Vec::new(),
            font_dropdown_open: false,
            font_search: String::new(),
            notifications: Vec::new(),
            notif_menu_open: None,
            workspace_menu_open: None,
            title_bar_menu_open: None,
            profile_menu_open: None,
            toast: None,
            _toast_task: None,
            loader_anim_running: false,
            swap_source: None,
            closed_panes: Vec::new(),
            show_about_dialog: false,
            show_theme_picker: false,
            theme_picker_query: String::new(),
            theme_picker_selected_idx: 0,
            theme_picker_focus: cx.focus_handle(),
            pending_update,
            update_status: None,
            self_update_status: update::SelfUpdateStatus::default(),
            install_method,
            update_attempt_count: 0,
            custom_buttons_modal: None,
            custom_buttons_modal_focus: cx.focus_handle(),
            telemetry,
            launch_instant: std::time::Instant::now(),
            telemetry_enabled_last,
        };

        // US-013 AC #1 — fire `app_started` once per launch. `Null` clients
        // (opt-out / unanswered consent / env kill-switch) no-op; only a
        // consenting user produces an HTTP call, batched on the flusher
        // above. Must happen after the struct literal so `self.telemetry`
        // and `self.install_method` are both populated.
        app.emit_app_started(is_first_run_for_telemetry);
        app
    }
}

// ---------------------------------------------------------------------------
// Free helper functions called from `fn main()` (US-002 extraction).
// ---------------------------------------------------------------------------

/// Build the copy-pasteable upgrade command for a system-package install.
///
/// `version` is safe to interpolate into a shell string without escaping: it
/// comes from `UpdateStatus::Available { version }`, which is set from a
/// `semver::Version::to_string()` — the semver parser rejects any input that
/// would survive into `;`/`$()`/whitespace/bidi, so malformed GitHub tags
/// short-circuit to `UpdateStatus::Failed` long before this function runs.
///
/// Version format notes:
/// - apt pinning uses `name=upstream-debrev`. `cargo-deb` emits `-1` as the
///   debian revision by default, so `paneflow=<v>-1` targets the exact tag.
/// - dnf accepts `name-upstream` as a NEVR prefix match. The `<v>` we pass is
///   already the raw upstream version from GitHub Releases.
/// - `PackageManager::Other` gets a plain-English hint rather than a command,
///   because we don't know the syntax (eopkg/xbps/apk all differ).
pub(crate) fn system_package_update_command(
    manager: Option<&update::install_method::PackageManager>,
    version: &str,
) -> String {
    match manager {
        Some(update::install_method::PackageManager::Apt) => {
            format!("sudo apt update && sudo apt install paneflow={version}-1")
        }
        Some(update::install_method::PackageManager::Dnf) => {
            format!("sudo dnf upgrade paneflow-{version}")
        }
        // US-004: `rpm-ostree upgrade` takes no package argument — it
        // rebases the whole deployment. Version string is intentionally
        // NOT included, unlike the apt/dnf arms.
        Some(update::install_method::PackageManager::RpmOstree) => "rpm-ostree upgrade".to_string(),
        Some(update::install_method::PackageManager::Other) | None => {
            "Update PaneFlow via your system's package manager".to_string()
        }
    }
}

/// Install the macOS menu bar.
///
/// US-012: three top-level menus — PaneFlow / Edit / Window — populated with
/// the actions listed in the PRD. The `PaneFlow` menu name matches the
/// `CFBundleName` from the future US-013 Info.plist (AC6). Keyboard shortcuts
/// are derived from the global keybindings table (e.g. Quit shows `⌘Q`
/// because US-010's `MACOS_ONLY_DEFAULTS` binds `cmd-q → quit`; Window items
/// show `⌘⇧N` / `⌘⇧Q` / `⌘Tab` from US-009's `secondary-*` bindings).
/// Copy / Paste / Select All carry an `OsAction` hint so macOS routes them
/// through the native responder chain and renders `⌘C` / `⌘V` / `⌘A`.
#[cfg(target_os = "macos")]
pub(crate) fn install_macos_menu_bar(cx: &mut gpui::App) {
    use gpui::{Menu, MenuItem, OsAction};

    use crate::{
        About, CloseWorkspace, Copy, NewWorkspace, NextWorkspace, OpenHelp, Paste, Quit, SelectAll,
    };

    cx.set_menus(vec![
        Menu::new("PaneFlow").items(vec![
            MenuItem::action("About PaneFlow", About),
            MenuItem::separator(),
            MenuItem::action("Quit PaneFlow", Quit),
        ]),
        Menu::new("Edit").items(vec![
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("Window").items(vec![
            MenuItem::action("New Workspace", NewWorkspace),
            MenuItem::action("Close Workspace", CloseWorkspace),
            MenuItem::separator(),
            MenuItem::action("Next Workspace", NextWorkspace),
        ]),
        // macOS convention: every app ships a Help menu (even if it only
        // points to an online doc/repo). Without one, Apple's HIG-conforming
        // users perceive the app as unfinished. "PaneFlow Help" dispatches
        // `OpenHelp` which opens the GitHub README in the default browser.
        Menu::new("Help").items(vec![MenuItem::action("PaneFlow Help", OpenHelp)]),
    ]);
}

/// Detect whether the Apple Silicon binary is running under Rosetta 2
/// translation on an Intel Mac (or, more commonly, an Intel binary on
/// Apple Silicon — which Apple translates transparently). Either way it
/// warns once at startup so a user who grabbed the wrong `.dmg` knows
/// why GPU performance is degraded instead of silently eating the hit.
///
/// Edge case 4 of the macOS port PRD. Uses `sysctl.proc_translated`: returns
/// `1` for a translated process, `0` native, ENOENT → native Intel kernel
/// (no Rosetta available at all). Failure to read the sysctl is silent —
/// this warning is diagnostic, not load-bearing.
#[cfg(target_os = "macos")]
pub(crate) fn warn_if_rosetta_translated() {
    use std::ffi::CString;
    use std::mem::size_of;

    let name = match CString::new("sysctl.proc_translated") {
        Ok(n) => n,
        Err(_) => return,
    };
    let mut translated: i32 = 0;
    let mut size = size_of::<i32>();
    // SAFETY: `sysctlbyname` reads a small integer into a stack buffer whose
    // size is passed by pointer. `name.as_ptr()` is a valid NUL-terminated
    // C string from a CString we just constructed. `translated` and `size`
    // are live stack variables for the duration of the call. Zero-initialized
    // buffer means a kernel short-write can't expose uninitialized memory.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut translated as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && translated == 1 {
        log::warn!(
            "running under Rosetta 2 translation — GPU rendering will be \
             degraded. For best performance, download the matching \
             architecture from https://github.com/ArthurDEV44/paneflow/releases"
        );
    }
}

/// The old `.run` installer (removed in US-007) dropped a standalone binary
/// at `~/.local/bin/paneflow`. The new tar.gz installer instead drops a
/// `~/.local/paneflow.app/` directory and symlinks `~/.local/bin/paneflow`
/// into it. We warn when the old layout is detected so users know why the
/// in-app updater can no longer fetch a `.run` asset (there are none).
pub(crate) fn warn_if_legacy_run_install() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let app_dir = home.join(".local/paneflow.app");
    let legacy_bin = home.join(".local/bin/paneflow");

    let legacy_bin_is_regular_file = legacy_bin
        .symlink_metadata()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false);

    if !app_dir.exists() && legacy_bin_is_regular_file {
        log::warn!(
            "legacy .run install detected at {} — see README for migration \
             to the .tar.gz / .deb / .AppImage formats",
            legacy_bin.display()
        );
    }
}
