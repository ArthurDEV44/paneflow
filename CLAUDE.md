# CLAUDE.md — PaneFlow

Cross-platform terminal multiplexer (cmux port) built in pure Rust with Zed's GPUI framework and Zed's Alacritty fork for VT emulation. Linux-first, targeting Wayland + X11.

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
PaneFlowApp (Entity<Render>)              ← src-app/src/main.rs
├── Entity<TitleBar>                      ← title_bar.rs — CSD window controls, drag-to-move
├── Sidebar (inline in render_sidebar)    ← main.rs:481 — workspace list, rename, "+" button
└── Vec<Workspace>                        ← workspace.rs
    └── Option<SplitNode>                 ← split.rs — binary tree enum
        ├── Leaf(Entity<TerminalView>)    ← terminal.rs — PTY + alacritty Term wrapper
        │   └── TerminalElement           ← terminal_element.rs — GPUI Element, cell-by-cell paint
        └── Split { direction, ratio, first, second }
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

GPUI and related crates are **local path dependencies** pointing to a Zed monorepo checkout:

```toml
gpui = { path = "/home/arthur/dev/zed/crates/gpui" }
gpui_platform = { path = "/home/arthur/dev/zed/crates/gpui_platform", features = ["wayland", "x11"] }
collections = { path = "/home/arthur/dev/zed/crates/collections" }
```

The Zed repo must exist at `/home/arthur/dev/zed` for the project to compile. Two crates-io patches are required by GPUI:
- `async-task` → `smol-rs/async-task` (specific git commit)
- `calloop` → `zed-industries/calloop` fork

Terminal emulation uses Zed's Alacritty fork: `git = "https://github.com/zed-industries/alacritty", rev = "9d9640d4"`.

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

All registered at `main.rs:710–734` via `cx.bind_keys()`. 24 total actions.

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

- **Theme hot-reload**: 500ms mtime polling in a `cx.spawn` loop. 5 bundled themes: Catppuccin Mocha (default), One Dark, Dracula, Gruvbox Dark, Solarized Dark.
- **`window_decorations`**: read at startup only — requires restart. `"client"` = CSD, `"server"` = SSD.
- **`shortcuts`**: schema exists but is **not wired** to action dispatch — keybindings are hardcoded.
- **`ConfigWatcher`** (notify crate, 300ms debounce): fully implemented in config crate but **not used** by the app — mtime polling is used instead.

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
- **`alacritty_terminal` is Zed's fork**, not upstream Alacritty — APIs differ. Use `ZedListener`, `FairMutex`, and Zed-specific `Term` methods.
- **No macOS/Windows code exists** — this is Linux-only right now. PTY uses POSIX APIs, display uses Wayland+X11.
- **`dirs` version mismatch**: `src-app` uses `dirs = "5.0"`, config crate uses `dirs = "6"`. They coexist but are separate semver releases.
- **Config `default_shell` is not wired** — `TerminalState::new()` reads `$SHELL` env var directly, ignoring the config value.
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
