# Spike: macOS Build Validation (US-009)

**Date:** 2026-04-14
**Status:** Partial — code changes applied, awaiting macOS hardware validation

## Summary

Audited PaneFlow's codebase for Linux-specific constructs that would prevent compilation on macOS. Found one true compile blocker and one functional issue. Both resolved. Cross-compilation from Linux is not viable — macOS hardware or CI runner required for full validation.

## Findings & Resolutions

### 1. BLOCKER — `gpui_platform` wayland/x11 features (RESOLVED)

**File:** `src-app/Cargo.toml:14`

The `gpui_platform` dependency unconditionally enabled `wayland` and `x11` features, which forward to `gpui_linux/wayland` and `gpui_linux/x11`. These pull in Linux-only system libraries (`libwayland-client`, `libxcb`, etc.) that do not exist on macOS.

**Fix:** Split into a base dependency (all platforms) and a target-specific dependency that adds `wayland`/`x11` features only on Linux:

```toml
[dependencies]
gpui_platform = { git = "...", rev = "..." }

[target.'cfg(target_os = "linux")'.dependencies]
gpui_platform = { git = "...", rev = "...", features = ["wayland", "x11"] }
```

On macOS, `gpui_platform` automatically pulls in `gpui_macos` via its own `[target.'cfg(target_os = "macos")'.dependencies]` — no feature flag needed.

### 2. FUNCTIONAL — `/proc/<pid>/cwd` Linux-only path (RESOLVED)

**File:** `src-app/src/terminal.rs:378-381`

The `cwd_now()` method reads `/proc/<pid>/cwd` to detect the shell's working directory. `/proc` is a Linux-only virtual filesystem. On macOS this silently returns `None` (via `.ok()`), so no crash, but the function is semantically broken.

**Fix:** Gated behind `#[cfg(target_os = "linux")]` with a `#[cfg(not(target_os = "linux"))]` stub returning `None`. Future macOS implementation should use `proc_pidinfo()` with `PROC_PIDVNODEPATHINFO`.

### 3. NO ISSUE — Unix socket IPC

**File:** `src-app/src/ipc.rs:7-8, 62`

`std::os::unix::net::{UnixListener, UnixStream}` and `PermissionsExt::from_mode(0o600)` — macOS is Unix, these compile and work correctly. No change needed.

### 4. NO ISSUE — `libc::kill` / `libc::ESRCH`

**File:** `src-app/src/main.rs:386-389`

Standard POSIX APIs available on macOS via the `libc` crate. No change needed.

### 5. BEHAVIORAL DIFFERENCE — `XDG_RUNTIME_DIR` socket path

**Files:** `src-app/src/ipc.rs:187-191`, `src-app/src/terminal.rs:439-458`

`XDG_RUNTIME_DIR` is a Linux XDG spec variable. On macOS it is unset. The code already falls back to `dirs::runtime_dir()`, which returns `$TMPDIR` on macOS. The IPC socket will land in a temp directory rather than `/run/user/<uid>`. Functional but different behavior. No code change needed — documenting for awareness.

### 6. FUNCTIONAL — `/proc/` port detection in `workspace.rs` (RESOLVED)

**File:** `src-app/src/workspace.rs:174, 207, 258`

Three functions read Linux-only `/proc/` paths for port detection:
- `collect_descendant_pids()` reads `/proc/{pid}/task/{pid}/children`
- `detect_ports()` reads `/proc/net/tcp` and `/proc/net/tcp6`
- `collect_socket_inodes()` reads `/proc/{pid}/fd/`

On macOS these silently return empty results (file reads fail gracefully). However, this means port detection is non-functional on macOS.

**Fix:** Gated all three functions behind `#[cfg(target_os = "linux")]`. Added a `#[cfg(not(target_os = "linux"))]` stub for the public `detect_ports()` returning `vec![]`. Future macOS implementation could use `lsof -i -P` or `libproc` bindings.

### 7. NO ISSUE — `portable-pty`

Cross-platform by design (US-007). Uses `openpty` on Unix (Linux + macOS) and ConPTY on Windows. No change needed.

### 8. NO ISSUE — `notify` crate

Uses OS-native file watchers: inotify on Linux, FSEvents on macOS, ReadDirectoryChanges on Windows. No change needed.

## GPUI macOS Rendering

GPUI uses **Metal** on macOS — activated automatically via `cfg(target_os = "macos")` in GPUI's own `Cargo.toml`. There is no PaneFlow-side configuration needed. The same WGSL shaders compile to Metal (macOS) and Vulkan (Linux) via the wgpu abstraction layer.

PaneFlow has no Vulkan-specific assumptions — all rendering goes through GPUI's `paint_quad()` and `shape_line()` APIs, which are platform-agnostic.

## Cross-Compilation

**Not viable.** GPUI's macOS build requires:
- macOS SDK (Xcode/CommandLineTools) for `bindgen`/`cbindgen` against Apple framework headers
- Native framework linking: `cocoa`, `core-graphics`, `core-text`, `metal`, `objc`
- System libraries that have no Linux stubs

`cargo check --target aarch64-apple-darwin` will fail on Linux without the macOS SDK. Full validation requires either:
1. A macOS machine (Apple Silicon or Intel)
2. A macOS CI runner (e.g., GitHub Actions `macos-latest`)

## Manual Verification Checklist (requires macOS)

- [ ] `cargo build` succeeds on macOS
- [ ] A terminal window opens and a shell prompt appears
- [ ] GPUI renders via Metal (check with `RUST_LOG=gpui=debug`)
- [ ] IPC socket is created (in `$TMPDIR/paneflow/`)
- [ ] `cwd_now()` returns `None` (expected until macOS impl added)
- [ ] TUI apps render correctly (Codex, neovim)

## Files Modified

| File | Change |
|------|--------|
| `src-app/Cargo.toml` | Platform-gated `gpui_platform` wayland/x11 features to Linux-only |
| `src-app/src/terminal.rs` | Gated `cwd_now()` behind `#[cfg(target_os = "linux")]` with non-Linux stub |
| `src-app/src/workspace.rs` | Gated `/proc/`-based port detection behind `#[cfg(target_os = "linux")]` with non-Linux stub |
| `tasks/spike-macos-build.md` | This document |

## Conclusion

PaneFlow's codebase is now macOS-compilation-ready at the source level. The one compile blocker (Cargo.toml feature gating) and one functional issue (`/proc` path) have been resolved. All remaining code uses cross-platform APIs (POSIX Unix, portable-pty, GPUI abstractions). Full validation awaits macOS hardware.
