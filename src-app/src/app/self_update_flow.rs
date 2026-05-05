//! In-app self-update dispatcher — routes clicks on the update pill to the
//! right installer branch (SystemPackage / AppImage / TarGz+Unknown / legacy
//! `.run`) based on the detected [`crate::update::install_method::InstallMethod`].
//!
//! Extracted from `main.rs` per US-028 of the src-app refactor PRD.

use gpui::{ClipboardItem, Context, Window};

use crate::app::telemetry_events::UpdateDismissReason;
use crate::{
    DismissUpdate, PaneFlowApp, StartSelfUpdate, TOAST_HOLD_MS, ToastAction,
    system_package_update_command, update,
};

/// One-line summary of the install method for log messages — used by the
/// auto-kickoff gate to keep diagnostic noise low when the running binary
/// is not auto-updatable.
fn install_method_label(method: &update::install_method::InstallMethod) -> &'static str {
    match method {
        update::install_method::InstallMethod::AppImage { .. } => "appimage",
        update::install_method::InstallMethod::TarGz { .. } => "targz",
        update::install_method::InstallMethod::AppBundle { .. } => "app-bundle",
        update::install_method::InstallMethod::WindowsMsi { .. } => "windows-msi",
        update::install_method::InstallMethod::SystemPackage { .. } => "system-package",
        update::install_method::InstallMethod::ExternallyManaged { .. } => "externally-managed",
        update::install_method::InstallMethod::Unknown => "unknown",
    }
}

/// Strict-semver guard for the release tag before it reaches any
/// user-facing surface (clipboard, toast, argv). Matches the regex
/// `^v?\d+\.\d+\.\d+$` — identical to the validator inside
/// `update::linux::system_package::validate_version`, inlined here so
/// the check runs even on code paths that bypass `run_update`
/// (`PackageManager::Other` clipboard fallback, non-Linux targets,
/// and the `EnvironmentBroken` clipboard fallback). Keeping the rule
/// in two places is a deliberate trade for keeping `validate_version`
/// private to its Linux-only module.
fn is_strict_semver(raw: &str) -> bool {
    let rest = raw.strip_prefix('v').unwrap_or(raw);
    let mut completed_parts: usize = 0;
    let mut segment_len: usize = 0;
    for ch in rest.chars() {
        match ch {
            '0'..='9' => segment_len = segment_len.saturating_add(1),
            '.' => {
                if segment_len == 0 {
                    return false;
                }
                completed_parts = completed_parts.saturating_add(1);
                segment_len = 0;
            }
            _ => return false,
        }
    }
    if segment_len == 0 {
        return false;
    }
    completed_parts.saturating_add(1) == 3
}

impl PaneFlowApp {
    /// Action entry point. Stays a thin wrapper around
    /// [`PaneFlowApp::kickoff_self_update_install`] so that auto-kickoff
    /// from the polling loop can share the exact same logic without
    /// having to forge a `Window`.
    pub(crate) fn handle_start_self_update(
        &mut self,
        _: &StartSelfUpdate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kickoff_self_update_install(cx);
    }

    /// US-007 AC3: dismiss the update pill for the current launch.
    /// Clears `update_status` so the title-bar pill disappears, fires
    /// a `update_dismissed` PostHog event, and forces a re-render.
    /// Intentionally NOT persisted — the next paneflow launch will
    /// re-detect the update and re-show the pill (we don't want a
    /// user accidentally sticking on an old version because the
    /// preference outlived their interest).
    pub(crate) fn handle_dismiss_update(
        &mut self,
        _: &DismissUpdate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Capture the to_version BEFORE we drop update_status so the
        // emit helper can reference it.
        self.emit_update_dismissed(UpdateDismissReason::UserDismissed);
        self.update_status = None;
        cx.notify();
    }

    /// Kick off the in-app self-update flow. See the module-level doc for the
    /// branch matrix; on any failure a toast surfaces and the update pill
    /// returns to "Update failed".
    pub(crate) fn kickoff_self_update_install(&mut self, cx: &mut Context<Self>) {
        // Fast path: the new binary is already on disk and
        // `set_restart_path` has been wired ahead of time by the
        // background pre-installer (see `try_auto_kickoff_install`
        // below). The click handler does ZERO I/O — just hand control
        // to GPUI's relauncher script. This is what makes the
        // user-perceived restart latency drop from "vachement long"
        // (download + install + analytics flush) to GPUI's
        // ~100 ms `kill -0` polling interval.
        if matches!(
            self.self_update_status,
            update::SelfUpdateStatus::ReadyToRestart { .. }
        ) {
            log::info!("self-update: ReadyToRestart click — invoking cx.restart()");
            cx.restart();
            return;
        }

        // Externally managed runtime (Flatpak / Snap / packager-baked
        // `PANEFLOW_UPDATE_EXPLANATION`). The in-app updater is disabled
        // by design — surface the packager's explanation copy and copy
        // the upgrade command to the clipboard so the user has a one-click
        // path forward. Mirrors how Zed handles `ZED_UPDATE_EXPLANATION`.
        if let update::install_method::InstallMethod::ExternallyManaged { explanation } =
            &self.install_method
        {
            cx.write_to_clipboard(ClipboardItem::new_string(explanation.clone()));
            self.push_toast(explanation.clone(), Vec::new(), TOAST_HOLD_MS * 4, cx);
            return;
        }

        if self.self_update_status.is_busy() {
            return;
        }

        // System-package installs (.deb/.rpm). Fedora / Ubuntu / openSUSE
        // / Debian users on the signed pkg.paneflow.dev repo get an
        // in-app pkexec-elevated `dnf|apt-get install` (US-002). Solus /
        // Void / NixOS et al. fall back to the clipboard-copy flow so
        // they at least see a runnable upgrade command. `return`s
        // BEFORE reading `asset_url` below — the pkexec flow pulls its
        // payload from the system repo; no direct GitHub download.
        //
        // Note: `InstallMethod::SystemPackage` is declared unconditionally
        // (not `#[cfg]`-gated) so this `if let` must compile on every
        // target. The pkexec call below is Linux-only; non-Linux targets
        // route through the clipboard-copy fallback. In practice
        // `install_method::detect()` only produces `SystemPackage` on
        // Linux, so the non-Linux path is compile-only ballast.
        if let update::install_method::InstallMethod::SystemPackage { manager } =
            &self.install_method
        {
            let version = match &self.update_status {
                Some(update::checker::UpdateStatus::Available { version, .. }) => version.clone(),
                _ => return,
            };

            // Defence in depth: reject any version that is not strict
            // semver BEFORE formatting it into a clipboard string, a
            // toast, or argv. US-001 already regex-validates inside
            // `run_update`, but the `PackageManager::Other` branch and
            // the `EnvironmentBroken` fallback both construct a
            // user-visible "Copied: sudo apt install paneflow=<ver>"
            // toast WITHOUT going through `run_update` first. A
            // compromised GitHub tag (e.g. `0.2.3; rm -rf $HOME`)
            // would otherwise end up in the user's clipboard verbatim.
            // The three-dot-decimal grammar matches
            // `system_package::validate_version`.
            if !is_strict_semver(&version) {
                log::warn!(
                    "self-update/system-package: refusing malformed version string: {version:?}"
                );
                self.show_toast("Update unavailable — invalid release tag".to_string(), cx);
                return;
            }

            // US-004: rpm-ostree (Silverblue / Kinoite / Bazzite).
            // Immutable distros stage updates offline for the next
            // reboot — pkexec+dnf would fail against the read-only
            // `/usr`. Surface a dedicated informational toast and
            // copy `rpm-ostree upgrade` to the clipboard. No
            // subprocess spawn, no `cx.restart()` — the update does
            // not take effect until the user reboots.
            if matches!(manager, update::install_method::PackageManager::RpmOstree) {
                cx.write_to_clipboard(ClipboardItem::new_string("rpm-ostree upgrade".to_string()));
                // Long-form informational copy; use `push_toast`
                // with 4× hold so the user has time to read it
                // (default TOAST_HOLD_MS is tuned for short
                // "Copied: …" confirmations).
                self.push_toast(
                    "PaneFlow detects an immutable distribution. Update must be run via `rpm-ostree upgrade` at the system level — the update has been copied to your clipboard.".to_string(),
                    Vec::new(),
                    TOAST_HOLD_MS * 4,
                    cx,
                );
                return;
            }

            // PackageManager::Other (Solus, Void, NixOS, …): no reliable
            // repo from our side → keep the clipboard-copy behaviour.
            // Same code path as pre-US-002. Also used as the
            // compile-only fallback on macOS / Windows for the
            // (unreachable-at-runtime) Dnf / Apt variants.
            //
            // `RpmOstree` is intentionally absent from the whitelist
            // below — Silverblue / Kinoite users are already served
            // by the dedicated informational arm above, which always
            // `return`s. If a future refactor removes that early
            // return, `RpmOstree` would fall through to the generic
            // clipboard-copy path (safe but wrong copy — never to
            // pkexec, because the whitelist excludes it).
            #[cfg(not(target_os = "linux"))]
            let run_pkexec = false;
            #[cfg(target_os = "linux")]
            let run_pkexec = matches!(
                manager,
                update::install_method::PackageManager::Dnf
                    | update::install_method::PackageManager::Apt
            );

            if !run_pkexec {
                let command = system_package_update_command(Some(manager), &version);
                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                self.show_toast(format!("Copied: {command}"), cx);
                return;
            }

            // Dnf / Apt on Linux: full pkexec flow, matching the
            // AppImage / TarGz one-click UX. Status transitions:
            // Idle → Downloading → (on Ok) Installing → save_session →
            // set_restart_path → restart.
            #[cfg(target_os = "linux")]
            {
                let manager_owned = manager.clone();
                let manager_label: &'static str = match manager_owned {
                    update::install_method::PackageManager::Dnf => "dnf",
                    update::install_method::PackageManager::Apt => "apt",
                    // Other / RpmOstree are short-circuited above via
                    // the rpm-ostree informational arm and the
                    // `run_pkexec` gate; these arms exist purely for
                    // compile-time exhaustiveness.
                    update::install_method::PackageManager::Other => "system-package",
                    update::install_method::PackageManager::RpmOstree => "rpm-ostree",
                };
                self.self_update_status = update::SelfUpdateStatus::Downloading;
                cx.notify();

                cx.spawn(async move |this, cx| {
                    let result = smol::unblock({
                        // Clone into the worker task; `manager_owned`
                        // (outer) stays in scope for the
                        // `EnvironmentBroken` clipboard fallback below.
                        let manager_for_worker = manager_owned.clone();
                        let version_for_worker = version.clone();
                        move || {
                            update::linux::system_package::run_update(
                                &manager_for_worker,
                                &version_for_worker,
                            )
                        }
                    })
                    .await;

                    match result {
                        Ok(()) => {
                            let restart_path = std::path::PathBuf::from("/usr/bin/paneflow");
                            let restart_path_for_state = restart_path.clone();
                            let _ = this.update(cx, |app, cx| {
                                // Pre-installed: flip to ReadyToRestart so the
                                // pill becomes a one-call restart button. The
                                // analytics flush + session save still run
                                // here (now, while the user is busy), not at
                                // click time.
                                app.self_update_status =
                                    update::SelfUpdateStatus::ReadyToRestart {
                                        restart_path: restart_path_for_state,
                                    };
                                app.save_session(cx);
                                app.emit_update_success_and_flush();
                                cx.notify();
                            });
                            cx.update(|cx| {
                                log::info!(
                                    "self-update/{manager_label}: pre-installed — restart pending at /usr/bin/paneflow"
                                );
                                cx.set_restart_path(restart_path);
                            });
                        }
                        Err(err) => {
                            // Classify once on the async side; the
                            // closure below only decides which state
                            // transition + toast copy to run on the
                            // main thread.
                            let classified = update::UpdateError::classify(&err);
                            let _ = this.update(cx, |app, cx| match classified {
                                // Polkit "Cancel" — benign. Revert to
                                // Idle, neutral toast, DO NOT bump the
                                // retry counter (user intent, not a
                                // failure).
                                update::UpdateError::InstallDeclined { .. } => {
                                    app.self_update_status = update::SelfUpdateStatus::Idle;
                                    app.show_toast("Update cancelled".to_string(), cx);
                                    cx.notify();
                                }
                                // pkexec missing / no polkit agent /
                                // exit 127 — fall back to the
                                // clipboard-copy behaviour so the user
                                // has a runnable command. No retry
                                // bump (transient env issue, not a
                                // package-mgr failure).
                                update::UpdateError::EnvironmentBroken { .. } => {
                                    let command = system_package_update_command(
                                        Some(&manager_owned),
                                        &version,
                                    );
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        command.clone(),
                                    ));
                                    app.self_update_status = update::SelfUpdateStatus::Idle;
                                    app.show_toast(format!("Copied: {command}"), cx);
                                    cx.notify();
                                }
                                // US-005: backpressure — `dnf-automatic`
                                // or an interactive `sudo apt install`
                                // held the package-manager lock at
                                // pre-flight time. Transient condition
                                // (user can retry in a moment), NOT a
                                // real failure, so skip the 3-strikes
                                // counter and show a neutral toast.
                                // Match on the exact sentinel emitted
                                // by `run_update` so brittle substring
                                // matching is avoided.
                                update::UpdateError::Other(ref msg)
                                    if msg == update::linux::system_package::BUSY_MESSAGE =>
                                {
                                    app.self_update_status = update::SelfUpdateStatus::Idle;
                                    app.push_toast(
                                        update::linux::system_package::BUSY_MESSAGE.to_string(),
                                        Vec::new(),
                                        TOAST_HOLD_MS * 2,
                                        cx,
                                    );
                                    cx.notify();
                                }
                                // Anything else (mirror 5xx, disk full,
                                // signal, transaction conflict) — real
                                // failure; feed the 3-strikes counter
                                // via record_update_failure.
                                _ => {
                                    app.record_update_failure(manager_label, &err, cx);
                                }
                            });
                        }
                    }
                })
                .detach();
                return;
            }
        }

        // After 3 consecutive failures, the 4th click stops re-trying and
        // points the user at the releases page (US-013). Skipping the
        // network here is important — repeated fast retries against a
        // flaky mirror are never the right answer.
        if self.update_attempt_count >= 3 {
            let releases_url = match &self.update_status {
                Some(update::checker::UpdateStatus::Available { url, .. }) => url.clone(),
                _ => "https://github.com/ArthurDEV44/paneflow/releases".to_string(),
            };
            self.push_toast(
                "Update keeps failing. Download manually from the releases page.".to_string(),
                vec![ToastAction::OpenReleasesPage(releases_url)],
                TOAST_HOLD_MS * 4,
                cx,
            );
            return;
        }

        let asset_url = match &self.update_status {
            Some(update::checker::UpdateStatus::Available {
                asset_url: Some(url),
                ..
            }) => url.clone(),
            Some(update::checker::UpdateStatus::Available { url, .. }) => {
                // No Linux asset on this release (edge case: draft, mis-tagged).
                // Fall back to opening the release page so the user can grab it.
                let _ = open::that(url);
                return;
            }
            _ => return,
        };

        // Use the cached install method. The install location never changes
        // at runtime, so one probe at startup is enough.
        let method = self.install_method.clone();
        if let update::install_method::InstallMethod::AppImage { source_path, .. } = &method {
            let source_path = source_path.clone();
            // `appimageupdatetool` does one opaque call that covers both the
            // zsync download and the in-place rewrite. Most of the
            // wall-clock time is spent fetching delta blocks, so `Downloading`
            // matches what the user actually sees on a slow link.
            self.self_update_status = update::SelfUpdateStatus::Downloading;
            cx.notify();

            cx.spawn(async move |this, cx| {
                let result = smol::unblock({
                    let source_path = source_path.clone();
                    move || update::linux::appimage::run_update(&source_path)
                })
                .await;

                match result {
                    Ok(updated_path) => {
                        let restart_path_for_state = updated_path.clone();
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::ReadyToRestart {
                                restart_path: restart_path_for_state,
                            };
                            app.save_session(cx);
                            app.emit_update_success_and_flush();
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/appimage: pre-installed — restart pending at {}",
                                updated_path.display()
                            );
                            cx.set_restart_path(updated_path);
                        });
                    }
                    Err(err) => {
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("appimage", &err, cx);
                        });
                    }
                }
            })
            .detach();
            return;
        }

        if matches!(
            &method,
            update::install_method::InstallMethod::TarGz { .. }
                | update::install_method::InstallMethod::Unknown
        ) {
            // Atomic directory swap under `$HOME/.local/paneflow.app/`.
            // `run_update` derives the target paths from `$HOME` internally,
            // so we only need to hand it the release asset URL.
            //
            // `Unknown` (dev builds, legacy `.run` migrations) dispatches here
            // too: `pick_asset` already returns a `.tar.gz` URL for Unknown
            // (see `AssetFormat::from_install_method`), and since v0.2.0 no
            // longer emits `.run` assets, falling through to the legacy
            // installer below would try to `chmod +x` + execve a gzip file.
            //
            // Log the migration path so dev-build users (who hit the Unknown
            // branch after `cargo run`) see what's happening. The updater
            // still proceeds — the install lands at `$HOME/.local/paneflow.app/`
            // regardless of where `current_exe()` was — but the log makes the
            // directory change visible instead of silent.
            if matches!(&method, update::install_method::InstallMethod::Unknown) {
                log::warn!(
                    "self-update: install method Unknown — downloading tar.gz release \
                     into $HOME/.local/paneflow.app/; the updated binary will be at a \
                     different path than the currently-running one."
                );
            }
            let url = asset_url.clone();
            self.self_update_status = update::SelfUpdateStatus::Downloading;
            cx.notify();

            cx.spawn(async move |this, cx| {
                let result = smol::unblock(move || update::linux::targz::run_update(&url)).await;

                match result {
                    Ok(restart_path) => {
                        let restart_path_for_state = restart_path.clone();
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::ReadyToRestart {
                                restart_path: restart_path_for_state,
                            };
                            app.save_session(cx);
                            app.emit_update_success_and_flush();
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/targz: pre-installed — restart pending at {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                        });
                    }
                    Err(err) => {
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("targz", &err, cx);
                        });
                    }
                }
            })
            .detach();
            return;
        }

        // US-010: Windows MSI install — download, SHA-verify, invoke
        // msiexec, map exit codes. `InstallMethod::WindowsMsi` is only
        // produced on Windows by install_method::detect(), so on
        // Linux/macOS this branch is a runtime-dead `if let` — the
        // `msiexec.exe` lookup inside `msi::install` would otherwise
        // fail there, but the branch guard prevents ever reaching it.
        if let update::install_method::InstallMethod::WindowsMsi { .. } = &method {
            let url = asset_url.clone();
            self.self_update_status = update::SelfUpdateStatus::Downloading;
            cx.notify();

            cx.spawn(async move |this, cx| {
                let result = smol::unblock(move || update::windows::msi::install(&url)).await;
                match result {
                    Ok(restart_path) => {
                        let restart_path_for_state = restart_path.clone();
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::ReadyToRestart {
                                restart_path: restart_path_for_state,
                            };
                            app.save_session(cx);
                            app.emit_update_success_and_flush();
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/msi: pre-installed — restart pending at {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                        });
                    }
                    Err(err) => {
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("msi", &err, cx);
                        });
                    }
                }
            })
            .detach();
            return;
        }

        // US-009: macOS `.app` bundle — mount the DMG, swap bundle
        // atomically, restart into the new `Contents/MacOS/paneflow`.
        // Dispatch is an `if let` (not a cfg guard) so the code remains
        // a single compile-closure across all targets; the
        // `InstallMethod::AppBundle` variant is only produced on macOS
        // by `install_method::detect()`, so on Linux / Windows this
        // branch is runtime-dead without needing a `#[cfg(target_os)]`.
        if let update::install_method::InstallMethod::AppBundle { .. } = &method {
            let url = asset_url.clone();
            self.self_update_status = update::SelfUpdateStatus::Downloading;
            cx.notify();

            cx.spawn(async move |this, cx| {
                let result = smol::unblock(move || update::macos::dmg::install(&url)).await;
                match result {
                    Ok(restart_path) => {
                        let restart_path_for_state = restart_path.clone();
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::ReadyToRestart {
                                restart_path: restart_path_for_state,
                            };
                            app.save_session(cx);
                            app.emit_update_success_and_flush();
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/dmg: pre-installed — restart pending at {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                        });
                    }
                    Err(err) => {
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("dmg", &err, cx);
                        });
                    }
                }
            })
            .detach();
            return;
        }

        // US-008: the legacy `.run` fall-through is Unix-only. On Linux,
        // this branch is runtime-dead for the already-handled install
        // methods above (AppImage / TarGz / SystemPackage) plus Unknown,
        // and reachable only for older dev builds that slipped past the
        // `TarGz | Unknown` match. On Windows/macOS the branch is
        // cfg-eliminated at compile time — those platforms route via
        // `InstallMethod::WindowsMsi` (US-010) and `AppBundle` (US-009)
        // respectively, and the fall-through below must never be reached.
        //
        // If US-009/US-010 land before those dispatch arms are fully
        // wired, the `#[cfg(not(unix))]` sibling below records a
        // deliberate error-toast rather than bubbling up a mysterious
        // "no updater wired" runtime failure.
        #[cfg(unix)]
        {
            self.self_update_status = update::SelfUpdateStatus::Downloading;
            cx.notify();

            cx.spawn(async move |this, cx| {
                // Download off the GPUI main thread so the UI stays responsive.
                let download_result = smol::unblock({
                    let url = asset_url.clone();
                    move || update::download_installer(&url)
                })
                .await;

                let installer_path = match download_result {
                    Ok(path) => path,
                    Err(err) => {
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("legacy-download", &err, cx);
                        });
                        return;
                    }
                };

                let _ = this.update(cx, |app, cx| {
                    app.self_update_status = update::SelfUpdateStatus::Installing;
                    cx.notify();
                });

                let install_result = smol::unblock({
                    let path = installer_path.clone();
                    move || update::run_installer(&path)
                })
                .await;

                if let Err(err) = install_result {
                    let _ = this.update(cx, |app, cx| {
                        app.record_update_failure("legacy-install", &err, cx);
                    });
                    return;
                }

                // Persist session and pre-wire the relauncher with the new
                // binary path. The actual `cx.restart()` happens on the
                // user's next click (now reduced to a one-call no-I/O
                // operation) — see the ReadyToRestart short-circuit at
                // the top of `handle_start_self_update`.
                match update::installed_binary_path() {
                    Ok(path) => {
                        let path_for_state = path.clone();
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::ReadyToRestart {
                                restart_path: path_for_state,
                            };
                            app.save_session(cx);
                            app.emit_update_success_and_flush();
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/legacy: pre-installed — restart pending at {}",
                                path.display()
                            );
                            cx.set_restart_path(path);
                        });
                    }
                    Err(e) => {
                        log::error!("self-update: cannot resolve install path: {e}");
                        let _ = this.update(cx, |app, cx| {
                            app.record_update_failure("legacy-dispatch", &e, cx);
                        });
                    }
                }
            })
            .detach();
        }

        // US-008: non-Unix fall-through. Reached only when the caller is
        // running on macOS (`InstallMethod::AppBundle`) or Windows
        // (`InstallMethod::WindowsMsi`) AND the platform-specific updater
        // story (US-009 / US-010) has not yet landed. Surfaces a toast
        // instead of silently attempting the legacy `.run` flow, which on
        // these platforms would download an MSI/DMG and attempt to
        // `chmod +x`/execve it. The `asset_url` binding is consumed here
        // via the error message so it's not flagged as unused.
        #[cfg(not(unix))]
        {
            let msg = anyhow::anyhow!(
                "Self-update for this platform is not yet available. Download the new \
                 release manually from {asset_url}"
            );
            self.record_update_failure("legacy-dispatch", &msg, cx);
        }
    }

    /// Best-effort background pre-install. Called once per polling cycle
    /// after `update_status` transitions to `Available`. By the time
    /// the user actually clicks the pill, the new binary is already on
    /// disk and `set_restart_path` is wired — `cx.restart()` is the
    /// only thing left to do, dropping click→restart latency from
    /// download-time + 2 s analytics flush to GPUI's `kill -0` watcher
    /// interval (~100 ms). Mirrors Zed's silent auto-update worker
    /// (`crates/auto_update/src/auto_update.rs::poll`).
    ///
    /// Gating, in order:
    /// - `update_status` is `Available`.
    /// - `self_update_status` is `Idle` — never re-kick a flow that's
    ///   already downloading, installed, or errored.
    /// - `update_attempt_count < 3` — reuse the 3-strikes circuit
    ///   breaker so a flaky mirror doesn't burn user bandwidth every
    ///   poll cycle.
    /// - `install_method` is auto-installable (AppImage / TarGz /
    ///   AppBundle / WindowsMsi / Unknown). SystemPackage needs
    ///   pkexec (interactive auth — never auto), ExternallyManaged
    ///   defers to the host package manager.
    pub(crate) fn try_auto_kickoff_install(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.update_status,
            Some(update::checker::UpdateStatus::Available { .. })
        ) {
            return;
        }
        if !matches!(self.self_update_status, update::SelfUpdateStatus::Idle) {
            return;
        }
        if self.update_attempt_count >= 3 {
            return;
        }
        let auto_eligible = matches!(
            self.install_method,
            update::install_method::InstallMethod::AppImage { .. }
                | update::install_method::InstallMethod::TarGz { .. }
                | update::install_method::InstallMethod::AppBundle { .. }
                | update::install_method::InstallMethod::WindowsMsi { .. }
                | update::install_method::InstallMethod::Unknown
        );
        if !auto_eligible {
            log::debug!(
                "self-update/auto-kickoff: skipped (install_method={})",
                install_method_label(&self.install_method)
            );
            return;
        }

        log::info!(
            "self-update/auto-kickoff: starting background pre-install (install_method={})",
            install_method_label(&self.install_method)
        );
        self.kickoff_self_update_install(cx);
    }
}
