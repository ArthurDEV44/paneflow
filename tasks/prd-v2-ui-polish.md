[PRD]
# PRD: PaneFlow v2 — UI Polish & Custom Shader Terminal Renderer

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-03 | Arthur (via Claude) | Initial draft — GPU renderer + cmux-tier UI polish |

## Problem Statement

1. **PaneFlow v2 terminal output is monochrome and unreadable.** The current `view_terminal_pane()` strips all ANSI color information from `TerminalState.to_grid()` and renders lines as plain `text()` widgets with a single foreground color (`#CDD6F4`). The `TerminalCanvas` GPU renderer exists in `renderer.rs` but is not wired to the view. Shell prompts, syntax highlighting, colored `ls` output, and git diffs all appear as flat white text on dark background.

2. **The UI chrome looks like a prototype, not a product.** Compared to cmux (the target design reference): sidebar tabs have no corner radius (cmux: 6pt), no accent color on selection (cmux: `#0091FF` blue fill), notification badges are inline text `"title (N)"` instead of 16x16 circle badges, fonts are too large (14pt vs cmux's 12.5pt semibold), and there is no visual hierarchy between active/inactive workspace items.

3. **The text() widget rendering path cannot scale.** Each PtyOutput message triggers `update() → to_grid() → Vec<String> → view()` which rebuilds the entire widget tree. For high-throughput terminal output (e.g., `cat /dev/urandom | xxd`), this CPU-bound path becomes the bottleneck. A GPU-instanced rendering path is needed to achieve the < 1ms per-frame target from the v2 PRD.

**Why now:** The v2 native rewrite (20 stories) is functionally complete — window, sidebar, splits, PTY, IPC all work. The rendering and visual polish are the remaining gaps before the app is usable for daily driving.

## Overview

Two parallel tracks that transform PaneFlow v2 from a functional prototype into a production-quality terminal multiplexer:

**Track A — Custom WGPU Shader Terminal Renderer:** Replace the text() widget path with a GPU-instanced cell renderer using iced's Shader widget. Build a glyph atlas using `cosmic-text` (font shaping) + `etagere` (bin packing) + custom WGSL shaders. Render all visible cells in 2 instanced draw calls (backgrounds + glyphs). As an intermediate step, use `rich_text` + `Span` for per-cell coloring before the Shader is ready.

**Track B — UI Chrome Polish:** Restyle all UI components to match cmux's design language — rounded sidebar tabs with accent fill, circle notification badges, proper typography (12.5pt semibold), tab bar for horizontal navigation, sidebar drag-resize, and a cohesive dark color theme built on the `#0091FF` accent system.

## Goals

| Goal | Current State | Target | Timeframe |
|------|--------------|--------|-----------|
| Terminal color rendering | Monochrome (colors stripped) | Full 16/256/truecolor per-cell | Week 1 (rich_text), Week 3 (Shader) |
| GPU frame render time (200x60) | N/A (CPU text widgets) | < 1ms via instanced WGPU draws | Week 3 |
| Sidebar visual quality | Flat rectangles, no badges | cmux-tier: rounded tabs, circle badges, accent system | Week 1-2 |
| UI consistency score | Prototype-level | Production-level dark theme with accent hierarchy | Week 2 |
| Rendering path for heavy output | ~50 fill_text() calls, jank on fast output | 2 GPU draw calls, 60 FPS sustained | Week 3-4 |

## Target Users

Same as v2 PRD: AI Agent Developers (primary) and Terminal Power Users (secondary). This PRD specifically addresses their expectation that a native terminal multiplexer should look and perform as well as cmux or Warp.

## Research Findings

### Competitive UI Analysis (cmux)

cmux achieves its polished look through:
- **Accent color system:** `#0091FF` blue for selected tabs, notification badges, drop indicators
- **Sidebar tabs:** 6pt `RoundedRectangle`, 10pt inner + 6pt outer horizontal padding, 8pt vertical padding
- **Typography:** 12.5pt semibold for titles, 10pt for subtitles, 9pt semibold for badges
- **Notification badges:** 16x16pt circles with accent fill, positioned leading before title
- **Material background:** `NSVisualEffectView` `.sidebar` (macOS-only, not replicable cross-platform — use solid color)

### Technical Stack Assessment

| Approach | Compatibility | Performance | Complexity |
|----------|--------------|-------------|------------|
| **iced `rich_text` + `Span`** | iced 0.13 native | CPU-bound, adequate for 80x24 | Low |
| **iced `Canvas` + `Cache`** | iced 0.13 native | Better with dirty tracking | Medium |
| **iced `Shader` widget + custom WGSL** | iced 0.13 (`wgpu` feature) | GPU-instanced, < 1ms | High |
| **`glyphon` text renderer** | **Incompatible** (needs wgpu 29, iced has 0.19) | N/A | N/A |
| **`iced_term` crate** | Needs iced 0.14, pre-stable | Canvas-based | Low (if compatible) |

**Decision:** Use `rich_text` + `Span` as the immediate rendering path (P0), then build the custom Shader pipeline (P1) for maximum performance. Skip glyphon and iced_term due to version incompatibilities.

### Glyph Atlas Architecture

All high-performance terminals (Alacritty, WezTerm, Ghostty) use the same pattern:
1. Rasterize monospace glyphs into a texture atlas (bin-packed via shelf/skyline allocator)
2. Per visible cell: encode `(row, col, glyph_id, fg_rgba, bg_rgba)` into an instance buffer
3. Two instanced draw calls: backgrounds (colored quads), then glyphs (textured quads from atlas)
4. Dirty-cell tracking: only re-upload changed instances

For iced 0.13 Shader widget: `Primitive::prepare()` gets `wgpu::Device` + `Queue` for buffer uploads; `Primitive::render()` gets `CommandEncoder` + `TextureView` for draw calls. `Storage` holds persistent pipeline/buffer/atlas state across frames.

## Assumptions & Constraints

### Assumptions
- iced 0.13's `Shader` widget `Storage` API supports lazily initializing a custom pipeline struct (based on `custom_shader` example — HIGH confidence)
- `cosmic-text` can rasterize terminal monospace glyphs with sub-millisecond latency per glyph (based on COSMIC desktop usage — HIGH confidence)
- `rich_text` with ~100-200 spans per frame is fast enough for interactive terminal use (MEDIUM confidence — needs validation)

### Hard Constraints
- Must stay on iced 0.13 (0.14 upgrade is a separate migration effort)
- Must use wgpu 0.19 (pinned by iced 0.13's `iced_wgpu` dependency)
- No `glyphon` (version mismatch), no `iced_term` (needs iced 0.14)
- Cross-platform: no macOS-only APIs (NSVisualEffectView, Metal)
- Must not regress the current responsive UI (no return to Canvas per-cell fill_text lag)

## Quality Gates

These commands must pass for every user story:
- `cargo check --workspace` — compilation check
- `cargo clippy --workspace -- -D warnings` — lint with zero warnings
- `cargo test --workspace` — full test suite
- `cargo build --release` — release build succeeds
- Visual verification: launch app, confirm feature renders correctly

## Epics & User Stories

### EP-001: Colored Terminal Rendering (rich_text Path)

Wire per-cell ANSI colors from `TerminalState.to_grid()` through `rich_text` + `Span` widgets, replacing the monochrome `text()` path. This is the immediate fix that makes the terminal usable.

**Definition of Done:** Terminal output shows correct ANSI colors — colored shell prompts, syntax highlighting, and `ls --color` output all render with proper foreground and background colors.

#### US-001: Wire CellData Colors to rich_text Spans
**Description:** As a developer, I want terminal text rendered with per-cell ANSI colors so that shell prompts, syntax highlighting, and colored command output are visually distinguishable.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `update()` caches `Vec<Vec<(String, Color, Color, bool, bool)>>` (text runs with fg, bg, bold, italic) instead of plain `Vec<String>`
- [ ] `view_terminal_pane()` renders each line as a `rich_text![]` with `span()` per color run
- [ ] `echo -e "\e[31mRed\e[32mGreen\e[0m"` displays "Red" in ANSI red and "Green" in ANSI green
- [ ] `ls --color` output shows directories in blue, executables in green, symlinks in cyan
- [ ] Background colors render via `span.background()` (e.g., `git diff` highlighted lines)
- [ ] Bold text renders with `Font::weight(Weight::Bold)`
- [ ] Given a 200-column terminal, when rendering, then frame time stays under 16ms (no jank)
- [ ] Given no PTY output (idle terminal), when measuring, then CPU usage is < 1%

#### US-002: Cursor Rendering in rich_text Path
**Description:** As a developer, I want a visible cursor in the terminal so that I know where my input will appear.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] The cursor position from `TerminalGrid.cursor_row/cursor_col` is rendered as an inverted-color cell in the rich_text output
- [ ] The cursor cell's foreground and background colors are swapped (inverse video)
- [ ] Given the cursor is at position (5, 10), when rendered, then exactly one cell at that position appears inverted
- [ ] Given an empty terminal (all spaces), when the cursor is visible, then a block character renders at the cursor position

---

### EP-002: Sidebar Polish (cmux Design Language)

Restyle the sidebar to match cmux's design specs — rounded tabs, accent color selection, circle notification badges, proper typography, and visual hierarchy.

**Definition of Done:** The sidebar is visually indistinguishable from cmux's sidebar in a dark theme screenshot comparison (accounting for platform font rendering differences).

#### US-003: Rounded Tab Items with Accent Selection
**Description:** As a developer, I want sidebar workspace items to look polished with rounded corners and accent-colored selection so that the UI feels production-quality.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Each workspace tab item has `border.radius: 6.0` (matching cmux's `RoundedRectangle(cornerRadius: 6)`)
- [ ] Selected tab background is `Color::from_rgb(0.0, 0.569, 1.0)` (`#0091FF` cmux accent blue)
- [ ] Selected tab text color is `Color::WHITE`
- [ ] Unselected tab text primary color is `Color::from_rgb(0.85, 0.85, 0.87)` (light gray)
- [ ] Tab item inner horizontal padding is 10pt, outer margin from sidebar edge is 6pt
- [ ] Tab item vertical padding is 8pt
- [ ] Tab row spacing (between items) is 2pt
- [ ] Given 10 workspaces, when scrolling the sidebar, then all items have consistent rounded styling

#### US-004: Circle Notification Badges
**Description:** As a developer, I want unread notification counts displayed as circle badges on workspace items so that I can spot terminals needing attention at a glance.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Unread count > 0 renders a 16x16pt circle badge positioned leading (before the title)
- [ ] Badge fill color is `#0091FF` (accent blue) for unselected tabs
- [ ] Badge fill color is `Color::WHITE` with 0.25 opacity for the selected tab
- [ ] Badge text is 9pt semibold, white, centered in the circle
- [ ] Given unread count is 0, when rendering, then no badge is shown (no empty circle)
- [ ] Given unread count exceeds 99, when rendering, then badge shows "99+"
- [ ] Given workspace is selected, when badge is present, then badge clears after 200ms grace period

#### US-005: Typography and Visual Hierarchy
**Description:** As a developer, I want proper font sizing and weight across the sidebar so that the visual hierarchy matches a professional terminal multiplexer.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Workspace title font is 12.5pt with `Weight::Semibold` (matching cmux)
- [ ] Subtitle text (directory, pane count) is 10pt with `Weight::Normal`
- [ ] Section header "WORKSPACES" is 10pt with `Weight::Semibold`, uppercase, muted color
- [ ] Sidebar width default is 200px (changed from 220px to match cmux)
- [ ] "+" button matches sidebar accent style with 6pt corner radius
- [ ] Given dark theme, when rendering, then text contrast ratio meets WCAG AA (4.5:1 minimum)

#### US-006: Tab Close Button on Hover
**Description:** As a developer, I want a close button to appear when hovering a sidebar tab so that I can quickly close workspaces.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Hovering a workspace tab reveals an "x" icon (16x16pt) in the trailing position
- [ ] The close icon font is 9pt medium weight
- [ ] Clicking the close icon dispatches `Message::CloseWorkspace(id)`
- [ ] The close icon does not appear for pinned workspaces
- [ ] Given the tab is not hovered, when rendering, then no close button is visible

---

### EP-003: Custom WGPU Shader Terminal Renderer

Build a GPU-instanced terminal cell renderer using iced's Shader widget, replacing the rich_text path for maximum performance. Two instanced draw calls: backgrounds (colored quads) + glyphs (textured quads from atlas).

**Definition of Done:** Terminal panes render via the WGPU Shader widget at < 1ms per frame for a 200x60 grid, with full ANSI color support, correct cursor rendering, and no visual artifacts.

#### US-007: Glyph Atlas with cosmic-text + etagere
**Description:** As a developer, I want a GPU glyph texture atlas so that terminal characters can be rendered via instanced textured quads in a single draw call.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A `GlyphAtlas` struct manages a `wgpu::Texture` (initial 1024x1024, grows to 4096x4096)
- [ ] Glyphs are rasterized on-demand using `cosmic-text` `SwashCache` and packed via `etagere::AtlasAllocator`
- [ ] Each glyph entry stores: UV coordinates (u0, v0, u1, v1), glyph metrics (bearing, advance)
- [ ] A `HashMap<GlyphKey, AtlasEntry>` maps `(char, bold, italic)` to atlas positions
- [ ] Given a glyph not in the atlas, when first encountered, then it is rasterized and packed without dropping the current frame
- [ ] Given the atlas is full, when a new glyph is needed, then the atlas is grown (not evicted) up to 4096x4096
- [ ] Given 95 printable ASCII characters in regular + bold, when pre-warmed, then all fit in a 1024x1024 atlas
- [ ] CJK wide characters occupy 2x cell width in the atlas and render correctly

#### US-008: WGSL Background + Glyph Shaders
**Description:** As a developer, I want WGSL shaders that render terminal cell backgrounds and textured glyphs via instanced draws so that the GPU handles all cell rendering.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] A WGSL vertex shader generates quad vertices from `vertex_index` (0-5 for 2 triangles) and per-instance cell position/size
- [ ] A WGSL fragment shader samples the glyph atlas texture for glyph quads, outputs solid color for background quads
- [ ] Background pass: one instanced draw call with per-cell `(x, y, w, h, bg_rgba)` instance data
- [ ] Glyph pass: one instanced draw call with per-cell `(x, y, w, h, uv_rect, fg_rgba)` instance data
- [ ] A uniform buffer carries the projection matrix (pixel → clip space) updated on resize
- [ ] Given a 200x60 grid (12,000 cells), when both passes complete, then total GPU time is < 1ms (measured via debug timestamps)
- [ ] Given ANSI true color (24-bit RGB), when rendering, then colors are pixel-accurate
- [ ] Given an empty cell (space character), when rendering, then only the background quad is drawn (no glyph quad)

#### US-009: iced Shader Widget Integration
**Description:** As a developer, I want the WGPU terminal renderer integrated as an iced Shader widget so that it composites naturally with the sidebar, tabs, and split layout.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] `TerminalProgram` implements `shader::Program<Message>` with `draw()` producing a `TerminalPrimitive`
- [ ] `TerminalPrimitive` implements `shader::Primitive` with `prepare()` uploading instance buffers and `render()` issuing draw calls
- [ ] The pipeline and atlas are stored in iced's `Storage` and persist across frames
- [ ] `view_terminal_pane()` renders a `Shader::new(TerminalProgram { grid })` instead of rich_text
- [ ] The Shader widget coexists with sidebar and split layout widgets without z-order issues
- [ ] Given window resize, when the Shader widget bounds change, then the projection matrix and grid dimensions update correctly
- [ ] Given the Shader pipeline fails to initialize (no GPU), when starting, then fall back to the rich_text path with a logged warning

#### US-010: Dirty-Cell Tracking and Incremental Updates
**Description:** As a developer, I want only changed cells uploaded to the GPU so that steady-state rendering (cursor blink, partial output) is near-zero cost.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] `TerminalState` tracks a dirty region (set of changed rows) via `alacritty_terminal`'s `damage()` API
- [ ] `prepare()` only re-uploads instance data for rows in the dirty region
- [ ] After upload, `reset_damage()` clears the dirty state
- [ ] Given an idle terminal (no PTY output), when rendering, then zero bytes are uploaded to the GPU per frame
- [ ] Given a single character typed, when rendering, then only the cursor row's instances are re-uploaded
- [ ] Given `cat /dev/urandom | xxd` (full-screen refresh), when rendering, then all rows are uploaded but still < 1ms total

---

### EP-004: Tab Bar and Layout Refinements

Add a horizontal tab bar above the terminal area (matching cmux's Bonsplit pattern) and refine split layout visuals.

**Definition of Done:** Each workspace shows a tab bar with pane tabs, and split dividers are refined to be subtle and consistent with the dark theme.

#### US-011: Horizontal Tab Bar
**Description:** As a developer, I want a horizontal tab bar above the terminal area showing pane names so that I can identify and switch between panes in a workspace.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A tab bar row renders above the split pane area, with one tab per terminal pane in the current workspace
- [ ] Each tab shows: pane title (shell name or running process), close button on hover
- [ ] The active pane's tab is highlighted with the accent color (`#0091FF`) as an underline or background
- [ ] Clicking a tab focuses that pane
- [ ] Tab bar height is 30px (matching cmux's minimal mode strip height)
- [ ] Given 8 panes, when rendering tabs, then tabs are scrollable horizontally if they overflow
- [ ] Given the last pane in a workspace, when its tab close is clicked, then the workspace shows empty state

#### US-012: Refined Split Dividers
**Description:** As a developer, I want split dividers to be subtle and interactive so that they blend with the dark theme and are easy to drag.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Divider width is 2px (reduced from 4px) with color `Color::from_rgba(1.0, 1.0, 1.0, 0.08)`
- [ ] On hover, divider brightens to `Color::from_rgba(1.0, 1.0, 1.0, 0.2)` and cursor changes to col-resize/row-resize
- [ ] Divider is draggable (updates split ratio in real-time)
- [ ] Given a drag past minimum pane size (80px), when released, then ratio clamps correctly

#### US-013: Sidebar Drag-to-Resize
**Description:** As a developer, I want to resize the sidebar by dragging its right edge so that I can allocate space based on my workspace names.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A 4px drag handle on the sidebar's right edge changes cursor to col-resize on hover
- [ ] Dragging resizes the sidebar between 180px (min) and 600px (max), matching cmux
- [ ] The sidebar width persists across app restarts (stored in session.json)
- [ ] Given the sidebar is at minimum width, when dragging further left, then width clamps at 180px

---

### EP-005: Theme and Color System

Establish a cohesive color system with configurable accent color, proper ANSI palette, and consistent dark theme across all UI surfaces.

**Definition of Done:** All UI surfaces use colors from a centralized theme. The accent color is configurable via `paneflow.json`.

#### US-014: Centralized Color Theme
**Description:** As a developer, I want all UI colors to come from a single theme definition so that the app has a consistent visual identity.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A `UiTheme` struct defines: accent, sidebar_bg, content_bg, text_primary, text_secondary, text_muted, border, divider
- [ ] Default values: accent `#0091FF`, sidebar_bg `#1C1C21`, content_bg `#0F0F14`, text_primary `#D9D9DD`, text_secondary `#808086`, text_muted `#4A4A52`, border `#2A2A32`, divider `rgba(255,255,255,0.08)`
- [ ] All view methods read from `self.ui_theme` instead of hardcoded `Color::from_rgb(...)` values
- [ ] The accent color is loadable from `paneflow.json` config field `accent_color` (hex string)
- [ ] Given a config change to `accent_color: "ff6600"`, when hot-reloaded, then all accent-colored elements update
- [ ] Given no config, when starting, then the default `#0091FF` blue theme applies

#### US-015: Notification Ring Animation
**Description:** As a developer, I want terminal bell events to flash a colored ring around the pane so that I notice when agents need attention.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] A bell event in a non-focused pane triggers a blue ring overlay (`#0091FF`, 2.5pt line width, 6pt corner radius)
- [ ] The ring animates: opacity 0 → 1 → 0 → 1 → 0 over 0.9 seconds (matching cmux's flash pattern)
- [ ] A glow effect (blur radius 6pt, opacity 0.6) accompanies the ring
- [ ] Given 100 rapid bell events, when processing, then only one animation plays (coalesced)
- [ ] Given the pane is already focused, when a bell fires, then no ring animation plays

## Functional Requirements

- FR-01: Terminal cells must render with per-cell foreground and background colors (16 ANSI, 256 indexed, 24-bit truecolor)
- FR-02: The GPU Shader renderer must complete a full 200x60 frame in < 1ms GPU time
- FR-03: The glyph atlas must support on-demand rasterization without frame drops
- FR-04: Sidebar tab items must have 6pt corner radius, 10pt horizontal inner padding, 8pt vertical padding
- FR-05: Selected workspace must be highlighted with accent color (`#0091FF` default)
- FR-06: Notification badges must be 16x16pt circles with accent fill and 9pt semibold centered text
- FR-07: The accent color must be configurable via the JSON config file

## Non-Functional Requirements

- **Performance:** GPU frame render time < 1ms for 200x60 grid. rich_text path < 16ms for 80x24. Idle CPU < 1%.
- **Visual Quality:** All UI surfaces use colors from the centralized theme. No hardcoded `Color::from_rgb()` in view methods.
- **Compatibility:** Must work on iced 0.13 with wgpu 0.19. No dependencies on glyphon (incompatible), iced_term (needs 0.14), or macOS-only APIs.
- **Accessibility:** Text contrast ratio meets WCAG AA (4.5:1) for all primary text on dark backgrounds.
- **Binary size:** Release binary remains < 30 MB after adding cosmic-text/etagere dependencies.

## Edge Cases & Error States

| # | Scenario | Expected Behavior |
|---|----------|-------------------|
| 1 | GPU context lost (driver crash) | Fall back to rich_text rendering path, log warning |
| 2 | Glyph not in atlas (rare Unicode) | Rasterize on demand, pack into atlas, render next frame |
| 3 | Atlas full (4096x4096 exhausted) | Log warning, render missing glyphs as `?` placeholder |
| 4 | 200+ column terminal | Shader handles via instanced draw; rich_text may jank — acceptable |
| 5 | CJK wide character | 2x cell width in atlas, 2-column instance in draw call |
| 6 | Config accent_color invalid hex | Log warning, retain previous valid accent color |
| 7 | 50+ workspaces in sidebar | Scrollable list, no rendering degradation |
| 8 | Notification badge count > 99 | Display "99+" |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | iced 0.13 Shader widget API differs from docs/examples (0.15-dev) | Medium | High | US-009 is the integration spike. If API mismatch, fall back to Canvas+Cache path. |
| 2 | cosmic-text glyph rasterization too slow for on-demand atlas fills | Low | Medium | Pre-warm atlas with 95 ASCII chars + bold variants at startup. Benchmark in US-007. |
| 3 | rich_text with 200+ spans jank on large terminals | Medium | Low | Acceptable — Shader path (EP-003) replaces it. rich_text is the interim solution. |
| 4 | wgpu 0.19 lacks features needed for instanced rendering | Low | High | wgpu 0.19 fully supports instanced draws. Confirmed in Alacritty's renderer. |

## Non-Goals

- **iced 0.14 upgrade:** Separate migration effort, not in scope for this PRD.
- **macOS-specific visuals:** No NSVisualEffectView backdrop blur, no vibrancy. Cross-platform dark theme only.
- **Mouse selection rendering:** Selection highlighting is tracked in the v2 PRD (US-007). This PRD focuses on cell coloring and GPU rendering.
- **Sixel/image protocol:** Image rendering in terminal cells is deferred to v3.
- **Custom font loading:** Uses system monospace font via cosmic-text. Custom font file loading deferred.

## Files NOT to Modify

- `crates/paneflow-core/` — Domain model unchanged
- `crates/paneflow-ipc/` — Socket server unchanged
- `crates/paneflow-cli/` — CLI unchanged
- `crates/paneflow-terminal/src/bridge.rs` — PTY bridge unchanged (output pipeline already optimized)
- `crates/paneflow-terminal/src/pty_manager.rs` — PTY manager unchanged

## Technical Considerations

- **Shader widget Storage API:** The exact `Storage::get::<T>()` / `Storage::store(T)` method signatures need validation against iced 0.13 (not 0.15-dev docs). US-009 validates this. Fallback: use Canvas+Cache if Storage API differs.
- **Atlas growth strategy:** Start at 1024x1024, double on exhaustion up to 4096x4096. No LRU eviction (terminal glyph set is bounded — ~200 unique characters in practice).
- **Instance buffer layout:** `#[repr(C)]` struct with `bytemuck::Pod + Zeroable`. Upload via `queue.write_buffer()` on each dirty frame.
- **Projection matrix:** Orthographic 2D projection from pixel coordinates to clip space. Updated on window resize via uniform buffer.

## Success Metrics

| Metric | Current | Target | Timeframe |
|--------|---------|--------|-----------|
| ANSI colors visible | 0% (monochrome) | 100% (16/256/truecolor) | Week 1 |
| GPU frame time (200x60) | N/A | < 1ms | Week 3 |
| Sidebar visual parity with cmux | ~20% | > 90% | Week 2 |
| UI theme consistency (hardcoded colors) | 30+ hardcoded Color values | 0 (all from UiTheme) | Week 2 |
| Release binary size | 20 MB | < 30 MB | Maintained |

## Open Questions

- **cosmic-text availability in iced 0.13:** iced uses cosmic-text internally — can we import `cosmic-text` as a direct dependency without version conflicts? Engineering to validate in US-007.
- **Shader widget vs Canvas+Cache performance delta:** Is the Shader widget meaningfully faster than Canvas+Cache for terminal rendering? US-009 benchmark will determine if the complexity is justified.
- **Atlas texture format:** `R8Unorm` (grayscale alpha) vs `Rgba8Unorm` (full color for emoji/images)? Recommend `R8Unorm` for v1, upgrade if image support is added later.
[/PRD]
