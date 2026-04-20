# CLAUDE.md — PaneFlow

Cross-platform terminal multiplexer (cmux port) built in pure Rust with Zed's GPUI framework and upstream `alacritty_terminal` (crates.io) for VT emulation. Linux-first, targeting Wayland + X11.

## Commands

```bash
# Build
cargo build
cargo build --release          # LTO thin, strip, codegen-units=1

# Run
cargo run                      # debug build, needs GPUI GPU support (Vulkan)
RUST_LOG=info cargo run        # with logging (env_logger)
PANEFLOW_LATENCY_PROBE=1 cargo run  # keystroke→pixel latency tracing (debug only)

# Test
cargo test --workspace         # all tests (config crate only — app crate has zero tests)
cargo test -p paneflow-config  # config crate tests only
cargo test <test_name> -- --nocapture  # single test with output

# Lint
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Architecture

```
PaneFlowApp (Entity<Render>)           ← src-app/src/main.rs
├── app/                               ← PaneFlowApp impl, split across modules
│   ├── actions.rs                     ← 57 GPUI action types (paneflow namespace)
│   ├── bootstrap.rs                   ← app init, window creation, GPUI setup
│   ├── event_handlers.rs              ← title-bar/pane/terminal event subscribers + stale-PID sweep
│   ├── ipc_handler.rs                 ← JSON-RPC handler dispatched to GPUI main thread
│   ├── self_update_flow.rs            ← check/download/install orchestration
│   ├── session.rs                     ← persist/restore workspaces to session.json
│   ├── settings.rs                    ← legacy inline settings (being migrated to settings/)
│   ├── sidebar/                       ← sidebar list + context menus
│   └── workspace_ops/                 ← create/close/select/rename/reveal
├── window_chrome/
│   ├── csd.rs                         ← client-side decorations, resize edges
│   └── title_bar.rs                   ← window controls, drag-to-move
├── workspace/                         ← Vec<Workspace> state
│   ├── mod.rs                         ← Workspace struct, AI agent PIDs, ports
│   ├── git.rs                         ← branch detection for badges
│   └── ports.rs                       ← TCP port scan (Linux /proc, macOS libproc, Windows stub)
├── layout/                            ← N-ary tree of panes (replaces old binary SplitNode)
│   ├── tree.rs / mutations.rs / navigation.rs / close.rs
│   ├── presets.rs                     ← even_h, even_v, main_vertical, tiled
│   ├── render.rs                      ← GPUI flex emission
│   └── queries.rs / serde.rs
├── pane.rs                            ← Pane: tab strip + active terminal
├── terminal/                          ← PTY session + VT emulation + rendering
│   ├── view.rs                        ← TerminalView (Entity<Render>)
│   ├── pty_session.rs / pty_loops.rs  ← portable-pty session, I/O threads
│   ├── listener.rs / input.rs         ← ZedListener, keystroke translation
│   ├── scanners.rs / search.rs        ← grid scan, find-in-buffer
│   ├── service_detector.rs / shell.rs ← dev-server detection, shell resolution
│   ├── types.rs                       ← shared terminal types
│   └── element/                       ← low-level GPUI Element rendering
│       ├── mod.rs                     ← TerminalElement: layout → prepaint → paint
│       ├── color.rs                   ← ANSI→Hsla, APCA contrast
│       ├── geometry.rs                ← cell geometry
│       ├── hyperlink.rs               ← OSC 8 + URL scanning
│       └── paint/                     ← paint-pass helpers
├── theme/                             ← theme model + hot-reload (6 bundled themes)
│   ├── mod.rs                         ← re-exports
│   ├── model.rs                       ← TerminalTheme (35 slots), UiColors, ui_colors()
│   ├── builtin.rs                     ← 6 themes + THEMES table + theme_by_name
│   └── watcher.rs                     ← 500 ms mtime cache, active_theme()
├── keybindings/
│   ├── defaults.rs / registry.rs      ← default bindings, action registry
│   ├── apply.rs                       ← apply_keybindings() wires cx.bind_keys
│   └── display.rs                     ← human-readable binding strings
├── settings/                          ← settings window (extracted from settings_window.rs)
│   ├── window.rs                      ← SettingsWindow root
│   ├── sidebar.rs / keyboard.rs       ← sidebar nav, keyboard-shortcut editor
│   └── tabs/                          ← appearance, shortcuts, …
├── update/                            ← self-update (replaces self_update/)
│   ├── checker.rs / error.rs          ← release checker, structured UpdateError
│   ├── install_method.rs              ← detect install mode (AppImage / .deb / .msi / .app / .tar.gz)
│   ├── linux/ / macos/ / windows/     ← per-OS install paths
│   └── mod.rs
├── fonts.rs                           ← load_mono_fonts (Linux/macOS fc-list, Windows stub)
├── ai_types.rs                        ← AiToolState enum shared by workspace/event_handlers
├── ipc.rs                             ← JSON-RPC server over interprocess (cross-platform)
├── keys.rs / mouse.rs / pty.rs        ← key/mouse translation, portable-pty helpers
├── search.rs                          ← find-in-buffer UI glue
├── runtime_paths.rs                   ← XDG + %APPDATA% path helpers
├── config_writer.rs                   ← read-modify-write paneflow.json
└── assets.rs                          ← rust-embed asset registry
```

### Thread model

- **Main thread**: GPUI event loop — owns all Entity state, rendering, input dispatch
- **PTY I/O threads**: one per terminal (spawned by `alacritty_terminal::EventLoop::spawn()`)
- **IPC thread**: Unix socket server at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` (JSON-RPC 2.0)
- **Shared state**: `Arc<FairMutex<Term<ZedListener>>>` is the only cross-thread data (terminal grid)

### Data flow: keystroke → pixel

```
KeyDownEvent → TerminalView::handle_key_down() → keys::to_esc_str()
→ write_to_pty() → Notifier → PTY EventLoop thread → shell
→ shell output → AlacEventLoop reads PTY → vte parser → Term grid mutations
→ ZedListener::send_event(Wakeup) → UnboundedChannel
→ 4ms smol::Timer poll → sync() → dirty=true → cx.notify()
→ TerminalElement::prepaint() → term.lock() → renderable_content()
→ TerminalElement::paint() → paint_quad + shape_line → pixels
```

### Workspace crates

| Crate | Path | Type | Purpose |
|-------|------|------|---------|
| `paneflow-app` | `src-app/` | Binary | GPUI application — all UI, PTY, IPC |
| `paneflow-config` | `crates/paneflow-config/` | Library | Config schema, JSON loader, file watcher |

## Critical external dependencies

GPUI and related crates are **git dependencies** pinned to a specific Zed monorepo commit:

```toml
gpui = { git = "https://github.com/zed-industries/zed", rev = "0b984b5..." }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "0b984b5..." }
collections = { git = "https://github.com/zed-industries/zed", rev = "0b984b5..." }
```

Cargo fetches GPUI from the Zed repo automatically — no local checkout required. Two crates-io patches are required by GPUI:
- `async-task` → `smol-rs/async-task` (specific git commit)
- `calloop` → `zed-industries/calloop` fork

Terminal emulation uses upstream `alacritty_terminal = "0.26"` from crates.io (migrated from Zed fork).

## GPUI patterns

- **Entity/Context model**: all mutable state lives in `Entity<T>`, mutated via `Context<Self>`. Use `cx.new()` to create, `cx.notify()` to trigger repaint, `cx.spawn()` for async tasks.
- **`actions!` macro** (`main.rs:40`): generates zero-sized typed action structs in the `paneflow` namespace. Actions are dispatched through GPUI's focus chain.
- **`Render` trait**: implement for high-level views (PaneFlowApp, TitleBar, TerminalView). Returns a div element tree.
- **`Element` trait**: implement for low-level custom rendering (TerminalElement only). Has 3 phases: `request_layout()` → `prepaint()` → `paint()`.
- **Focus**: each `TerminalView` owns a `FocusHandle`. Key context `"Terminal"` scopes terminal-only keybindings. Focus navigation is structural (binary tree traversal), not spatial.
- **No `Arc`/`Mutex` for UI state** — use `Rc<Cell<f32>>` for single-threaded shared state (e.g., split ratios in render closures).

## Split system (split.rs)

- `SplitNode` is a binary tree: `Leaf(Entity<TerminalView>)` | `Split { direction, ratio, first, second }`
- `Horizontal` = panes top/bottom (flex_col). `Vertical` = panes side-by-side (flex_row).
- Layout uses GPUI flex divs with `flex_basis(relative(ratio))`. Min pane size: 80px. Ratio clamped to 0.1–0.9.
- Max 32 panes, max 20 workspaces.
- Divider is a 4px bar. Drag-to-resize uses `Rc<Cell<f32>>` — known issue: hardcoded 800px container estimate at `split.rs:141`.

## Keybindings

All registered in `keybindings::apply_keybindings()` via `cx.bind_keys()`. 57 total actions (see `app/actions.rs`).

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+Shift+D/E` | Split horizontal/vertical | Global |
| `Ctrl+Shift+W` | Close pane | Global |
| `Alt+Arrow` | Focus navigation | Global |
| `Ctrl+Shift+N` | New workspace | Global |
| `Ctrl+Shift+Q` | Close workspace | Global |
| `Ctrl+Tab` | Next workspace | Global |
| `Ctrl+1–9` | Select workspace | Global |
| `Ctrl+Shift+C/V` | Copy/Paste | Terminal |
| `Shift+PageUp/Down` | Scroll | Terminal |

## Config

Location: `~/.config/paneflow/paneflow.json` (Linux XDG).

```json
{
  "default_shell": "/bin/zsh",
  "theme": "Catppuccin Mocha",
  "window_decorations": "client",
  "shortcuts": {},
  "commands": []
}
```

- **Theme hot-reload**: 500ms mtime polling in a `cx.spawn` loop. 6 bundled themes: Catppuccin Mocha (default), PaneFlow Light, One Dark, Dracula, Gruvbox Dark, Solarized Dark.
- **`window_decorations`**: read at startup only — requires restart. `"client"` = CSD, `"server"` = SSD.
- **`shortcuts`**: wired via `keybindings::apply_keybindings()` at startup. Users can override default keybindings in config.
- **`ConfigWatcher`** (notify crate, 300ms debounce): fully wired — background thread detects file changes and deposits new config for the GPUI main thread to apply.

## IPC (ipc.rs)

Unix socket JSON-RPC 2.0 at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`. Methods:

| Method | Thread | Description |
|--------|--------|-------------|
| `system.ping/capabilities/identify` | Socket | Stateless health checks |
| `workspace.list/current/create/select/close` | GPUI | Workspace management |
| `surface.list/send_text/split` | GPUI | Pane operations |

Stateful methods dispatch to GPUI main thread via `mpsc::channel`, polled every 10ms.

## Styling conventions

- **All styling is inline** via GPUI's Tailwind-like builder API: `.bg(rgb(0x181825)).px_3().rounded_md()`
- **Sidebar/titlebar colors are hardcoded** Catppuccin Mocha hex values — they do NOT change with the terminal theme.
- **Terminal colors** use the `TerminalTheme` struct (30 Hsla slots) resolved via `active_theme()`.
- **Font**: defaults to a platform-specific installed monospace fallback at 14px (`terminal_element.rs`). Invalid Linux font names fall back to the first available preferred mono family.

## Gotchas

- **GPUI is not on crates.io** — it's a local path dep from `/home/arthur/dev/zed`. Never suggest adding it as a crates-io dependency.
- **Never recommend iced** for this project — it was evaluated and rejected (unstable, custom WGPU glyph atlas too complex). The decision is final.
- **`SplitDirection::Horizontal`** means a horizontal divider bar (panes stacked top/bottom), NOT side-by-side. This is counterintuitive but consistent with the codebase.
- **`alacritty_terminal` is upstream** (crates.io `0.26`), migrated from Zed's fork. Uses `ZedListener` and `FairMutex` from the GPUI integration layer.
- **AI hook scripts** at `assets/bin/{claude,codex,paneflow-hook}` are Unix-only shell scripts; Windows equivalents are tracked in `prd-windows-port.md`.
- **`dirs` version mismatch**: `src-app` uses `dirs = "5.0"`, config crate uses `dirs = "6"`. They coexist but are separate semver releases.
- **Config `default_shell` is wired** — `TerminalState::new()` uses fallback chain: config `default_shell` → `$SHELL` → `/bin/sh`.
- **The `_io_thread` handle is discarded** (`terminal.rs:139`) — PTY I/O threads run detached. Shutdown is via `Msg::Shutdown` in `Drop`.
- **No tests in the app crate** — all 39 tests are in `paneflow-config`. UI is verified manually.
- **No CI/CD** — no GitHub Actions or automation. Quality gates are developer-run.
- **No LICENSE file** — license is declared as MIT in Cargo.toml but no LICENSE file exists at the repo root.

## PRD reference

Active PRDs in `tasks/`:
- `prd-v2-gpui-terminal.md` — 19 stories, all delivered (US-001 through US-019)
- `prd-v2-title-bar.md` — 12 stories, all delivered (US-001 through US-012)

Architecture decision: `tasks/audit-v2-options-final.md`
cmux reference spec: `CMUX_ANALYSIS.md` (417 lines, covers cmux Swift architecture)

## Commit convention

```
feat(module): US-NNN — description
refactor(module): description
docs: description
chore: description
```

Atomic commits per user story. Branch naming: `feat/description`.

## Cross-platform compatibility (mandatory)

Any new code, refactor, or change that touches the codebase in any way **must** be fully compatible with all three target platforms:

- **Linux** — every major distribution (Fedora, Ubuntu/Debian, Arch, openSUSE, etc.), both Wayland and X11.
- **macOS (Apple)** — Intel and Apple Silicon.
- **Windows** — Windows 10 and 11 (x64, and ARM64 where applicable).

Concretely this means:

- Never hardcode POSIX-only paths, shell commands, env vars, or separators. Use `std::path::PathBuf`, `std::env`, and the `dirs` crate (or equivalent) for all filesystem and environment access.
- Guard platform-specific code with `#[cfg(target_os = "…")]` and always provide a working path for the other two platforms (at minimum a graceful fallback or documented stub).
- Prefer cross-platform crates (`portable-pty`, `notify`, `dirs`, `which`, etc.) over POSIX-only APIs. If a POSIX-only crate is unavoidable, isolate it behind a trait with per-OS implementations.
- PTY, IPC, packaging, auto-update, keybindings, fonts, and file watching must each have Linux + macOS + Windows paths — never Linux-only.
- Before shipping a change, mentally (or actually) verify it compiles and behaves correctly on all three platforms. If you cannot verify, say so explicitly rather than assume.

This rule overrides any older "Linux-only" gotcha in this file — the project is actively porting to macOS and Windows, and all new work must land cross-platform by default.

## Anti-Friction Rules (claude-doctor)

Règles pour éviter les patterns de friction détectés par `claude-doctor` sur ce projet : edit-thrashing, restart-cluster, repeated-instructions, negative-drift, error-loop, excessive-exploration.

### Editing discipline (anti edit-thrashing)

- Read the full file before editing. Plan all changes, then make ONE complete edit.
- If you've edited the same file 3+ times, STOP. Re-read the user's original requirements and re-plan from scratch.
- Prefer one large coherent edit over multiple small incremental ones.

### Stay aligned with the user (anti repeated-instructions, rapid-corrections)

- Re-read the user's last message before responding. Follow through on every instruction completely — don't partially address requests.
- Every few turns on a long task, re-read the original request to verify you haven't drifted from the goal.
- When the user corrects you: stop, re-read their message, quote back what they actually asked for, and confirm understanding before proceeding.

### Act, don't explore (anti excessive-exploration)

- Don't read more than 3-5 files before making a change. Get a basic understanding, make the change, then iterate.
- Prefer acting early and correcting via feedback over prolonged reading and planning.

### Break loops (anti error-loop, restart-cluster)

- After 2 consecutive tool failures or the same error twice, STOP. Change your approach entirely — don't retry the same strategy. Explain what failed and try something genuinely different.
- When truly stuck, summarize what you've tried and ask the user for guidance rather than retrying.

### Verify output (anti negative-drift)

- Before presenting your result, double-check it actually addresses what the user asked for.
- If the diff doesn't map cleanly to the user's request, don't ship it — re-plan.
