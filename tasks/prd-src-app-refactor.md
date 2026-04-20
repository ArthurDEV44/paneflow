[PRD]
# PRD: `src-app/` Monolith Refactor — Break 21 kLOC Into Intention-Revealing Modules

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-19 | Claude + Arthur | Initial draft from 7-subagent swarm exploration of `src-app/src/` (21 002 LOC across 24 files) |

## Problem Statement

The `paneflow-app` crate has drifted into a maintainability crisis. **56 % of its 21 002 lines live in just three files** — `main.rs` (5 182 L), `terminal.rs` (3 697 L), `terminal_element.rs` (2 293 L). Eight additional files sit between 502 and 1 079 lines. This shape creates five concrete problems:

1. **Coding-agent context pressure.** A single `main.rs` occupies ~150 kB of tokens — more than 10 % of a 1 M context window — just to ground a trivial edit. Multi-file reasoning is forced to page through unrelated sections, hurting grounding and increasing hallucination risk.

2. **Human review is blocked by scale.** `terminal.rs`'s 3 697 lines mix PTY spawning, shell integration, OSC scanners, key dispatch, mouse handling, search, copy mode, and a 400-line `impl Render`. No reviewer holds this in working memory. Regressions slip through.

3. **Silent duplication.** `settings_window.rs:135–363` copies the title-bar rendering logic from `title_bar.rs` — any style tweak requires two synchronized edits. `keybindings.rs` maintains **three parallel exhaustive matches** on action names (`action_from_name`, `context_for_action`, `action_description`). Adding one action means editing three places.

4. **Circular coupling between `terminal.rs` ↔ `terminal_element.rs`.** `terminal_element.rs:26` imports `PtyNotifier, SpikeTermSize, ZedListener` from `terminal`; `terminal.rs:35` imports `TerminalElement`. Three types cross the boundary (`CopyModeCursorState`, `HyperlinkZone`, `SearchHighlight`) with no shared owner.

5. **Cross-platform hidden bug.** `self_update/mod.rs:225` references `libc::ENOSPC` un-gated — Windows builds will break when `libc` is unavailable. An additional Unix-only path (`fc-list` spawn in `config_writer.rs:104`) has no Windows equivalent.

**Why now.** The project has just completed its v2 GPUI rewrite, its Linux packaging migration, and is mid-port to macOS + Windows. Shipping cross-platform on the current code shape will multiply these three bottlenecks: each target adds conditional code paths into already-bloated files. A structural reset now, before platform divergence lands, costs weeks; delaying costs months.

**Scope.** This PRD is **refactoring only** — zero behavior change, zero new features, zero dependency bumps. Every commit must leave the app functionally identical. Success is measured by file-size distribution and absence of regressions, not by feature delivery.

## Overview

The refactor is organized into six epics in strict dependency order:

- **EP-001 (Quick Wins — Zero Risk)** lands three trivial extractions that prove the tooling works: the 63-item `actions!` block, a dead-code removal, and a file rename. Each is a single-file commit with a green `cargo check`.

- **EP-002 (Terminal Core Decomposition — Low Risk)** breaks the circular `terminal.rs` ↔ `terminal_element.rs` coupling by extracting a shared `terminal/types.rs`, then pulls out the leaf modules (scanners, service detector, shell integration, font, color, hyperlink). These have zero runtime state and no GPUI coupling — extraction is mechanical.

- **EP-003 (Terminal Finalization + Paint Passes — Medium Risk)** completes the terminal module by extracting `pty_session.rs`, `input.rs`, and splitting the `TerminalElement` paint method into passes (`background`, `text`, `cursor`, `selection`, `scrollbar`, `overlay`). Requires careful handling of shared `Bounds`/`Pixels`/`Arc<Mutex>` state.

- **EP-004 (App Layer + Window Chrome — Medium Risk)** extracts `session`, `settings`, and `self_update_flow` from `main.rs`, creates `window_chrome/`, and **eliminates the title-bar duplication** between `title_bar.rs` and `settings_window.rs`.

- **EP-005 (Keybindings + Core App Ops — High Risk)** rebuilds `keybindings/` around a unified `ActionMeta` struct (killing the triple-match), then carves `workspace_ops`, `ipc_handler`, `sidebar`, `event_handlers`, and `bootstrap` out of `main.rs`. This is the longest epic and the one with the highest blast radius.

- **EP-006 (Peripheral Modules + Hygiene)** finishes the peripheral splits (`layout/`, `workspace/`, `update/`, `theme/`, `fonts.rs`), fixes the cross-platform `ENOSPC` bug, and synchronizes documentation (`CLAUDE.md`) with the discovered reality (63 actions, 6 themes, new folder layout).

### Key Decisions

- **Re-export facade pattern** for every monolith split — `foo.rs` becomes `foo/mod.rs` with `pub use` re-exports. Public API stays byte-identical to external callers. Validated as lowest-risk pattern in Rust community (Rust Book, Sling Academy).
- **Distributed `impl` blocks** for `PaneFlowApp` and `TerminalView` — methods grouped by concern across sibling files within the same module directory. Each file declares `impl StructName { ... }` blocks.
- **`pub(super)` / `pub(crate)` only** — never widen fields to `pub`. Enforced by `clippy::unreachable_pub` lint.
- **`terminal/types.rs` FIRST** — extracting the shared types breaks the circular coupling before any other `terminal/` work starts. US-004 is a hard dependency for US-005 through US-016.
- **`ActionMeta` struct** for keybindings — fuses `action_from_name`, `context_for_action`, `action_description` into one data structure. Eliminates triple-maintenance burden.
- **Atomic commits, one per story** — each commit produces a green `cargo check` + `cargo clippy -D warnings` + `cargo fmt --check` + manual smoke test. No squashing across stories.
- **No new tests in this PRD.** The app crate has zero tests today (by project convention, 39 tests live in `paneflow-config`). Adding tests mid-refactor doubles the risk surface. Test coverage is tracked separately in the existing `prd-stabilization-polish.md`.
- **No behavior changes, no new features, no dep bumps.** GPUI rev stays pinned. `alacritty_terminal 0.26` unchanged.

## Goals

| Goal | End-of-PRD Target | Verification |
|------|-------------------|--------------|
| Largest file size | ≤ 800 LOC (hard ceiling) | `wc -l src-app/src/**/*.rs \| sort -rn \| head -1` ≤ 800 |
| Mean file size | < 500 LOC | `(total LOC) / (file count)` < 500 |
| `main.rs` reduction | From 5 182 L → ≤ 350 L | `wc -l src-app/src/main.rs` ≤ 350 |
| `terminal.rs` reduction | From 3 697 L → ≤ 700 L (becomes `terminal/view.rs`) | `wc -l src-app/src/terminal/view.rs` ≤ 700 |
| `terminal_element.rs` reduction | From 2 293 L → ≤ 600 L (becomes `terminal/element/mod.rs`) | `wc -l src-app/src/terminal/element/mod.rs` ≤ 600 |
| Public API stability | Zero breakage | `cargo check --workspace` passes after every story |
| Lint cleanliness | Zero warnings | `cargo clippy --workspace -- -D warnings` passes after every story |
| Cross-platform correctness | Every `#[cfg(unix)]` has a `#[cfg(windows)]` pendant | Grep audit at epic end |
| Doc synchronization | `CLAUDE.md` reflects new structure, 63 actions, 6 themes | US-035 verifies |

## Target Users

### AI Coding Agent (Claude Code / Codex)

- **Role:** Primary contributor to PaneFlow's day-to-day evolution
- **Behaviors:** Reads 3–5 files per task, reasons across them, makes targeted edits
- **Pain points today:** A single edit in `main.rs` costs ~5 000 lines of context just to ground the change. Finding the right function inside `terminal.rs` requires pagination. Cross-referencing `keybindings.rs`'s three matches is error-prone.
- **Current workaround:** Excessive exploration, over-reading, context fragmentation — all of which reduce change quality.
- **Success looks like:** 95 % of typical tasks (add an action, tweak a pane operation, add a setting) fit within a 4-file read budget; each file is < 500 LOC and focused on one concern.

### Human Maintainer (Arthur + future contributors)

- **Role:** Architect and reviewer of PaneFlow's design
- **Behaviors:** Reviews PRs, designs new features, traces bugs across modules
- **Pain points today:** Cannot hold `terminal.rs` or `main.rs` in working memory. Code review for any change in these files is shallow (can't see all the implications).
- **Current workaround:** Relies on integration in head; skips deep review of big files; risks accepting subtle regressions.
- **Success looks like:** File names reveal intent; grepping for a concern lands in one file; bug bisection narrows to a module within minutes.

## Research Findings

### Rust Module Decomposition — Community Consensus

**Pattern 1 — Re-export facade (chosen).** Convert monolithic `foo.rs` into `foo/mod.rs` + children. `pub use` statements in `mod.rs` preserve the public API exactly. External callers see zero change. Sources: [Rust Book ch. 7.5](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html), [Sling Academy — Large-Scale Rust Structuring](https://www.slingacademy.com/article/best-practices-for-structuring-large-scale-rust-applications-with-modules/).

**Pattern 2 — Distributed `impl` blocks.** Multiple `impl Foo { ... }` in sibling files, allowed by Rust since RFC 735. Used by Zed for exactly this purpose. Pitfall: reaching for `pub` to expose private fields. Correct answer: `pub(super)` (parent module only) or `pub(crate)`. Source: [Rust users forum thread 7785](https://users.rust-lang.org/t/code-structure-for-big-impl-s-distributed-over-several-files/7785), [RFC 1422 — pub(restricted)](https://rust-lang.github.io/rfcs/1422-pub-restricted.html).

**Pattern 3 — Platform subdirectories.** `sys/linux/`, `sys/macos/`, `sys/windows/` behind a trait, with `#[cfg_attr(unix, path = "sys/linux.rs")]` to avoid duplicated `mod` declarations. Canonical reference: [crosvm style guide](https://crosvm.dev/book/contributing/style_guide_platform_specific_code.html). Applied in this PRD to the `update/` split (EP-006).

### Validation Protocol Without Tests

No integration tests in `src-app`. Community consensus for test-less refactoring ([The Rust Book ch. 12.3](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html), [codenotary guide](https://codenotary.com/blog/step-by-step-guide-refactoring-a-large-rust-codebase-with-aiderdev-and-custom-llms)):

1. One atomic commit per extraction.
2. Each commit: move → fix `use` paths → `cargo check` → `cargo clippy -D warnings` → `cargo fmt --check` → manual smoke test → commit.
3. Never combine move + rename in one commit — compiler errors become ambiguous.
4. Start from leaf modules, work inward toward the root.
5. `cargo watch -x check` for fast feedback during file moves.

### Zed's Reference Architecture

Zed (source of GPUI) splits its terminal UI exactly as we propose: `terminal_view.rs` hosts `impl Render + impl Item`; `terminal_element.rs` hosts `impl Element` (prepaint/paint). Validated via [DeepWiki: Zed terminal view and rendering](https://deepwiki.com/zed-industries/zed/9.2-terminal-view-and-rendering). This PRD extends that pattern to sub-modules (`terminal/element/paint/*`).

### PaneFlow-Specific Findings (from 7-subagent swarm)

- **63 actions** in `main.rs:130-194` (not the 24 documented in `CLAUDE.md`, nor the 53 initially estimated by the swarm).
- **6 bundled themes** — Catppuccin Mocha, PaneFlow Light, One Dark, Dracula, Gruvbox Dark, Solarized Dark.
- **Dead code**: `split.rs:58-84` — `#[allow(dead_code)] pub fn resize_check` never wired, annotation dates to US-008 of the v2 PRD.
- **Hidden cross-platform bug**: `self_update/mod.rs:225` — `libc::ENOSPC` non-gated.
- **Misplaced function**: `config_writer.rs:104` spawns `fc-list` (Unix fontconfig) — belongs in `fonts.rs`.
- **Misleading filename**: `ai_detector.rs` defines types but performs no detection — should be `ai_types.rs`.

## Assumptions & Constraints

### Assumptions (validated progressively; no standalone spike stories)

- **A1 — `actions!` macro re-exportable.** GPUI's `actions!(paneflow, [...])` produces types in the `paneflow::` namespace. We assume these types can be declared in `app/actions.rs` and re-exported by `app/mod.rs` via `pub use`, with handlers in `main.rs`'s `impl Render` continuing to work unchanged. US-001 validates this — if it compiles and the binary runs, the assumption holds.

- **A2 — Distributed `impl` blocks don't trigger borrow-checker regressions.** Splitting methods of `PaneFlowApp` and `TerminalView` across sibling files does not require widening field visibility beyond `pub(super)`. US-005 through US-007 progressively validate this.

- **A3 — `#[cfg_attr(target_os = "...", path = "...")]` works for update subdirs.** Used in EP-006 for `update/linux/{appimage, targz}.rs`. Fallback if not: plain `#[cfg(target_os = "...")] mod appimage;` blocks.

- **A4 — Workspace keeps building with the crate name unchanged.** `paneflow-app` stays as the crate name. Internal module paths change; `Cargo.toml` does not.

### Hard Constraints

- **No behavior changes** — if functionality differs before/after any commit, the commit is wrong.
- **No new dependencies** — refactoring uses only what is already in `Cargo.toml`.
- **GPUI rev pinned** at `0b984b5` throughout this PRD.
- **Rust edition 2024** — matches `src-app/Cargo.toml:4`.
- **Codebase language: English** — all new file/folder/symbol names in English. Comments in English. French allowed only in PRs or internal conversations (per user instructions).
- **Cross-platform mandatory** — every `#[cfg(unix)]` needs a `#[cfg(windows)]` pendant. No Linux-only regressions introduced.
- **`impl Render`, `impl Element`, `impl EventEmitter`, `impl Focusable`** — each stays in exactly one file per type (Rust orphan rules).
- **Public API stability** — external callers (via `mod` declarations in `main.rs`) see no import path changes for previously-public items.
- **Atomic commits** — one commit per story, message format `refactor(<module>): US-NNN — <short description>`.

### Out of Scope

- Adding tests — tracked in `prd-stabilization-polish.md`.
- Behavior changes, features, bug fixes beyond the three explicitly enumerated (dead code removal, misleading rename, cross-platform ENOSPC).
- Upstream GPUI bumps.
- Performance optimization.

## Quality Gates

These commands must pass for every user story before its commit lands on `main`:

- `cargo check --workspace` — compiles
- `cargo clippy --workspace -- -D warnings` — zero warnings (includes `unreachable_pub` lint which catches accidental `pub` widening)
- `cargo fmt --check` — formatted
- `cargo test --workspace` — all 39 existing tests pass (no new tests added)
- `cargo build --release` (once per epic, not per story) — LTO build still succeeds

Manual smoke test (after every story; ~60 s):

1. `cargo run` — app launches without panic
2. `Ctrl+Shift+D` / `Ctrl+Shift+E` — split horizontally + vertically
3. Type `echo hello` in a pane, observe output
4. `Ctrl+Shift+N` — new workspace, `Ctrl+1` / `Ctrl+2` — switch
5. Open Settings (gear icon or keyboard), switch tabs, close
6. `Ctrl+Shift+W` — close pane
7. Quit via title bar, relaunch — session restores

If the feature touched by the story has a specific manual check (e.g. search overlay for US-014), execute that check too.

## Epics & User Stories

### EP-001: Quick Wins — Zero-Risk Extractions

**Priority:** P0 — must land first, validates tooling and assumption A1.

**Definition of Done:** All three stories committed, `main.rs` reduced by ~70 L, one file renamed, one dead function removed.

#### US-001 — Extract `actions!` block into `app/actions.rs`

**As** an AI coding agent,
**I want** the 63-item `actions!` declaration to live in its own file,
**so that** I can grep for action definitions without loading 5 000 lines of unrelated UI code.

**Acceptance Criteria:**

- [ ] `src-app/src/app/mod.rs` exists and declares `pub mod actions;`
- [ ] `src-app/src/app/actions.rs` exists with the full `actions!(paneflow, [...])` block (63 items), copied verbatim from `main.rs:130–194`
- [ ] `main.rs` declares `mod app;` and uses `app::actions::*` in place of the inline block
- [ ] `main.rs` has ≥ 60 fewer lines (ends ≤ 5 122 L)
- [ ] No `pub use` in `app/actions.rs` — GPUI's `actions!` macro already makes items public
- [ ] Unhappy path: grep confirms zero remaining `actions!` invocation in `main.rs`
- [ ] Quality gates pass (compile, clippy, fmt)
- [ ] Manual smoke test: a representative keybinding (`Ctrl+Shift+D`) still splits panes
- [ ] Commit: `refactor(app): US-001 — extract actions! block into app/actions.rs`

**Priority:** P0 | **Size:** S (2 pt) | **Blocked by:** none

---

#### US-002 — Rename `ai_detector.rs` → `ai_types.rs`

**As** a human maintainer,
**I want** the file defining `AiTool` and `AiToolState` enums to be named honestly,
**so that** I don't waste time looking for detection logic that doesn't exist there.

**Acceptance Criteria:**

- [ ] `src-app/src/ai_types.rs` exists with identical content to the old `ai_detector.rs`
- [ ] `src-app/src/ai_detector.rs` no longer exists
- [ ] `main.rs` module declaration updated from `mod ai_detector;` to `mod ai_types;`
- [ ] All imports `use crate::ai_detector::*` become `use crate::ai_types::*` (verify with `rg "ai_detector"` returns zero matches)
- [ ] Unhappy path: binary still compiles and IPC hooks `ai.session_start` / `ai.session_end` still route (manual smoke: not required — internal-only rename)
- [ ] Quality gates pass
- [ ] Commit: `refactor: US-002 — rename ai_detector.rs to ai_types.rs`

**Priority:** P1 | **Size:** XS (1 pt) | **Blocked by:** none

---

#### US-003 — Remove dead `resize_check` from `split.rs`

**As** a code reviewer,
**I want** the dead `resize_check` function deleted,
**so that** `#[allow(dead_code)]` is not used as wallpaper over unused code.

**Acceptance Criteria:**

- [ ] `src-app/src/split.rs:54–84` — the `#[allow(dead_code)]` attribute AND the `pub fn resize_check` body are deleted
- [ ] `split.rs` has ≥ 30 fewer lines (ends ≤ 1 049 L)
- [ ] `cargo check` confirms no caller exists (would have errored if it did)
- [ ] No other `#[allow(dead_code)]` is introduced
- [ ] Unhappy path: `cargo clippy -D warnings` passes (no orphan import left over)
- [ ] Quality gates pass
- [ ] Commit: `refactor(split): US-003 — remove dead resize_check function`

**Priority:** P2 | **Size:** XS (1 pt) | **Blocked by:** none

---

### EP-002: Terminal Core Decomposition — Low-Risk Leaf Extractions

**Priority:** P0 — unblocks EP-003 and EP-004.

**Definition of Done:** `terminal/types.rs` breaks the circular coupling; 7 leaf modules extracted; `terminal.rs` reduced by ~1 150 LOC; `terminal_element.rs` reduced by ~570 LOC.

#### US-004 — Create `terminal/types.rs` and break circular coupling

**As** a coding agent,
**I want** `CopyModeCursorState`, `HyperlinkZone`, `SearchHighlight`, and `HyperlinkSource` in a shared `types.rs`,
**so that** `terminal.rs` and `terminal_element.rs` can be split further without the circular dependency blocking extraction.

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/mod.rs` exists (converted from `terminal.rs`)
- [ ] `src-app/src/terminal/types.rs` contains `CopyModeCursorState`, `HyperlinkZone`, `SearchHighlight`, `HyperlinkSource`
- [ ] `terminal_element.rs` imports from `crate::terminal::types` instead of `crate::terminal_element` for cursor state
- [ ] `terminal/mod.rs` re-exports every public item previously exposed by `terminal.rs` via `pub use` — external callers (`main.rs`, `workspace.rs`, `ipc.rs`) need zero import changes
- [ ] `rg "from crate::terminal_element::CopyModeCursorState"` returns zero matches outside `terminal_element.rs` itself
- [ ] Unhappy path: `cargo check --workspace` still passes (would have failed on cyclical `use` if the break wasn't clean)
- [ ] Quality gates pass
- [ ] Manual smoke: copy mode toggle (`Ctrl+Shift+Space` or config default) still enters/exits
- [ ] Commit: `refactor(terminal): US-004 — extract shared types, break circular coupling`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-001 (app/ structure precedent)

---

#### US-005 — Extract `terminal/scanners.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/scanners.rs` exists with `XtversionScanner`, `Osc7Scanner`, `Osc133Scanner`, `parse_osc7_uri`, `percent_decode`, `hex_val` (from `terminal.rs:2161–2494 + 2706–2762`)
- [ ] File ≤ 450 LOC
- [ ] `terminal/mod.rs` (ex `terminal.rs`) has ≥ 380 fewer lines
- [ ] `pty_reader_loop` (in `pty_loops.rs` after US-011) or current location imports scanners from `crate::terminal::scanners`
- [ ] All scanner structs use `pub(super)` or narrower visibility — none pushed to `pub`
- [ ] Unhappy path: terminal emits and responds to OSC 7 / OSC 133 sequences — smoke test: open a shell with OSC 133 prompt integration (e.g. starship, oh-my-bash, or any shell rc that emits the command-boundary escapes), observe prompt jumps work
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-005 — extract OSC/Xtversion scanners into scanners.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-004

---

#### US-006 — Extract `terminal/service_detector.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/service_detector.rs` exists with `ServiceInfo`, `parse_service_line`, `extract_local_port`, `extract_url`, `detect_framework` (from `terminal.rs:2594–2705`)
- [ ] File ≤ 130 LOC
- [ ] `terminal/mod.rs` has ≥ 110 fewer lines
- [ ] `TerminalState::scan_output` imports from `crate::terminal::service_detector`
- [ ] Unhappy path: start `npm run dev` or `vite` in a pane → service detection fires (workspace sidebar shows port)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-006 — extract service_detector.rs`

**Priority:** P0 | **Size:** S (2 pt) | **Blocked by:** US-004

---

#### US-007 — Extract `terminal/shell.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/shell.rs` exists with `ZSH_OSC7`, `BASH_OSC7`, `FISH_OSC7`, `PWSH_OSC7` constants, `resolve_default_shell`, `configured_shell_if_usable`, `resolve_default_shell_fallback` (Unix + Windows cfg blocks), `setup_shell_integration` (from `terminal.rs:88–361`)
- [ ] File ≤ 320 LOC
- [ ] `terminal/mod.rs` has ≥ 260 fewer lines
- [ ] Every `#[cfg(unix)]` branch has an explicit `#[cfg(not(unix))]` or `#[cfg(windows)]` counterpart — no platform silently falls through
- [ ] `TerminalState::new` imports from `crate::terminal::shell`
- [ ] Unhappy path: set `default_shell` to a non-existent path in `paneflow.json`, relaunch → app falls back to `$SHELL` or `/bin/sh` without panicking
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-007 — extract shell resolution and OSC7 wrappers into shell.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-004

---

#### US-008 — Extract `terminal/element/font.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/element/mod.rs` exists (converted from `terminal_element.rs`)
- [ ] `src-app/src/terminal/element/font.rs` contains `FONT_FALLBACKS`, `INSTALLED_MONO_FONTS`, `CachedFontConfig`, `FONT_CONFIG_CACHE`, `default_font_family`, `resolve_font_family`, `cached_font_config`, `base_font`, `font_size`, `measure_cell` (from `terminal_element.rs:1–193`)
- [ ] File ≤ 220 LOC
- [ ] `terminal/element/mod.rs` has ≥ 180 fewer lines
- [ ] All public items re-exported through `terminal/element/mod.rs` so external references break zero imports
- [ ] Unhappy path: set `font_family` to an invalid name in `paneflow.json` → app falls back to the first preferred mono family without panicking (smoke test)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal/element): US-008 — extract font resolution into element/font.rs`

**Priority:** P0 | **Size:** S (2 pt) | **Blocked by:** US-004

---

#### US-009 — Extract `terminal/element/color.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/element/color.rs` contains `srgb_to_y`, `apca_contrast`, `ensure_minimum_contrast`, `adjust_lightness_for_apca`, `convert_color`, `named_color`, `indexed_color`, `rgb_to_hsla` (from `terminal_element.rs:194–370 + 2095–2193`)
- [ ] File ≤ 310 LOC
- [ ] `terminal/element/mod.rs` has ≥ 270 fewer lines
- [ ] Unhappy path: switch to `Solarized Dark` theme (low-contrast variant) → text remains readable (APCA contrast enforcement still fires)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal/element): US-009 — extract color/APCA logic into element/color.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-008

---

#### US-010 — Extract `terminal/element/hyperlink.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/element/hyperlink.rs` contains `URL_REGEX_PATTERN`, `url_regex`, `detect_urls_on_line_mapped`, `is_url_scheme_openable`, plus re-exports of `HyperlinkZone`/`HyperlinkSource` from `crate::terminal::types`
- [ ] File ≤ 130 LOC
- [ ] `terminal/element/mod.rs` has ≥ 100 fewer lines
- [ ] Unhappy path: Ctrl+Hover over a URL in terminal output shows the tooltip; clicking opens the URL in the default browser
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal/element): US-010 — extract URL detection into element/hyperlink.rs`

**Priority:** P0 | **Size:** S (2 pt) | **Blocked by:** US-004

---

#### US-011 — Extract `terminal/listener.rs` + `terminal/pty_loops.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/listener.rs` contains `ZedListener`, `SpikeTermSize`, `impl EventListener` (from `terminal.rs:58–87`)
- [ ] `src-app/src/terminal/pty_loops.rs` contains `pty_reader_loop` + `pty_message_loop` (from `terminal.rs:2495–2593`)
- [ ] `listener.rs` ≤ 40 LOC, `pty_loops.rs` ≤ 120 LOC
- [ ] `terminal/mod.rs` has ≥ 120 fewer lines
- [ ] `pty_loops.rs` imports scanners from `crate::terminal::scanners` (relies on US-005)
- [ ] Unhappy path: PTY reader thread still detaches and survives terminal close (smoke: close a pane, the shell process is killed per `impl Drop`)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-011 — extract listener and PTY I/O loops`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-005

---

### EP-003: Terminal Finalization + Paint Passes — Medium Risk

**Priority:** P0 — completes the `terminal/` module, unlocks `main.rs` cleanup.

**Definition of Done:** `terminal.rs` fully replaced by `terminal/` module directory; `terminal_element.rs` decomposed into `terminal/element/` with paint sub-passes.

#### US-012 — Extract `terminal/pty_session.rs`

**As** a reviewer,
**I want** `TerminalState` and its PTY lifecycle in a single file named for its concern,
**so that** PTY spawning, notifier wiring, and `impl Drop` cleanup can be read in one sitting.

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/pty_session.rs` contains `PtySender`, `PtyNotifier`, `Osc52Mode`, `PromptMarkKind`, `PromptMark`, `ClipboardOp`, `ColorOp`, `hsla_to_alac_rgb`, full `struct TerminalState` + `impl TerminalState` + `impl Drop` (from `terminal.rs:362–1224`)
- [ ] File ≤ 850 LOC
- [ ] `terminal/mod.rs` has ≥ 800 fewer lines
- [ ] `#[cfg(unix)] use libc` and `#[cfg(windows)] use windows-sys` paths both present — no bare `libc` usage leaks into Windows build
- [ ] Unhappy path: closing a pane with a runaway `yes` running in the shell still kills the child within 100 ms (the `Msg::Shutdown` → SIGKILL fallback)
- [ ] Quality gates pass on both `cargo check --target x86_64-unknown-linux-gnu` and `cargo check --target x86_64-pc-windows-msvc` (if cross-compilation is set up; otherwise note manual verification)
- [ ] Commit: `refactor(terminal): US-012 — extract TerminalState and PTY lifecycle into pty_session.rs`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-007, US-011

---

#### US-013 — Extract `terminal/input.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/input.rs` contains `impl TerminalView { fn handle_key_down, fn pixel_to_grid, fn pixel_to_viewport, fn write_mouse_report, fn handle_mouse_down, fn handle_mouse_move, fn handle_mouse_up, fn handle_copy, fn handle_paste, fn handle_file_drop, fn write_paste_text, fn handle_scroll_wheel, fn handle_scroll_page_up, fn handle_scroll_page_down }` (from `terminal.rs:1569–2160`)
- [ ] File ≤ 620 LOC
- [ ] `terminal/mod.rs` (or `terminal/view.rs` if already moved) has ≥ 580 fewer lines
- [ ] Any field of `TerminalView` accessed by `input.rs` methods is marked `pub(super)` — no widening to `pub`
- [ ] Unhappy path: test every input category — Ctrl+C copy, Ctrl+V paste, mouse-drag selection, Shift+PgUp scroll, mouse-wheel scroll, file drop (drag a file from file manager)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-013 — extract input handlers into input.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-012

---

#### US-014 — Extract `terminal/search.rs` (search + copy mode)

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/search.rs` contains `impl TerminalView { fn clear_scroll_history, fn reset_terminal, fn toggle_search, fn dismiss_search, fn toggle_search_regex, fn search_next, fn search_prev, fn run_search, fn scroll_to_current_match, fn jump_to_prompt_prev, fn jump_to_prompt_next, fn toggle_copy_mode, fn enter_copy_mode, fn exit_copy_mode, fn move_copy_cursor, fn extend_copy_selection, fn ensure_copy_cursor_visible }` (from `terminal.rs:2809–3112`)
- [ ] File ≤ 340 LOC
- [ ] Note: this does NOT merge with the existing top-level `src-app/src/search.rs` (which provides `search_term` and `scroll_to_match` utilities). The top-level `search.rs` stays untouched; it is imported by `terminal/search.rs`.
- [ ] Unhappy path: toggle search (`Ctrl+F`), type a query, Enter to scroll to next match, toggle regex mode, Esc to dismiss; then enter copy mode, navigate with arrows, Enter to copy selection
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-014 — extract search + copy mode into search.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-012

---

#### US-015 — Split `terminal/element/` paint passes + geometry

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/element/geometry.rs` contains cell↔pixel conversion helpers and a `CellGeometry { origin, cell_width, line_height }` struct
- [ ] `src-app/src/terminal/element/paint/background.rs` — block chars + background rects (preserves `.floor()` / `.ceil()` pixel-alignment logic intact)
- [ ] `src-app/src/terminal/element/paint/text.rs` — `shape_line` glyph rendering
- [ ] `src-app/src/terminal/element/paint/cursor.rs` — cursor rendering (incl. copy mode cursor)
- [ ] `src-app/src/terminal/element/paint/selection.rs` — selection highlight
- [ ] `src-app/src/terminal/element/paint/scrollbar.rs` — scrollbar thumb + track
- [ ] `src-app/src/terminal/element/paint/overlay.rs` — search highlights, hyperlink tooltip, `#[cfg(debug_assertions)]` latency probe
- [ ] `terminal/element/mod.rs` ≤ 600 LOC (retains `impl Element for TerminalElement` with `request_layout` / `prepaint` / `paint` coordinating the sub-passes)
- [ ] Every paint sub-pass is a free function accepting `&mut Window`, `&mut cx`, a borrow of the painter state, and `CellGeometry` — no hidden state
- [ ] Unhappy path: toggle theme to `PaneFlow Light`, verify block chars (powerline glyphs, box drawing) render without gaps; scroll through a buffer while search is active, verify highlights follow
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal/element): US-015 — split paint passes into dedicated sub-modules`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-008, US-009, US-010

---

#### US-016 — Finalize `terminal/view.rs` and collapse `terminal/mod.rs` to re-exports

**Acceptance Criteria:**

- [ ] `src-app/src/terminal/view.rs` contains the `TerminalView` struct, `new`/`with_cwd`/IME methods, `detect_url_at_hover`, `TerminalEvent` enum, `impl EventEmitter`, `impl Focusable`, `dispatch_context`, `impl Render` (from the residual `terminal.rs:1225–1568 + 2714–2808 + 3113–3609`)
- [ ] `terminal/view.rs` ≤ 750 LOC
- [ ] `terminal/mod.rs` ≤ 60 LOC — contains only `mod` declarations, `pub use` re-exports, and top-level crate docs
- [ ] `impl Render for TerminalView`'s search overlay (lines ~3380–3609 of old `terminal.rs`) is split into a private helper `fn render_search_overlay(&self, cx: &mut Context<Self>) -> AnyElement` to keep the main render path under ~300 LOC
- [ ] Unhappy path: toggle search, type query, search overlay renders; IME marked text (composition) still displays in an Asian input scenario (smoke test on macOS if available, skip otherwise with a note)
- [ ] Quality gates pass
- [ ] Commit: `refactor(terminal): US-016 — finalize view.rs and collapse mod.rs to re-exports`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-012, US-013, US-014, US-015

---

### EP-004: App Layer + Window Chrome — Medium Risk

**Priority:** P0 — eliminates the title-bar duplication and carves settings/session out of `main.rs`.

**Definition of Done:** `settings_window.rs` is deleted (replaced by `settings/` directory); `title_bar.rs` + `csd.rs` live under `window_chrome/`; title-bar rendering code exists in exactly one location.

#### US-017 — Extract `app/session.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/session.rs` contains `impl PaneFlowApp { fn save_session, fn load_session, fn restore_workspaces, fn spawn_pane_from_surfaces }` (from `main.rs:1189–1347`)
- [ ] File ≤ 180 LOC
- [ ] `main.rs` has ≥ 150 fewer lines
- [ ] Unhappy path: close app mid-work (with split panes across 2 workspaces), relaunch — layout + CWD restored
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-017 — extract session persistence into app/session.rs`

**Priority:** P0 | **Size:** S (2 pt) | **Blocked by:** US-001

---

#### US-018 — Extract `app/settings.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/settings.rs` contains `impl PaneFlowApp { fn render_settings_page, fn render_settings_sidebar, fn render_shortcuts_content, fn render_appearance_content, fn handle_settings_key_down, fn close_settings, fn open_settings_window, fn handle_shortcut_recording }` (from `main.rs:3619–4204`)
- [ ] File ≤ 620 LOC
- [ ] `main.rs` has ≥ 580 fewer lines
- [ ] Settings-related fields on `PaneFlowApp` (`settings_section`, `recording_shortcut_idx`, `font_dropdown_open`, `font_search`, `mono_font_names`) marked `pub(super)` or kept private where possible
- [ ] Unhappy path: open settings, record a new shortcut for `SplitHorizontally`, save, relaunch — new shortcut persists and works
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-018 — extract settings rendering into app/settings.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-001

---

#### US-019 — Create `window_chrome/` directory

**Acceptance Criteria:**

- [ ] `src-app/src/window_chrome/mod.rs` created with `pub mod title_bar; pub mod csd;` and `pub use title_bar::*; pub use csd::*;`
- [ ] `src-app/src/window_chrome/title_bar.rs` — content of the old `title_bar.rs`, unchanged apart from module path updates
- [ ] `src-app/src/window_chrome/csd.rs` — content of the old `csd.rs`, unchanged
- [ ] Old `src-app/src/title_bar.rs` and `src-app/src/csd.rs` deleted
- [ ] `main.rs` declares `mod window_chrome;` (not `mod title_bar;` or `mod csd;`)
- [ ] External imports updated: `rg "use crate::(title_bar|csd)"` returns zero matches
- [ ] Unhappy path: title bar still renders all controls (close/minimize/maximize), drag-to-move still works on Wayland + X11, resize edges still snap
- [ ] Quality gates pass
- [ ] Commit: `refactor(window_chrome): US-019 — move title_bar.rs and csd.rs under window_chrome/`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** none (parallel with US-017/018)

---

#### US-020 — Eliminate title-bar duplication in `settings_window.rs`

**As** a maintainer,
**I want** one source of truth for window-control button rendering,
**so that** a style change to the close/min/max buttons doesn't require editing two files.

**Acceptance Criteria:**

- [ ] `settings_window.rs:135–363` (or its post-US-019 equivalent) — `render_window_button_group`, `render_window_button`, and the duplicated `render_title_bar` are DELETED
- [ ] `settings_window.rs` imports and uses the canonical `TitleBar` or a shared `render_button_group` helper from `crate::window_chrome`
- [ ] If a shared helper is needed, it lives in `window_chrome/csd.rs` (next to `default_button_layout`) and is `pub(crate)`
- [ ] `settings_window.rs` has ≥ 220 fewer lines (ends ≤ 720 L)
- [ ] Unhappy path: open settings window, verify title bar, close/min/max buttons, drag-to-move — all identical visually and behaviorally to the main window's title bar
- [ ] Quality gates pass
- [ ] Commit: `refactor(settings): US-020 — remove title-bar duplication, use window_chrome`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-019

---

#### US-021 — Split `settings_window.rs` into `settings/` directory

**Acceptance Criteria:**

- [ ] `src-app/src/settings/mod.rs` — `pub mod window; pub mod keyboard; pub mod tabs;` and `open_or_focus` entry point
- [ ] `src-app/src/settings/window.rs` — `SettingsWindow` struct, `impl SettingsWindow { new, cleanup }`, `impl Focusable`, `impl Render` (the shell only — backdrop, CSD hitbox, resize handles)
- [ ] `src-app/src/settings/keyboard.rs` — `handle_settings_key_down`
- [ ] `src-app/src/settings/tabs/mod.rs` — `pub mod shortcuts; pub mod appearance;`
- [ ] `src-app/src/settings/tabs/shortcuts.rs` — `render_shortcuts_content` + `handle_shortcut_recording`
- [ ] `src-app/src/settings/tabs/appearance.rs` — `render_appearance_content` (theme selector, font dropdown, preview)
- [ ] Every file ≤ 400 LOC
- [ ] Old `src-app/src/settings_window.rs` deleted
- [ ] `main.rs` declares `mod settings;` (not `mod settings_window;`)
- [ ] Unhappy path: every settings tab switch works; font search filters correctly; theme preview updates immediately on hover
- [ ] Quality gates pass
- [ ] Commit: `refactor(settings): US-021 — split settings_window.rs into settings/ directory`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-020

---

### EP-005: Keybindings + Core App Ops — High Risk, High Value

**Priority:** P0 — `main.rs` final collapse + triple-match elimination.

**Definition of Done:** `main.rs` ≤ 350 LOC; `keybindings.rs` replaced by `keybindings/` directory with `ActionMeta` unified registry.

#### US-022 — Rebuild `keybindings/` with `ActionMeta` unified struct

**As** a maintainer,
**I want** `action_from_name`, `context_for_action`, and `action_description` fused into a single data structure,
**so that** adding an action requires exactly one edit.

**Acceptance Criteria:**

- [ ] `src-app/src/keybindings/mod.rs` — re-exports `apply_keybindings`, `effective_shortcuts`, `ShortcutEntry`, `format_keystroke`
- [ ] `src-app/src/keybindings/defaults.rs` — `DefaultBinding` struct + `DEFAULTS` + `MACOS_ONLY_DEFAULTS` tables
- [ ] `src-app/src/keybindings/registry.rs` — **`ActionMeta { name, factory: fn() -> Box<dyn Action>, context: &'static str, description: &'static str }`** and a single `const ACTIONS: &[ActionMeta] = &[...]` table replacing the three matches
- [ ] `src-app/src/keybindings/apply.rs` — `normalize_keystroke`, `make_binding`, `apply_keybindings`
- [ ] `src-app/src/keybindings/display.rs` — `format_keystroke`, `effective_shortcuts`, `ShortcutEntry`, `is_bare_modifier`, `action_name_at`
- [ ] Old `src-app/src/keybindings.rs` deleted
- [ ] Every file ≤ 450 LOC (defaults.rs is data-heavy and may hit this ceiling — tests remain where they are)
- [ ] `action_from_name`, `context_for_action`, `action_description` no longer exist as separate functions — all routed through `ACTIONS` lookup
- [ ] `main.rs` declares `mod keybindings;` (path unchanged for callers)
- [ ] Unhappy path: every one of the 63 actions still dispatches — verify 10 representative ones (Split, FocusUp, NewWorkspace, ToggleSearch, Copy, Paste, OpenSettings, NextWorkspace, CloseWorkspace, Quit)
- [ ] Quality gates pass — especially `cargo test --workspace` (existing keybinding tests must all pass)
- [ ] Commit: `refactor(keybindings): US-022 — unify action registry via ActionMeta struct`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-001

---

#### US-023 — Extract `app/workspace_ops.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/workspace_ops.rs` contains all `impl PaneFlowApp` methods for workspace/pane lifecycle: `active_workspace`, `create_workspace`, `split`, `handle_focus_*`, `handle_tab_*`, `handle_swap_*`, `handle_layout_preset_*`, `handle_undo_close`, `handle_reorder_*`, `close_workspace_at`, etc. (from `main.rs:2054–3027`)
- [ ] File ≤ 1 000 LOC (borderline — if >1 000 after extraction, split further into `workspace_ops/{focus, tab, swap, layout}.rs` before commit)
- [ ] `main.rs` has ≥ 950 fewer lines
- [ ] Unhappy path: split 4 panes horizontally, swap two, close one, undo-close, switch workspace, drag-reorder workspaces — all interactions preserve state correctly
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-023 — extract workspace operations into app/workspace_ops.rs`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-001, US-017

---

#### US-024 — Extract `app/ipc_handler.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/ipc_handler.rs` contains `impl PaneFlowApp { fn process_ipc_requests, fn process_config_changes, fn process_update_check, fn handle_ipc }` (from `main.rs:1348–2053`, excluding `handle_start_self_update` which is US-028)
- [ ] File ≤ 700 LOC — if over, split `handle_ipc`'s giant `match` into sub-functions per namespace (`handle_system_*`, `handle_workspace_*`, `handle_surface_*`, `handle_ai_*`)
- [ ] `main.rs` has ≥ 680 fewer lines
- [ ] Unhappy path: send IPC `system.ping` via `paneflow-cli` or `nc` to the socket — responds `pong`. Send `surface.send_text` to a pane — text appears. Send `workspace.list` — JSON response with current workspaces.
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-024 — extract IPC dispatcher into app/ipc_handler.rs`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-001

---

#### US-025 — Extract `app/sidebar.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/sidebar.rs` contains `impl PaneFlowApp { fn render_sidebar, fn sidebar_action_btn }` (from `main.rs:3028–3618`)
- [ ] File ≤ 620 LOC
- [ ] `main.rs` has ≥ 580 fewer lines
- [ ] Sidebar-related types (`WorkspaceContextMenu`, `WorkspaceDrag`, `WorkspaceDragPreview`, `ClosedPaneRecord`, `Notification`, `Toast`, `ToastAction`) remain in `main.rs` or move to `app/mod.rs` — determined by whether they're used outside sidebar rendering
- [ ] Unhappy path: click `+` to create workspace, right-click to open context menu, drag-reorder workspaces, click the notification badge
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-025 — extract sidebar rendering into app/sidebar.rs`

**Priority:** P0 | **Size:** L (5 pt) | **Blocked by:** US-001

---

#### US-026 — Extract `app/event_handlers.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/event_handlers.rs` contains `handle_title_bar_event`, `handle_pane_event`, `handle_terminal_event`, `workspace_idx_for_terminal`, `sweep_stale_pids`, `start_loader_animation`, `schedule_port_scan`, `run_port_scan`, `handle_cwd_change` (from `main.rs:382–833`)
- [ ] File ≤ 480 LOC
- [ ] `main.rs` has ≥ 440 fewer lines
- [ ] Unhappy path: open a terminal, `cd` to a directory, verify CWD change propagates to workspace title; start `npm run dev`, verify port detection appears in sidebar
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-026 — extract event handlers into app/event_handlers.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-001

---

#### US-027 — Extract `app/bootstrap.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/bootstrap.rs` contains `impl PaneFlowApp { fn new }` (from `main.rs:835–1188`)
- [ ] File ≤ 400 LOC
- [ ] `main.rs` has ≥ 350 fewer lines
- [ ] All helper closures called during construction (IPC server spawn, config watcher subscription, update check spawn) move with `new` — if they are reused elsewhere, they stay public-in-bootstrap via `pub(super)`
- [ ] Unhappy path: cold start from a fresh XDG cache (delete `~/.cache/paneflow/`) — app creates a fresh workspace, IPC socket binds, update check runs
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-027 — extract bootstrap/new() into app/bootstrap.rs`

**Priority:** P0 | **Size:** M (3 pt) | **Blocked by:** US-024 (IPC helper dependencies)

---

#### US-028 — Extract `app/self_update_flow.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/app/self_update_flow.rs` contains `impl PaneFlowApp { fn handle_start_self_update }` (from `main.rs:1381–1597`)
- [ ] File ≤ 240 LOC
- [ ] `main.rs` has ≥ 210 fewer lines
- [ ] Unhappy path: trigger self-update (via settings or toast), verify the 3-branch dispatch (AppImage / tar.gz / package manager) still routes correctly — integration-test via injecting a synthetic update, skip if infeasible
- [ ] Quality gates pass
- [ ] Commit: `refactor(app): US-028 — extract self-update flow into app/self_update_flow.rs`

**Priority:** P1 | **Size:** S (2 pt) | **Blocked by:** US-001

---

### EP-006: Peripheral Modules + Hygiene

**Priority:** P1 — finishes the refactor and synchronizes documentation.

**Definition of Done:** `layout/`, `workspace/`, `update/`, `theme/`, `fonts.rs` all in place; `ENOSPC` bug fixed; `CLAUDE.md` updated.

#### US-029 — Split `split.rs` → `layout/` directory

**Acceptance Criteria:**

- [ ] `src-app/src/layout/mod.rs` — re-exports `SplitDirection`, `LayoutTree`, `FocusDirection`, `FocusNav` etc. for external callers
- [ ] `src-app/src/layout/tree.rs` — `SplitDirection`, `DragState`, `LayoutChild`, `LayoutTree` enum, constants `DIVIDER_PX`/`MIN_PANE_SIZE`, `normalize_ratios`
- [ ] `src-app/src/layout/mutations.rs` — `split_at_focused`, `split_first_leaf`, `split_at_pane`, `close_focused`, `remove_pane`, `swap_panes`, `insert_sibling`, `redistribute_equal`
- [ ] `src-app/src/layout/queries.rs` — `focused_pane`, `leaf_count`, `collect_leaves`, `equalize_ratios`, `first_leaf`, `last_leaf`
- [ ] `src-app/src/layout/render.rs` — `LayoutTree::render` only (drag handlers, canvas size capture, flex build)
- [ ] `src-app/src/layout/presets.rs` — `from_panes_equal`, `main_vertical`, `tiled`
- [ ] `src-app/src/layout/navigation.rs` — `FocusDirection`, `FocusNav`, `focus_first`, `focus_last`, `focus_in_direction`, `is_forward`, `is_backward`
- [ ] `src-app/src/layout/serde.rs` — `serialize`, `from_layout_node`
- [ ] Every file ≤ 280 LOC
- [ ] Old `src-app/src/split.rs` deleted
- [ ] `main.rs` declares `mod layout;` (not `mod split;`) — external `use crate::split::*` call-sites updated
- [ ] Unhappy path: all split/drag/resize/navigation/preset operations work identically
- [ ] Quality gates pass
- [ ] Commit: `refactor(layout): US-029 — split split.rs into layout/ module`

**Priority:** P1 | **Size:** L (5 pt) | **Blocked by:** US-003 (dead code removal precedes the move)

---

#### US-030 — Split `workspace.rs` → `workspace/` directory

**Acceptance Criteria:**

- [ ] `src-app/src/workspace/mod.rs` — `struct Workspace`, `impl Workspace` constructors, lifecycle methods
- [ ] `src-app/src/workspace/git.rs` — `GitDiffStats`, `read_capped`, `find_git_dir`, `parse_head`, `detect_branch`, `parse_shortstat` (portable, cross-platform)
- [ ] `src-app/src/workspace/ports.rs` — `detect_ports` + `collect_descendant_pids` with `#[cfg(target_os = "linux")]` / `#[cfg(target_os = "macos")]` / `#[cfg(not(any(...)))]` branches preserved
- [ ] Existing 9 tests (from `workspace.rs:601–786`) migrate to `workspace/git.rs` under `#[cfg(test)]`
- [ ] Every file ≤ 470 LOC
- [ ] Old `src-app/src/workspace.rs` deleted
- [ ] Unhappy path: `cargo test --workspace` — all 39 tests still pass
- [ ] Quality gates pass
- [ ] Commit: `refactor(workspace): US-030 — split workspace.rs into workspace/ module`

**Priority:** P1 | **Size:** S (2 pt) | **Blocked by:** none

---

#### US-031 — Reorganize `self_update/` → `update/` with platform subdirs

**Acceptance Criteria:**

- [ ] `src-app/src/update/mod.rs` — `SelfUpdateStatus`, dispatch entry point by `InstallMethod`
- [ ] `src-app/src/update/error.rs` — `UpdateError`, `classify`, `IntegrityMismatch`, `is_disk_full` (with proper cross-platform guards — see US-034)
- [ ] `src-app/src/update/checker.rs` — `AssetFormat`, `UpdateStatus`, `pick_asset`, `check_github_release`, `spawn_check` (from old `update_checker.rs`)
- [ ] `src-app/src/update/install_method.rs` — `InstallMethod`, `PackageManager`, `detect`, `classify` (from old `install_method.rs`)
- [ ] `src-app/src/update/linux/mod.rs` — re-exports `run_appimage_update`, `run_targz_update`, and the `.run` legacy flow
- [ ] `src-app/src/update/linux/appimage.rs` — content of old `self_update/appimage.rs`
- [ ] `src-app/src/update/linux/targz.rs` — content of old `self_update/targz.rs`
- [ ] `src-app/src/update/macos/dmg.rs` — stub with `run_dmg_update() -> Result<()> { bail!("macOS DMG self-update not yet implemented"); }`
- [ ] `src-app/src/update/windows/msi.rs` — stub with `run_msi_update() -> Result<()> { bail!("Windows MSI self-update not yet implemented"); }`
- [ ] Old `src-app/src/self_update/` directory, `update_checker.rs`, and `install_method.rs` deleted
- [ ] `main.rs` declares `mod update;` (not `mod self_update; mod update_checker; mod install_method;`)
- [ ] Every file ≤ 820 LOC (targz.rs may stay near its current size — the density is justified)
- [ ] Unhappy path: `system.capabilities` IPC call still reports correct install method; update check still runs and caches result
- [ ] Quality gates pass on both `cargo check` (Linux target) and `cargo check --target x86_64-pc-windows-msvc` (if wired)
- [ ] Commit: `refactor(update): US-031 — reorganize self_update into update/ with platform subdirs`

**Priority:** P1 | **Size:** L (5 pt) | **Blocked by:** US-034 (ENOSPC fix goes into `error.rs`)

---

#### US-032 — Split `theme.rs` → `theme/` directory

**Acceptance Criteria:**

- [ ] `src-app/src/theme/mod.rs` — re-exports `TerminalTheme`, `UiColors`, `active_theme`, `invalidate_theme_cache`, `config_mtime`, `THEMES`, `ThemeEntry`
- [ ] `src-app/src/theme/model.rs` — `TerminalTheme` struct (35 slots), `UiColors` struct, `h`, `ha`, `apply_surface_overrides`, `is_light_theme`, `ui_colors`
- [ ] `src-app/src/theme/builtin.rs` — `catppuccin_mocha`, `paneflow_light`, `one_dark`, `dracula`, `gruvbox_dark`, `solarized_dark` (6 themes, not 5), `THEMES` table, `ThemeEntry`
- [ ] `src-app/src/theme/watcher.rs` — `CachedTheme`, `THEME_CACHE`, `read_config_theme_name`, `resolve_theme`, `active_theme`, `invalidate_theme_cache`, `config_mtime`, `THEME_CHECK_INTERVAL`
- [ ] Every file ≤ 280 LOC
- [ ] Old `src-app/src/theme.rs` deleted
- [ ] Unhappy path: edit `theme` field in `paneflow.json`, wait 500 ms, theme hot-reloads; cycle through all 6 themes in settings, each applies
- [ ] Quality gates pass (including the 2 existing tests in `theme.rs:481–502`)
- [ ] Commit: `refactor(theme): US-032 — split theme.rs into theme/ module`

**Priority:** P1 | **Size:** M (3 pt) | **Blocked by:** none

---

#### US-033 — Extract `fonts.rs` from `config_writer.rs`

**Acceptance Criteria:**

- [ ] `src-app/src/fonts.rs` — contains `load_mono_fonts` (from `config_writer.rs:103–129`)
- [ ] `fonts.rs` has an explicit `#[cfg(windows)]` branch returning `Vec::new()` with a `log::warn!("Windows font enumeration not yet wired — returning empty list");` — OR a documented TODO referencing the Windows port PRD
- [ ] `config_writer.rs` no longer spawns `fc-list` (function removed)
- [ ] All 3 callers (`terminal_element.rs:54`, `main.rs:3949`, `settings_window.rs:561`) updated to `use crate::fonts::load_mono_fonts;`
- [ ] `config_writer.rs` ≤ 105 LOC
- [ ] `fonts.rs` ≤ 60 LOC
- [ ] `main.rs` declares `mod fonts;`
- [ ] Unhappy path on Linux: open settings → Appearance → font dropdown still populates with installed monospace fonts. On Windows: dropdown empty but app doesn't crash (manual verification in the Windows branch, else documented).
- [ ] Quality gates pass
- [ ] Commit: `refactor: US-033 — extract load_mono_fonts into fonts.rs with cross-platform guard`

**Priority:** P1 | **Size:** S (2 pt) | **Blocked by:** none

---

#### US-034 — Fix cross-platform `ENOSPC` bug in update error classification

**As** a Windows user running self-update,
**I want** disk-full detection to work on Windows,
**so that** the app doesn't refuse to compile against the Windows target because of an un-gated `libc` reference.

**Acceptance Criteria:**

- [ ] `src-app/src/update/error.rs:is_disk_full` (or its pre-US-031 equivalent at `self_update/mod.rs:225`) — the `libc::ENOSPC` branch is gated `#[cfg(unix)]`
- [ ] A `#[cfg(windows)]` branch is added using `std::io::ErrorKind::StorageFull` (stable since Rust 1.83) — no direct `windows-sys` call needed
- [ ] Both branches share the same behavior: return `true` when the underlying error is disk-full
- [ ] `grep -nE "libc::" src-app/src/` shows zero un-gated uses of `libc::` symbols in files meant to compile for Windows
- [ ] Unhappy path: `cargo check --target x86_64-pc-windows-msvc` succeeds (if cross-compile is configured, else document manual verification on a Windows host)
- [ ] Quality gates pass
- [ ] Commit: `fix(update): US-034 — gate libc::ENOSPC behind cfg(unix) with Windows StorageFull fallback`

**Priority:** P0 | **Size:** XS (1 pt) | **Blocked by:** none (can be done before US-031 to simplify)

---

#### US-035 — Synchronize `CLAUDE.md` with discovered reality

**As** a coding agent consuming project docs,
**I want** `CLAUDE.md` to reflect the current code,
**so that** I don't ground on outdated claims (24 actions, 5 themes, Linux-only).

**Acceptance Criteria:**

- [ ] `/home/arthur/dev/paneflow/CLAUDE.md` — "24 actions" → "63 actions"
- [ ] `CLAUDE.md` — "5 bundled themes" → "6 bundled themes"
- [ ] `CLAUDE.md` — architecture ASCII tree updated to reflect the new module directories (`app/`, `terminal/`, `terminal/element/`, `layout/`, `workspace/`, `window_chrome/`, `settings/`, `keybindings/`, `update/`, `theme/`, `fonts.rs`, `ai_types.rs`)
- [ ] `CLAUDE.md` — the "No macOS/Windows code exists — this is Linux-only right now" gotcha is REMOVED (project is now cross-platform — confirmed by the `Cross-platform compatibility (mandatory)` section)
- [ ] ~~`CLAUDE.md` — add one sentence under Gotchas about AI hook scripts.~~ Superseded by prd-v0.2.0-release-hardening US-013 (code deleted) + US-014 (doc line removed).
- [ ] Git diff on `CLAUDE.md` reviewed by human (Arthur) before commit
- [ ] Quality gates pass (trivially — markdown doesn't compile)
- [ ] Commit: `docs: US-035 — sync CLAUDE.md with refactored structure and corrected action/theme counts`

**Priority:** P1 | **Size:** XS (1 pt) | **Blocked by:** all other stories in this PRD (so the doc reflects final state)

---

## Dependencies & Execution Plan

```
EP-001 ──┐
         ├─ US-001 ──────┬─> US-017 ┐
         ├─ US-002       ├─> US-018 ┼─> EP-004 (US-019, US-020, US-021)
         └─ US-003 ───┐  ├─> US-022 │
                      │  ├─> US-023 ┘
                      │  ├─> US-024 ─> US-027
                      │  ├─> US-025
                      │  ├─> US-026
                      │  └─> US-028
                      │
EP-002 ─── US-004 ──┬─ US-005 ──> US-011 ──> US-012 ──┬─> US-013
                    ├─ US-006                         ├─> US-014
                    ├─ US-007                         └─> US-016
                    ├─ US-008 ──┬─> US-009 ──> US-015 ────> US-016
                    ├─ US-010 ─┘
                    └─ US-011
                      │
EP-006 ── US-034 ────┼─> US-031
         US-029 ─────┤ (can start after US-003)
         US-030 ─────┤ (independent)
         US-032 ─────┤ (independent)
         US-033 ─────┤ (independent)
         US-035 ───── (last — after all others)
```

Parallel execution opportunities:
- EP-002 stories US-005/US-006/US-007/US-008/US-010 all unblocked by US-004 → land in parallel branches
- US-029, US-030, US-032, US-033 in EP-006 are independent → parallel feature branches
- EP-005 workspace_ops / ipc_handler / sidebar / event_handlers are independent after EP-001 → parallel

## Risks & Mitigations

**R1 — Distributed `impl` blocks force `pub` widening (HIGH impact, MEDIUM likelihood).**
*Mitigation:* `cargo clippy -- -D warnings` enables `unreachable_pub` lint, which fires on every `pub` item never used outside its crate. If a story introduces `pub` widening, clippy catches it. Reviewer rule: if a story's diff adds `pub fn` where `pub(super) fn` would work, reject.

**R2 — Silent behavior regression without tests (HIGH impact, MEDIUM likelihood).**
*Mitigation:* Manual smoke test protocol (listed under Quality Gates) is mandatory per story. Atomic commits make `git bisect` cheap if regression is found later. Each story's unhappy-path acceptance criterion exercises the specific concern being extracted.

**R3 — Circular-dependency surprises in `terminal/` extractions (MEDIUM impact, LOW likelihood post-US-004).**
*Mitigation:* US-004 (types extraction) is a hard prerequisite for all other `terminal/` work. If US-004 fails to compile cleanly, halt the epic and re-scope.

**R4 — Keybinding registry rewrite breaks `cx.bind_keys` (HIGH impact, LOW likelihood).**
*Mitigation:* US-022 keeps the old `apply_keybindings` entry point signature unchanged. The internal registry rewrite is invisible to callers. Existing tests in `keybindings.rs:680–1052` (372 lines of tests) stay intact and must pass — they are the regression harness for this story.

**R5 — Merge conflicts during long-running refactor (MEDIUM impact, HIGH likelihood).**
*Mitigation:* Branch strategy is `refactor/us-NNN-short-description` per story, rebased onto `main` before merge. No long-lived epic branches. Merge to `main` promptly per story — maximum one day of divergence.

**R6 — Refactor scope creep (introducing fixes/features mid-refactor) (HIGH impact, MEDIUM likelihood).**
*Mitigation:* Hard constraint listed in Problem Statement: refactoring only. Any bug or improvement noticed during a story is logged in a separate issue, NOT fixed in the same commit. Exception: US-034 (ENOSPC) is explicitly scoped because it blocks the Windows build.

**R7 — Refactor stalls midway, leaving codebase in mixed state (MEDIUM impact, LOW likelihood).**
*Mitigation:* Each story is independently valuable (each reduces monolith size immediately). No story depends on a FUTURE story being landed. If the PRD stalls at EP-004, the code is still strictly better than at PRD start.

## Success Metrics

Baseline (measured 2026-04-19):
- Total LOC in `src-app/src/`: 21 002
- Files > 800 LOC: 6 (`main.rs`, `terminal.rs`, `terminal_element.rs`, `split.rs`, `keybindings.rs`, `settings_window.rs`, `targz.rs`)
- `main.rs` LOC: 5 182
- `terminal.rs` LOC: 3 697
- Mean file size: 876 LOC

Target (end-of-PRD, measured via `wc -l`):
- Total LOC in `src-app/src/`: 21 000 ± 500 (refactor preserves logic, adds some `mod.rs` re-export overhead)
- Files > 800 LOC: 0 (hard ceiling)
- `main.rs` LOC: ≤ 350
- `terminal/view.rs` LOC: ≤ 750
- Mean file size: < 500 LOC

Verification (ran after US-035 lands):
```bash
wc -l src-app/src/**/*.rs | sort -rn | head
# Largest file must be ≤ 800 LOC
```

## References

Internal:
- Swarm exploration report (conversation of 2026-04-19, 7 subagents)
- `tasks/prd-stabilization-polish.md` — format reference, adjacent scope
- `tasks/prd-windows-port.md` — Windows port PRD (depends on this refactor for cleaner platform dispatch)
- `CLAUDE.md` — project conventions
- `CMUX_ANALYSIS.md` — cmux reference spec (unchanged by this PRD)

External:
- [Rust Book ch. 7.5 — Separating Modules into Different Files](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)
- [Sling Academy — Structuring Large-Scale Rust Applications](https://www.slingacademy.com/article/best-practices-for-structuring-large-scale-rust-applications-with-modules/)
- [Rust users forum 7785 — impl blocks across files](https://users.rust-lang.org/t/code-structure-for-big-impl-s-distributed-over-several-files/7785)
- [RFC 1422 — pub(restricted)](https://rust-lang.github.io/rfcs/1422-pub-restricted.html)
- [crosvm style guide — platform-specific code](https://crosvm.dev/book/contributing/style_guide_platform_specific_code.html)
- [Chris Morgan — cfg_attr path for platform modules](https://chrismorgan.info/blog/rust-cfg_attr-mod-path/)
- [DeepWiki — Zed Terminal View and Rendering](https://deepwiki.com/zed-industries/zed/9.2-terminal-view-and-rendering)

[/PRD]
