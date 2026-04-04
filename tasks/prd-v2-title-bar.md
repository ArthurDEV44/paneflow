[PRD]
# PRD: PaneFlow v2 — Custom Title Bar with Window Controls

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-04 | Arthur (via Claude) | Initial draft — CSD title bar with close/min/max, based on Zed codebase analysis |

## Problem Statement

1. **PaneFlow v2 has no custom title bar.** The application uses OS Server-Side Decorations (SSD) by default, meaning the window manager draws a generic title bar that does not match PaneFlow's visual identity. The GPUI imports for `Decorations` and `WindowControlArea` exist in `main.rs:14,16` but are unused.

2. **SSD provides no app-level control over title bar content.** The current title bar shows only "PaneFlow" as an OS-level title string. There is no way to display the active workspace name, connection status, or app-specific controls without switching to Client-Side Decorations (CSD).

3. **Inconsistent appearance across Linux desktop environments.** SSD renders differently on GNOME, KDE, Sway, and i3 — users get a different experience depending on their DE. CSD gives PaneFlow a consistent, polished look everywhere.

**Why now:** The GPUI framework already provides all necessary CSD APIs (`window.start_window_move()`, `window.zoom_window()`, `window.minimize_window()`, `window.window_decorations()`, `window.window_controls()`). Zed's `platform_title_bar` crate (335 LOC) is a complete blueprint. PaneFlow v2's terminal rendering is functional (US-001 through US-019 complete) — the title bar is the next visual gap before the app feels production-ready.

## Overview

This PRD adds a custom Client-Side Decorated (CSD) title bar to PaneFlow v2. The implementation follows Zed editor's proven architecture, adapted for PaneFlow's simpler needs.

The title bar will render as a horizontal flex row at the top of the window, containing: the app title ("PaneFlow"), the active workspace name, and platform-appropriate window control buttons (close, minimize, maximize/restore). On Linux, the buttons are custom GPUI SVG elements with theme-aware colors. The title bar supports drag-to-move, double-click to maximize/restore, and right-click for the DE's native window menu.

Key design decisions:
- **Single module** (`src-app/src/title_bar.rs`) rather than Zed's 2-crate split — appropriate for PaneFlow's scope
- **CSD by default** with a `window_decorations` setting for users who prefer SSD
- **Button layout from DE** via `cx.button_layout()` — respects GNOME/KDE user preferences
- **Title bar theming** integrated into PaneFlow's existing `TerminalTheme` system

## Goals

| Goal | Month-1 Target | Month-3 Target |
|------|---------------|----------------|
| Title bar renders with close/min/max on Linux | 100% of launches | 100% |
| Drag-to-move works on Wayland + X11 | Works on GNOME, KDE, Sway | Works on all major compositors |
| Visual consistency across DEs | Same look on GNOME/KDE/Sway | Themes match DE button layout |
| Title bar render overhead | < 0.5ms per frame | < 0.3ms per frame |
| User satisfaction (no SSD fallback complaints) | Setting available for SSD preference | N/A |

## Target Users

### AI Agent Developer (Primary)
- **Role:** Developer running AI coding agents in parallel PaneFlow workspaces
- **Behaviors:** Launches PaneFlow via CLI or desktop entry, creates workspaces via socket API
- **Pain points:** Current SSD title bar looks generic and inconsistent across DEs; no workspace name visibility in title bar
- **Current workaround:** Relies on sidebar for workspace identification; accepts inconsistent DE styling
- **Success looks like:** Polished, branded title bar showing active workspace name; native-feeling close/min/max buttons that match their DE's button order

### Terminal Power User (Secondary)
- **Role:** Linux power user switching from tmux/Alacritty who expects native-quality window chrome
- **Behaviors:** Uses keyboard primarily but expects standard window controls for mouse interaction
- **Pain points:** Alacritty/WezTerm use OS decorations that vary wildly across DEs
- **Current workaround:** Uses WM keybindings for window management instead of buttons
- **Success looks like:** Title bar that feels like a first-class Linux app (like Zed, GNOME apps) with proper CSD rounded corners and DE-respecting button layout

## Research Findings

Key findings from deep analysis of Zed's codebase (6 parallel exploration agents):

### Competitive Context
- **Zed Editor**: Gold standard for GPUI title bars. 2-crate architecture (`platform_title_bar` 335 LOC + `title_bar` ~1200 LOC). 3 platform strategies: macOS native traffic lights, Linux CSD SVG buttons, Windows hit-test caption buttons. Rich content: project name, branch, collaborators, AV controls.
- **Alacritty**: No custom title bar — relies entirely on OS decorations
- **WezTerm**: Custom tab bar but OS window decorations for close/min/max
- **Market gap**: No terminal multiplexer offers a polished CSD title bar on Linux with DE-respecting button layout

### Best Practices Applied (from Zed codebase analysis)
- Title bar height = `1.75 * rem_size` with min 34px — scales with UI font size (Zed `crates/ui/src/utils/constants.rs:14-27`)
- CSD rounded corners use `CLIENT_SIDE_DECORATION_ROUNDING = px(10.0)` with tiling-aware corner squaring (Zed `crates/theme/src/theme.rs:49`)
- Button layout read from DE via `gtk-decoration-layout` gsetting through XDG Desktop Portal (Zed `crates/gpui_linux/src/linux/xdg_desktop_portal.rs:55-60`)
- Linux CSD buttons use SVG icons (`generic_close.svg`, `generic_minimize.svg`, `generic_maximize.svg`, `generic_restore.svg`) recolored at runtime via GPUI's `text_color` (Zed `crates/platform_title_bar/src/platforms/platform_linux.rs:84-105`)
- Window focus state toggles `title_bar_background` / `title_bar_inactive_background` on Linux only (Zed `platform_title_bar.rs:64-74`)

### Zed Reference Files (for implementation)
| File | Lines | What to Reference |
|------|-------|-------------------|
| `crates/platform_title_bar/src/platform_title_bar.rs` | 335 | `PlatformTitleBar::render()` — complete CSD render flow |
| `crates/platform_title_bar/src/platforms/platform_linux.rs` | ~200 | `LinuxWindowControls` — SVG button rendering |
| `crates/ui/src/utils/constants.rs` | 27 | `platform_title_bar_height()` calculation |
| `crates/theme/src/theme.rs` | 49 | `CLIENT_SIDE_DECORATION_ROUNDING` constant |
| `crates/gpui/src/window.rs` | 2022-2056 | `zoom_window()`, `window_decorations()`, `window_controls()` |
| `crates/workspace/src/workspace.rs` | 8051 | How title bar attaches to workspace render tree |

*Full research from 6 exploration agents available on request.*

## Assumptions & Constraints

### Assumptions (to validate)
- PaneFlow's GPUI checkout at `/home/arthur/dev/zed` includes `gpui_platform` with working CSD support on Wayland and X11 (validated by Zed running on same machine)
- The `svg()` GPUI element is available and supports `text_color`-based recoloring (confirmed in Zed codebase)
- `cx.button_layout()` is accessible from PaneFlow's GPUI dependency (needs verification — it's on the `App` context in Zed)

### Hard Constraints
- Must work on Linux (Wayland + X11) — macOS/Windows are future scope
- Must not regress typing latency (title bar rendering < 0.5ms overhead)
- Must respect the user's DE button layout (GNOME left-side close, KDE right-side, etc.)
- Must fall back gracefully to SSD when CSD is not supported (e.g., exotic X11 WMs without compositor)

## Quality Gates

These commands must pass for every user story:
- `cargo check --package paneflow-app` - build verification
- `cargo clippy --package paneflow-app -- -D warnings` - lint
- `cargo test --package paneflow-app` - unit tests (if applicable)

For UI stories, additional gates:
- Launch PaneFlow, verify title bar renders correctly on the developer's Linux environment
- Verify window controls (close/minimize/maximize) function correctly
- Verify drag-to-move works
- Check Zed reference file cited in story for implementation correctness

## Epics & User Stories

### EP-001: CSD Foundation & Window Controls

Enable Client-Side Decorations and render functional close/minimize/maximize buttons on the PaneFlow title bar.

**Definition of Done:** PaneFlow launches with a custom title bar containing working window controls that respect the DE's button layout. Drag-to-move and double-click-to-maximize work. SSD fallback is available via setting.

#### US-001: Enable Client-Side Decorations mode
**Description:** As a PaneFlow user, I want the application to use Client-Side Decorations by default so that the title bar appearance is consistent across all Linux desktop environments.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `WindowOptions.window_decorations` is set to `Some(WindowDecorations::Client)` in `main.rs` window creation
- [ ] The `titlebar.appears_transparent` option is set to `true` to allow custom title bar rendering
- [ ] When launched on Wayland (GNOME/KDE/Sway), the OS title bar is not rendered — only PaneFlow's custom title bar appears
- [ ] When launched on X11 with compositor, the OS title bar is not rendered
- [ ] Given X11 without compositor support for CSD, when the app launches, then it falls back to SSD gracefully (no crash, no visual artifacts)

**Zed reference:** `crates/gpui/src/platform.rs:370-377` (WindowDecorations enum), `crates/gpui_linux/src/linux/wayland/window.rs:1442-1458` (request_decorations)

#### US-002: Create title bar module with basic layout
**Description:** As a PaneFlow user, I want to see a styled title bar at the top of the window so that the app feels polished and I can identify which workspace is active.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] A new `src-app/src/title_bar.rs` module exists and is imported in `main.rs`
- [ ] The title bar renders as an `h_flex` row at the top of the window, above the existing sidebar + content layout
- [ ] The render tree is restructured to `v_flex` → [title_bar, existing h_flex(sidebar + content)]
- [ ] Title bar height follows Zed's formula: `1.75 * window.rem_size()` with a minimum of `px(34.0)`
- [ ] The title bar displays "PaneFlow" as left-aligned text, and the active workspace name next to it
- [ ] Given no active workspace, when the app launches, then the title bar shows "PaneFlow" only (no crash or empty text)
- [ ] Given a workspace name longer than 40 characters, when rendered, then it is truncated with ellipsis

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:196-326` (render layout), `crates/ui/src/utils/constants.rs:14-27` (height calc)

#### US-003: Render Linux CSD window control buttons
**Description:** As a Linux user, I want close, minimize, and maximize/restore buttons in the title bar so that I can control the window without keyboard shortcuts.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Close, minimize, and maximize buttons render as SVG icons in the title bar
- [ ] SVG icon files (`generic_close.svg`, `generic_minimize.svg`, `generic_maximize.svg`, `generic_restore.svg`) are included in PaneFlow's assets
- [ ] Clicking the close button dispatches a close action that terminates the application gracefully (PTY processes cleaned up)
- [ ] Clicking minimize calls `window.minimize_window()`
- [ ] Clicking maximize calls `window.zoom_window()` (toggles maximize/restore)
- [ ] When the window is maximized, the maximize button icon switches to the restore icon
- [ ] Given the compositor reports `window_controls.minimize == false`, when rendering, then the minimize button is hidden
- [ ] Buttons have hover and active visual states using theme colors (`ghost_element_hover`, `ghost_element_active`)
- [ ] Mouse-down on buttons does not trigger title bar drag (`cx.stop_propagation()`)

**Zed reference:** `crates/platform_title_bar/src/platforms/platform_linux.rs:7-244` (LinuxWindowControls + WindowControl render)

#### US-004: Drag-to-move window via title bar
**Description:** As a user, I want to drag the title bar to move the window so that I can position it on my screen.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Mouse-down + mouse-move on the title bar (outside of buttons) initiates window move via `window.start_window_move()`
- [ ] The drag only starts on actual mouse movement after mouse-down (not on click alone)
- [ ] Drag works correctly on both Wayland and X11
- [ ] Given the user clicks a window control button, when they drag, then the window does NOT move (button area excludes drag)

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:200-221` (should_move state machine + start_window_move)

#### US-005: Double-click title bar to maximize/restore
**Description:** As a Linux user, I want to double-click the title bar to maximize or restore the window, matching standard desktop behavior.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] Double-clicking the title bar area (outside buttons) calls `window.zoom_window()`, toggling between maximized and restored states
- [ ] The double-click is detected via `event.click_count() == 2` in an `on_click` handler
- [ ] Given the window is restored, when double-clicked, then it maximizes to fill the screen
- [ ] Given the window is maximized, when double-clicked, then it restores to previous size and position

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:233-239` (Linux double-click handler)

#### US-006: Read button layout from desktop environment
**Description:** As a Linux user, I want PaneFlow's window control buttons to appear in the same order and position (left or right) as my desktop environment configures them, so the app feels native.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] PaneFlow reads the button layout from `cx.button_layout()` (which reads `gtk-decoration-layout` via XDG Desktop Portal)
- [ ] Buttons are rendered on the left side, right side, or both sides according to the DE configuration
- [ ] On GNOME (default `close:maximize`), close appears on the left and maximize on the right
- [ ] On KDE (default `minimize,maximize,close` on right), all three appear on the right
- [ ] Given the DE does not expose a button layout, when rendering, then buttons default to right-side `[minimize, maximize, close]`
- [ ] Given the user changes their DE button layout at runtime, when PaneFlow receives the layout change callback, then buttons reorder without restart

**Zed reference:** `crates/gpui_linux/src/linux/xdg_desktop_portal.rs:55-116` (read + subscribe), `crates/gpui/src/platform.rs:456-478` (WindowButtonLayout parse + default)

---

### EP-002: Title Bar Polish & Settings

Visual refinements, theming integration, and user configurability for the title bar.

**Definition of Done:** Title bar has proper theming with active/inactive states, CSD rounded corners, right-click window menu, and a `window_decorations` setting for toggling CSD/SSD.

#### US-007: Title bar theming (active/inactive backgrounds)
**Description:** As a PaneFlow user, I want the title bar to change color when the window loses focus so that I can visually distinguish active and inactive PaneFlow windows.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Two new color fields are added to PaneFlow's theme system: `title_bar_background` and `title_bar_inactive_background`
- [ ] All bundled themes define these two color values (matching their palette)
- [ ] When the window is focused, the title bar uses `title_bar_background`
- [ ] When the window loses focus (on Linux), the title bar switches to `title_bar_inactive_background`
- [ ] Given the user switches themes at runtime, when the theme reloads, then the title bar colors update immediately
- [ ] Given a theme file does not define title bar colors, when loaded, then sensible defaults are used (e.g., slightly darker/lighter than the main background)

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:64-74` (title_bar_color method), `crates/theme/src/styles/colors.rs:124-125` (color fields)

#### US-008: CSD rounded corners with tiling awareness
**Description:** As a PaneFlow user on Linux CSD, I want the title bar to have rounded top corners that square off when the window is tiled/snapped to screen edges, matching standard CSD behavior.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] When in CSD mode and the window is floating (not tiled), the title bar has rounded top-left and top-right corners with radius `px(10.0)` (matching Zed's `CLIENT_SIDE_DECORATION_ROUNDING`)
- [ ] The title bar applies `mt(px(-1.))` and `border(px(1.))` to fill transparent gaps at rounded corners
- [ ] Given the window is tiled to the left edge, when rendering, then the top-left corner is squared (radius 0) while top-right remains rounded
- [ ] Given the window is tiled to the right edge, when rendering, then the top-right corner is squared while top-left remains rounded
- [ ] Given the window is maximized (all edges tiled), when rendering, then both top corners are squared
- [ ] Tiling state is read from `Decorations::Client { tiling }` returned by `window.window_decorations()`

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:263-281` (CSD rounding logic), `crates/theme/src/theme.rs:49` (rounding constant)

#### US-009: Right-click window menu on title bar
**Description:** As a Linux user, I want to right-click the title bar to access the desktop environment's native window menu (move, resize, minimize, maximize, close, always on top, etc.).

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Right-clicking the title bar (outside of buttons) calls `window.show_window_menu(event.position)`
- [ ] The window menu is only shown when `window.window_controls().window_menu` is `true`
- [ ] Given the compositor does not support window menus, when right-clicking, then nothing happens (no crash)
- [ ] The right-click does not interfere with left-click drag or double-click maximize

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:313-318` (right-click handler)

#### US-010: `window_decorations` setting (CSD/SSD toggle)
**Description:** As a PaneFlow user who prefers their desktop environment's native title bar, I want a setting to switch between Client-Side and Server-Side Decorations.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] A `window_decorations` field is added to PaneFlow's config schema (`paneflow-config/src/schema.rs`) with values `"client"` (default) and `"server"`
- [ ] When set to `"server"`, PaneFlow uses SSD — the OS draws the title bar and PaneFlow's custom title bar is hidden
- [ ] When set to `"client"`, PaneFlow uses CSD — PaneFlow's custom title bar is rendered
- [ ] The setting is read at startup and applied via `WindowOptions.window_decorations`
- [ ] Given the setting is changed in the config file while PaneFlow is running, when the config file watcher detects the change, then the decoration mode updates at next window creation (runtime toggle not required for v1)
- [ ] Given an invalid value in config, when loading, then it falls back to `"client"` with a warning log

**Zed reference:** `crates/settings_content/src/workspace.rs:337` (WindowDecorations enum), `crates/workspace/src/workspace_settings.rs:37` (setting field)

#### US-011: Title bar integration with sidebar collapse
**Description:** As a PaneFlow user, I want the title bar to span the full window width and visually connect with the sidebar when expanded, providing a cohesive layout.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] The title bar spans the full window width, above both sidebar and main content area
- [ ] When the sidebar is expanded, the title bar's left section (app title) aligns with the sidebar width
- [ ] When the sidebar is collapsed, the title bar's left section adjusts accordingly
- [ ] The title bar's background color is distinct from but harmonious with the sidebar background
- [ ] Given the window is resized to minimum width (800px), when rendering, then the title bar gracefully truncates content without overflow or visual breakage

**Zed reference:** `crates/platform_title_bar/src/platform_title_bar.rs:244-261` (sidebar-aware left padding), `platform_title_bar.rs:291` (overflow_x_hidden)

#### US-012: Window control button `window_control_area` hitboxes
**Description:** As a developer preparing for future Windows support, I want window control buttons to register `WindowControlArea` hitboxes so that platform hit-testing works correctly.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Each window control button element calls `.window_control_area(area)` with the appropriate `WindowControlArea` variant (`Close`, `Max`, `Min`)
- [ ] The title bar's drag area registers `WindowControlArea::Drag` via the `h_flex` container
- [ ] Given a future Windows port, when `WM_NCHITTEST` is received, then the registered hitboxes enable native window control behavior
- [ ] The hitbox registration does not interfere with existing Linux click handlers

**Zed reference:** `crates/platform_title_bar/src/platforms/platform_windows.rs:88-95` (control_area mapping), `crates/gpui/src/elements/div.rs:1128` (window_control_area API)

## Functional Requirements

- FR-01: The system must render a custom title bar at the top of the window when `window_decorations` is set to `"client"` (default)
- FR-02: The title bar must contain close, minimize, and maximize/restore buttons that perform their respective window operations
- FR-03: The title bar must support drag-to-move via mouse-down + mouse-move interaction
- FR-04: The title bar must display the application name and active workspace name
- FR-05: The system must read the DE's button layout and position buttons accordingly (left, right, or both sides)
- FR-06: The system must NOT render a custom title bar when `window_decorations` is set to `"server"`
- FR-07: The close button must gracefully terminate all PTY processes before closing the window

## Non-Functional Requirements

- **Performance:** Title bar render overhead < 0.5ms per frame at P95 (must not impact typing latency target of < 8ms)
- **Compatibility:** Works on GNOME 45+, KDE Plasma 6+, Sway 1.9+, i3 4.23+ (X11 with compositor)
- **Visual consistency:** Identical title bar appearance across all supported DEs when in CSD mode
- **Memory:** Title bar adds < 1 MB to application memory footprint (SVG assets + render state)
- **Startup:** Title bar rendering adds < 50ms to cold start time
- **Accessibility:** Window control buttons have minimum 24x24px hit target (WCAG 2.5.8 Target Size)

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | No active workspace | App launched with empty workspace list | Title bar shows "PaneFlow" only, no workspace name | — |
| 2 | CSD not supported on X11 | No compositor or no `_GTK_FRAME_EXTENTS` support | Fall back to SSD silently; log warning | — |
| 3 | Very long workspace name | User creates workspace with 100+ char name | Truncate with ellipsis at ~40 chars | — |
| 4 | CSD/SSD toggle at runtime | User changes `window_decorations` in config | Applied at next window creation; existing windows unchanged | — |
| 5 | Tiling state changes | Window snapped to screen edge | Title bar corners square off on tiled edges | — |
| 6 | DE button layout unavailable | XDG Desktop Portal not running or no gsetting | Default to right-side `[minimize, maximize, close]` | — |
| 7 | Window maximized | User clicks maximize or double-clicks title bar | Maximize icon switches to restore icon; corners square | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI CSD API is internal/unstable — may change in future Zed commits | Medium | High | Pin to specific Zed commit; wrap GPUI calls in PaneFlow abstraction layer |
| 2 | X11 CSD detection fails on exotic WMs (e.g., cwm, twm) | Low | Low | Graceful SSD fallback; `window_decorations: "server"` setting available |
| 3 | GNOME Wayland doesn't support `zxdg_toplevel_decoration_v1` | Medium | Low | GNOME forces CSD anyway — no protocol needed; works by default |
| 4 | Button layout reading requires `ashpd` (async runtime dependency) | Low | Medium | Already a transitive dependency via `gpui_platform`; no new deps needed |
| 5 | SVG icon rendering adds latency | Low | Medium | SVGs are tiny (16x16, <500 bytes each); GPUI caches rasterized output |

## Non-Goals

Explicit boundaries — what this version does NOT include:

- **macOS traffic light support** — requires Objective-C interop via `msg_send![]` for `NSWindow standardWindowButton`. Deferred to macOS platform epic.
- **Windows caption button support** — requires Segoe Fluent Icons font detection and `WM_NCHITTEST` hit-test pipeline. Deferred to Windows platform epic.
- **Tab bar in title bar** — Zed renders workspace tabs in the title bar area. PaneFlow uses a sidebar for workspace switching; no tab bar is planned.
- **Collaborator avatars or social features** — Zed's `collab.rs` renders facepiles and AV controls. PaneFlow is a local-first tool.
- **Application menu in title bar** — Zed renders a hamburger menu on Linux. PaneFlow uses the sidebar for navigation.

## Files NOT to Modify

- `src-app/src/terminal.rs` — Core terminal PTY handling; title bar must not touch keystroke path
- `src-app/src/terminal_element.rs` — GPU cell rendering; title bar is a separate render layer
- `crates/paneflow-core/` — Domain types; title bar is a UI concern only
- `crates/paneflow-ipc/` — Socket server; no IPC methods for title bar in this scope
- `crates/paneflow-cli/` — CLI binary; no CLI commands for title bar in this scope

## Technical Considerations

Frame as questions for engineering input — not mandates:

- **Architecture:** Single module `src-app/src/title_bar.rs` with platform-specific sub-sections — recommended based on PaneFlow's scope. Engineering to confirm if a separate crate is warranted. Zed's 2-crate approach (`platform_title_bar` + `title_bar`) is available as an alternative if complexity grows.
- **SVG Assets:** Copy Zed's `generic_close.svg`, `generic_minimize.svg`, `generic_maximize.svg`, `generic_restore.svg` into PaneFlow's `assets/icons/`. Alternative: use Unicode glyphs (simpler but less flexible). Trade-off: SVGs are theme-aware and scale cleanly; glyphs are zero-asset.
- **Theme Integration:** Add `title_bar_background` and `title_bar_inactive_background` to `TerminalTheme` in `src-app/src/theme.rs`. Alternative: hardcode colors. Trade-off: theme fields allow user customization; hardcoding is simpler.
- **Config Schema:** Add `window_decorations: "client" | "server"` to `paneflow-config/src/schema.rs`. Needs backward-compatible deserialization (missing field defaults to `"client"`).
- **GPUI API Availability:** Verify that `cx.button_layout()`, `window.window_controls()`, and `window.show_window_menu()` are accessible from PaneFlow's GPUI dependency path. These are confirmed in Zed's source but PaneFlow's import path may differ.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Title bar present on launch | 0% (SSD only) | 100% CSD | Month-1 | Manual verification |
| Window controls functional | 0% (no custom controls) | Close/min/max work | Month-1 | Manual test on GNOME + KDE + Sway |
| Typing latency regression | < 8ms P95 | < 8ms P95 (no regression) | Month-1 | `debug_assertions` latency probes (US-017) |
| Frame render time with title bar | N/A | < 0.5ms overhead P95 | Month-1 | GPUI frame timing |
| DE button layout respected | N/A | Correct on GNOME + KDE | Month-1 | Manual verification |

## Open Questions

- How should the title bar interact with fullscreen mode? Zed hides window controls in fullscreen (`platform_title_bar.rs:295`). Should PaneFlow hide the entire title bar or just the controls? — Engineering to decide during US-002 implementation.
- Should PaneFlow support runtime CSD/SSD toggle (like Zed's `request_decorations()` at runtime) or only at startup? — Product to decide. Current PRD specifies startup-only for v1, runtime toggle could be added later.
- Should the title bar show IPC connection status (connected agents count)? — Deferred to future PRD, but the architecture should allow adding content to the title bar later.
[/PRD]
