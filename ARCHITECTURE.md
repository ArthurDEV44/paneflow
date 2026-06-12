# Paneflow Architecture

Paneflow is a native GPU-accelerated terminal workspace for running CLI coding
agents in parallel. One Rust binary, no web runtime: the UI is built on
[Zed's GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
framework, terminal emulation is upstream
[`alacritty_terminal`](https://crates.io/crates/alacritty_terminal), and
everything else — PTY management, agent lifecycle tracking, IPC, the MCP
bridge, self-update — is purpose-built in this repository.

This document describes how the pieces fit together. It is aimed at
contributors and at anyone curious how you build a multiplexing terminal app
without Electron.

## Workspace layout

The repo is a Cargo workspace with one binary crate and a set of small,
focused library crates:

| Crate | Path | Purpose |
|---|---|---|
| `paneflow-app` | `src-app/` | The GPUI application: all UI, panes, PTY sessions, IPC server, self-update |
| `paneflow-config` | `crates/paneflow-config/` | Config schema, tolerant JSON loader, file watcher |
| `paneflow-shim` | `crates/paneflow-shim/` | PATH shim wrapping 16 known agent CLIs so Paneflow can observe their lifecycle |
| `paneflow-ai-hook` | `crates/paneflow-ai-hook/` | The hook binary agent CLIs invoke to report session events back over IPC |
| `paneflow-ipc-client` | `crates/paneflow-ipc-client/` | Blocking JSON-RPC client for the local IPC socket (shared by the MCP bridge and the CLI) |
| `paneflow-mcp` | `crates/paneflow-mcp/` | Stdio MCP server exposing read-only pane access (`list_panes`, `read_pane`, `search_pane`) |
| `paneflow-mcp-install` | `crates/paneflow-mcp-install/` | GPU-free install engine for the MCP bridge: per-agent detection, idempotent config merge, backup + atomic write |
| `paneflow-process` | `crates/paneflow-process/` | Bounded external-process execution (wall-clock deadline + stdout cap) shared across crates |
| `paneflow-acp` | `crates/paneflow-acp/` | Agent identity types for the Agents view |
| `paneflow-telemetry` | `crates/paneflow-telemetry/` | Opt-in telemetry plumbing (no event leaves the machine unless consent resolves to `true`) |

The split is deliberate: anything that runs *outside* the GUI process (shim,
hook, MCP bridge) must stay GPU-free and tiny, so it lives in its own crate
and never links GPUI.

## Thread model

```
┌─────────────────────────────────────────────────────────┐
│ Main thread — GPUI event loop                           │
│   owns all Entity state, rendering, input dispatch      │
└─────────────────────────────────────────────────────────┘
        ▲                    ▲                    ▲
        │ Wakeup events      │ mpsc (10ms poll)   │ channel
┌───────┴────────┐  ┌────────┴───────┐  ┌─────────┴────────┐
│ PTY I/O threads│  │ IPC thread     │  │ Watcher threads  │
│ one per pane   │  │ JSON-RPC 2.0   │  │ config, theme,   │
│ (alacritty     │  │ socket server  │  │ git state        │
│  EventLoop)    │  │                │  │                  │
└────────────────┘  └────────────────┘  └──────────────────┘
```

- **Main thread**: the GPUI event loop. All UI state lives in `Entity<T>`
  values mutated through GPUI contexts; there are no locks around UI state.
- **PTY I/O threads**: one per terminal, spawned by
  `alacritty_terminal::EventLoop`. The only cross-thread data is the terminal
  grid itself, an `Arc<FairMutex<Term<…>>>` shared between the reader thread
  (which feeds the VTE parser) and the render pass (which locks it briefly to
  snapshot renderable content).
- **IPC thread**: accepts connections on a Unix socket (Linux/macOS) or named
  pipe (Windows). Stateless methods reply in place; stateful methods are
  dispatched to the main thread over a channel.
- Blocking work (git subprocesses, filesystem walks, fleet-wide search) is
  pushed to background executors — registering a recursive file watcher or
  scanning a monorepo on the render thread is how you get a
  "not responding" window, so the codebase treats the main thread as
  render-only.

## Keystroke → pixel

The full input/output pipeline, end to end:

```
KeyDownEvent
  → TerminalView::handle_key_down() → keys::to_esc_str()
  → write_to_pty() → PTY EventLoop thread → shell / agent CLI
  → output bytes → VTE parser → Term grid mutations
  → Wakeup event → channel → 4ms timer poll → sync() → cx.notify()
  → TerminalElement::prepaint()  — lock grid, snapshot renderable content
  → TerminalElement::paint()     — quads + shaped glyph runs
  → GPU (Vulkan on Linux, Metal on macOS, DirectX on Windows)
```

`TerminalElement` (`src-app/src/terminal/element/`) is the one place Paneflow
implements GPUI's low-level `Element` trait directly instead of composing
divs: terminal rendering wants per-cell control over background quads, glyph
runs, cursor shapes, underlines and hyperlink hitboxes. Everything else in the
app (sidebar, tabs, settings, diff viewer) is regular GPUI flex layout.

Debug builds can trace the whole pipeline: `PANEFLOW_LATENCY_PROBE=1` stamps a
keystroke at ingress and reports time-to-pixel.

## Terminal emulation behind a boundary

Paneflow uses **upstream** `alacritty_terminal` from crates.io — not a fork.
All alacritty types are confined behind neutral wrapper types in
`src-app/src/terminal/types.rs`, and a guard test enforces that only an
explicit allowlist of files may import `alacritty_terminal` directly. The rest
of the app sees Paneflow's own `Point`, mode flags, and cell types.

The boundary has already paid for itself once (migrating off Zed's alacritty
fork to upstream) and keeps the door open for swapping or extending the
emulation core without a codebase-wide refactor.

## Agent lifecycle tracking

The feature that makes Paneflow more than a tiling terminal: it knows what
the agents inside its panes are doing.

```
agent CLI (claude, codex, opencode, …)
  └─ launched through a PATH shim (paneflow-shim)
       └─ agent hooks fire paneflow-ai-hook on lifecycle events
            └─ ai.* JSON-RPC notifications over the local socket
                 └─ GUI: tab dots, sidebar spinners, attention queue,
                    desktop notifications carrying the actual question
```

- **Shim**: launching an agent from Paneflow puts a shim directory first in
  `PATH`. The shim records the real PID and process start time (PID-reuse
  safe), then execs the real binary. Sixteen agent CLIs are recognized by
  name; unknown tools are reported as themselves.
- **Hooks**: agents that support lifecycle hooks (Claude Code, Codex, …)
  report `session_start`, `tool_use`, `notification`, `stop`, `session_end`
  through the `ai.*` IPC namespace. Agents without hooks fall back to
  terminal-activity detection.
- **States**: thinking, waiting for input (with the actual prompt text),
  finished, errored (non-zero exit), stalled (no hook activity past a
  threshold). Each state routes to the UI — and to your own tooling, since
  the same events are observable over IPC.

Everything is human-in-the-loop by design: Paneflow pre-fills prompts into
real PTY sessions, it never drives an agent headlessly.

## IPC and the MCP bridge

A JSON-RPC 2.0 endpoint (Unix socket at `$XDG_RUNTIME_DIR/paneflow/`, named
pipe on Windows) exposes `system.*`, `workspace.*`, `surface.*` and `ai.*`
namespaces — enough to script workspace creation, send text to panes, and
subscribe to agent events. The `paneflow` CLI (`paneflow up`, `paneflow flow`)
is built on the same socket.

The MCP bridge re-exposes a read-only slice of this to agents themselves:
`paneflow mcp install` registers a stdio MCP server with Claude Code, Codex,
Gemini CLI and opencode, giving any agent the ability to *read* (never write)
other panes' scrollback. An agent debugging a failing dev server can read the
server pane's output directly instead of asking you to paste it. The bridge
binary ships embedded in the main binary and is extracted to a stable path at
launch, so there is nothing extra to install.

Ingress is treated as untrusted: session and config files are validated
structurally (layout budgets, ratio clamps, id alphabets) before they touch
app state.

## Self-update

Each install format has its own update path (apt/dnf repos, AppImage swap,
tarball swap, macOS app replacement), all driven by one in-app updater. Update
artifacts are verified with [minisign](https://jedisct1.github.io/minisign/)
signatures and the client **fails closed**: an unsigned or tampered artifact
is rejected, never installed. macOS builds are additionally Developer ID
signed + notarized, with the Team ID pinned at verification time.

## Telemetry (opt-in, fail-closed)

Telemetry is **disabled by default**. A first-run modal asks for consent; no
event is sent unless the answer is an explicit yes, and
`PANEFLOW_NO_TELEMETRY=1` overrides everything unconditionally. The full
client lives in `crates/paneflow-telemetry/` — the event set is five
app-lifecycle events (`app_started`, `app_exited`, `update_check_started`,
`update_installed`, `session_corrupted`) with no terminal content, no paths,
no prompts, ever.

## Cross-platform strategy

One codebase, three first-class targets. Platform-specific code is gated
behind `#[cfg(target_os)]` with a working path (or a documented stub) for the
other two platforms:

| Concern | Linux | macOS | Windows |
|---|---|---|---|
| GPU | Vulkan | Metal | DirectX |
| Windowing | Wayland + X11 | AppKit | Win32 |
| PTY | `portable-pty` | `portable-pty` | ConPTY via `portable-pty` |
| IPC | Unix socket | Unix socket | Named pipe |
| Packaging | `.deb` / `.rpm` / AppImage / tarball | signed + notarized `.dmg` | `.msi` (in progress) |

Linux and macOS ship today; the Windows port is actively in progress
(see [`docs/WINDOWS.md`](docs/WINDOWS.md)).

## Performance discipline

Perf claims in release notes are backed by reproducible procedures, not
vibes: heaptrack diffs for memory work, `cargo flamegraph` for CPU work,
criterion benchmarks for hot paths, and a keystroke-latency probe in debug
builds. The render thread never does blocking I/O; scans and searches that
touch the filesystem or many panes run on background executors and report
back through events.

## Building

```bash
cargo build --release    # LTO thin, strip, codegen-units=1
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

See the [README](README.md#build-from-source) for per-platform build
instructions and system dependencies.
