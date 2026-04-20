//! In-app self-update dispatcher — routes clicks on the update pill to the
//! right installer branch (SystemPackage / AppImage / TarGz+Unknown / legacy
//! `.run`) based on the detected [`crate::update::install_method::InstallMethod`].
//!
//! Extracted from `main.rs` per US-028 of the src-app refactor PRD.

use gpui::{ClipboardItem, Context, Window};

use crate::{
    PaneFlowApp, StartSelfUpdate, TOAST_HOLD_MS, ToastAction, system_package_update_command, update,
};

impl PaneFlowApp {
    /// Kick off the in-app self-update flow. See the module-level doc for the
    /// branch matrix; on any failure a toast surfaces and the update pill
    /// returns to "Update failed".
    pub(crate) fn handle_start_self_update(
        &mut self,
        _: &StartSelfUpdate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.self_update_status.is_busy() {
            return;
        }

        // System-package installs (.deb/.rpm) are upgraded by apt/dnf, never
        // by the in-app updater — writing to `/usr/bin/paneflow` would fail
        // unprivileged and break immutable distros. Show the copy-pasteable
        // upgrade command instead. Crucially: return BEFORE reading
        // `asset_url`, so no network activity happens on click.
        if let update::install_method::InstallMethod::SystemPackage { manager } =
            &self.install_method
        {
            let version = match &self.update_status {
                Some(update::checker::UpdateStatus::Available { version, .. }) => version.clone(),
                _ => return,
            };
            let command = system_package_update_command(Some(manager), &version);
            cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
            self.show_toast(format!("Copied: {command}"), cx);
            return;
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
                        let _ = this.update(cx, |app, cx| {
                            app.save_session(cx);
                        });
                        cx.update(|cx| {
                            log::info!("self-update: restarting into {}", updated_path.display());
                            cx.set_restart_path(updated_path);
                            cx.restart();
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
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::Installing;
                            app.save_session(cx);
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/targz: restarting into {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                            cx.restart();
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
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::Installing;
                            app.save_session(cx);
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/msi: restarting into {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                            cx.restart();
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
                        let _ = this.update(cx, |app, cx| {
                            app.self_update_status = update::SelfUpdateStatus::Installing;
                            app.save_session(cx);
                            cx.notify();
                        });
                        cx.update(|cx| {
                            log::info!(
                                "self-update/dmg: restarting into {}",
                                restart_path.display()
                            );
                            cx.set_restart_path(restart_path);
                            cx.restart();
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

                // Persist session, register the new binary path with GPUI, and
                // trigger the launcher-based restart. GPUI's Linux platform
                // spawns a detached bash script that waits for our PID to exit
                // before exec'ing the new binary.
                let _ = this.update(cx, |app, cx| {
                    app.save_session(cx);
                });
                cx.update(|cx| match update::installed_binary_path() {
                    Ok(path) => {
                        log::info!("self-update: restarting into {}", path.display());
                        cx.set_restart_path(path);
                        cx.restart();
                    }
                    Err(e) => log::error!("self-update: cannot resolve install path: {e}"),
                });
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
}
