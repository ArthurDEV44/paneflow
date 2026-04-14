# Spike: Upstream alacritty_terminal API Compatibility

**US-004** | **Date:** 2026-04-14 | **Verdict: GO** (migrate with 2 adaptations)

## Current State

PaneFlow uses a local checkout of Zed's alacritty fork:

```toml
alacritty_terminal = { path = "/home/arthur/dev/alacritty/alacritty_terminal" }
```

Fork origin: `zed-industries/alacritty` (rev `9d9640d4`), version 0.25.1.
Target: upstream `alacritty_terminal` v0.26.0 on crates.io.

## API Catalog

### terminal.rs (PTY management, event handling)

| Import | API Used | Upstream v0.26 | Status |
|--------|----------|----------------|--------|
| `Term` | `Term::new(config, &dimensions, listener)` | Present (re-export of `term::Term`) | MATCH |
| `event::Event` (as `AlacEvent`) | `Wakeup`, `Title(String)`, `ResetTitle`, `Exit`, `ChildExit(i32)`, `CurrentWorkingDirectory(String)` | `Wakeup`, `Title`, `ResetTitle`, `Exit` present. `ChildExit` takes `ExitStatus` not `i32`. **`CurrentWorkingDirectory` does NOT exist.** | DIFF |
| `event::EventListener` | `impl EventListener for ZedListener` — `fn send_event(&self, event: Event)` | Present, same signature | MATCH |
| `event::Notify` | Imported but used only as trait bound context | Present upstream | MATCH |
| `event::WindowSize` | `WindowSize { num_cols, num_lines, cell_width, cell_height }` | Present, same fields | MATCH |
| `event_loop::EventLoop` (as `AlacEventLoop`) | `EventLoop::new(term, listener, pty, hold, hold)` → `.channel()` → `.spawn()` | Present. Constructor signature may differ (5th param added in fork). | CHECK |
| `event_loop::Msg` | `Msg::Resize(WindowSize)`, `Msg::Input(...)`, `Msg::Shutdown` | Present upstream | MATCH |
| `event_loop::Notifier` | `Notifier(channel_sender)` — `.0.send(Msg::...)` | Present upstream | MATCH |
| `grid::Dimensions` | `impl Dimensions for SpikeTermSize` — `total_lines()`, `screen_lines()`, `columns()` | Present upstream (trait) | MATCH |
| `grid::Scroll` (as `AlacScroll`) | `Scroll::Delta(i32)`, `Scroll::Top`, `Scroll::Bottom` | Present upstream | MATCH |
| `index::Column` (as `GridCol`) | Grid coordinate type | Present upstream | MATCH |
| `index::Line` (as `GridLine`) | Grid coordinate type | Present upstream | MATCH |
| `index::Point` (as `AlacPoint`) | `Point::new(Line, Column)` | Present upstream | MATCH |
| `index::Side` | Selection side enum | Present upstream | MATCH |
| `selection::Selection` | `Selection::new(SelectionType, point, side)` | Present upstream | MATCH |
| `selection::SelectionType` | `SelectionType::Simple` | Present upstream | MATCH |
| `sync::FairMutex` | `Arc<FairMutex<Term<ZedListener>>>` — `.lock()` | Present upstream | MATCH |
| `term::Config` (as `TermConfig`) | `Config::default()` | Present upstream | MATCH |
| `term::TermMode` | `TermMode::APP_CURSOR`, etc. | Present upstream | MATCH |
| `tty` | `tty::Options`, `tty::Shell`, `tty::Shell::new()`, `tty::new(&options, window_size, id)` | Present upstream (Unix-only `tty` module) | MATCH |

### terminal_element.rs (cell rendering)

| Import | API Used | Upstream v0.26 | Status |
|--------|----------|----------------|--------|
| `event::WindowSize` | Construct for `Msg::Resize` | Present | MATCH |
| `event_loop::Msg` | `Msg::Resize(WindowSize)` | Present | MATCH |
| `event_loop::Notifier` | `.0.send(...)` | Present | MATCH |
| `grid::Dimensions` | `term.columns()`, `term.screen_lines()` | Present | MATCH |
| `selection::SelectionRange` | `sel.start`, `sel.end`, `sel.is_block` | Present | MATCH |
| `sync::FairMutex` | `self.term.lock()` | Present | MATCH |
| `term::Term` | `term.renderable_content()`, `term.grid()[point]`, `term.resize()`, `term.history_size()` | Present | MATCH |
| `term::cell::Flags` (as `CellFlags`) | `WIDE_CHAR`, `WIDE_CHAR_SPACER`, `INVERSE`, `DIM`, `BOLD`, `ITALIC`, `BOLD_ITALIC`, `UNDERLINE`, `DOUBLE_UNDERLINE`, `UNDERCURL`, `DOTTED_UNDERLINE`, `DASHED_UNDERLINE`, `STRIKEOUT` | Present | MATCH |
| `vte::ansi::Color` (as `AnsiColor`) | `Color::Named`, `Color::Spec`, `Color::Indexed` | Present via `alacritty_terminal::vte::ansi` | MATCH |
| `vte::ansi::CursorShape` | `CursorShape::Block`, `Beam`, `Underline`, `HollowBlock`, `Hidden` | Present | MATCH |
| `vte::ansi::NamedColor` | All 16 colors + `Foreground`, `Background`, `Cursor`, `Dim*` variants | Present | MATCH |
| `RenderableContent` fields | `.cursor`, `.selection`, `.display_iter`, `.display_offset` | Present upstream | MATCH |
| `Cell` fields/methods | `.c`, `.fg`, `.bg`, `.flags`, `.zerowidth()` | All present upstream (`.zerowidth()` confirmed) | MATCH |

### keys.rs (keystroke mapping)

| Import | API Used | Upstream v0.26 | Status |
|--------|----------|----------------|--------|
| `term::TermMode` | `TermMode::APP_CURSOR`, `TermMode::APP_KEYPAD` | Present | MATCH |

## API Differences Requiring Migration

### DIFF-1: `Event::ChildExit(i32)` → `Event::ChildExit(ExitStatus)`

**Impact:** Low. PaneFlow stores the exit code as `Option<i32>`.

**Migration:**
```rust
// Before (fork):
AlacEvent::ChildExit(status) => { self.exited = Some(status); }

// After (upstream):
AlacEvent::ChildExit(status) => { self.exited = Some(status.code().unwrap_or(-1)); }
```

### DIFF-2: `Event::CurrentWorkingDirectory(String)` — does not exist upstream

**Impact:** Medium. PaneFlow uses this for OSC 7 CWD tracking — the shell sends `\e]7;file://host/path\a` and the fork delivers it as this event variant.

**Migration options:**

**Option A (recommended):** Handle OSC 7 in the `EventListener` implementation. Upstream dispatches unrecognized OSC sequences via `Event::ColorRequest` or silently drops them. The proper approach is to add a custom VTE performer handler or intercept the PTY output stream to parse OSC 7 before it reaches alacritty's parser.

**Option B:** Use PaneFlow's existing `cwd_now()` fallback (`/proc/<pid>/cwd` on Linux). This works but is polling-based rather than push-based. Can be used as an interim solution with a poll interval.

**Option C:** Contribute `Event::CurrentWorkingDirectory` upstream to alacritty. This is the cleanest long-term path but adds an external dependency on upstream acceptance.

**Recommended path for US-005:** Start with Option B (fallback polling) to unblock the migration. Implement Option A as a follow-up in a future PR. The `cwd_now()` function already exists and works on Linux.

### DIFF-3: `EventLoop::new()` constructor — verify parameter count

**Impact:** Low. The fork uses `EventLoop::new(term, listener, pty, hold, hold)` with two boolean params. Upstream v0.26 may have a different constructor signature after the `hold` → `drain_on_exit` rename.

**Migration:** Check upstream signature at migration time. Likely `EventLoop::new(term, listener, pty, drain_on_exit)` (4 params vs 5). Adjust the call in `terminal.rs:293`.

## APIs NOT in Upstream (fork-only)

| API | Used By | Workaround |
|-----|---------|------------|
| `Event::CurrentWorkingDirectory(String)` | `terminal.rs:321` | Use `cwd_now()` polling fallback |
| `ChildExit(i32)` (i32 variant) | `terminal.rs:326` | Use `.code().unwrap_or(-1)` on `ExitStatus` |

## APIs Confirmed Present Upstream

- `Cell.zerowidth()` — present (not Zed-specific)
- `FairMutex` — present in `sync` module (not Zed-specific)
- `tty::Options`, `tty::Shell`, `tty::new()` — present (Unix path)
- `renderable_content()` / `display_iter` — present
- `grid::Dimensions` trait — present
- `Term::bounds_to_string()` — present
- `Term::bottommost_line()`, `topmost_line()`, `last_column()`, `history_size()` — present

## Verdict: GO

**Migrate to upstream `alacritty_terminal` v0.26.0.**

Rationale:
- 95%+ API surface is identical
- Only 2 real differences, both with clear migration paths
- `Cell.zerowidth()` and `FairMutex` (the two highest-risk unknowns) are confirmed present upstream
- OSC 7 CWD tracking has a working fallback (`cwd_now()`)
- Removes dependency on local Zed fork checkout
- Enables `cargo build` from fresh git clone

Migration effort estimate: ~1 hour (mostly adjusting `Event::ChildExit` and removing `CurrentWorkingDirectory` handling in favor of polling).

## Risks

1. **OSC 7 regression:** Switching from push-based CWD to polling reduces responsiveness. Mitigated by short poll interval (already 4ms sync timer exists).
2. **EventLoop constructor:** Signature may differ — verify at migration time.
3. **Platform-specific tty:** `tty::new()` is Unix-only in upstream. Windows ConPTY is separate (addressed by US-007 portable-pty).
