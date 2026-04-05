[PRD]
# PRD: Terminal Rendering Parity

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-05 | Claude + Arthur | Initial draft from comparative analysis of PaneFlow, Zed, and Ghostty |

## Problem Statement

PaneFlow's terminal rendering is visibly inferior to Ghostty and Zed, causing several concrete problems:

1. **Block cursor hides the character underneath** — PaneFlow paints a solid opaque quad, making the cursor position unreadable. Every major terminal (Ghostty, Zed, Kitty, WezTerm, Alacritty) renders the character in inverted color under a block cursor.
2. **Emoji, Nerd Font icons, and CJK characters render as tofu (□)** — PaneFlow configures `fallbacks: None` on its font, so any glyph not in Noto Sans Mono produces a missing-glyph rectangle. Zero-width combining characters are also dropped entirely, breaking composed emoji (flags, skin tones, ZWJ sequences).
3. **DIM/faint text (SGR 2) has no visual effect** — CLI tools like Claude Code, bat, delta, and many prompts use faint text. PaneFlow never checks `CellFlags::DIM`, so faint text renders at full opacity.
4. **All underline styles are identical** — undercurl, double underline, dotted, and dashed are all collapsed to a 1px solid line with no color. Neovim LSP diagnostics, which use undercurl with custom colors, are indistinguishable from regular underlines.
5. **Text can be unreadable on similar backgrounds** — no minimum contrast enforcement. Dark text on a dark background is invisible.
6. **Visual polish gaps** — no left gutter margin (text stuck to edge), non-pixel-aligned background rects (sub-pixel bleeding), theme config re-read from disk every frame.

**Why now:** PaneFlow's v2 GPUI rewrite is feature-complete (19 terminal stories + 12 title bar stories delivered). The next adoption barrier is rendering fidelity — users compare PaneFlow directly against Ghostty and find it lacking. Zed's terminal uses the exact same GPUI framework and has solved all these problems, proving they are solvable without architectural changes.

## Overview

This PRD brings PaneFlow's terminal rendering to parity with Zed's terminal implementation, using the same GPUI APIs and patterns. The improvements target `terminal_element.rs` (the custom GPUI Element that paints terminal cells) and `terminal.rs` (terminal state management).

The approach is to port proven techniques from Zed's `terminal_element.rs` (2342 lines) into PaneFlow's equivalent (960 lines). Both use GPUI's `shape_line()`, `paint_quad()`, `TextRun`, and `UnderlineStyle` — the API surface is identical. No new crate dependencies are required.

Key decisions from brainstorming:
- **Zed-first approach**: port Zed's GPUI patterns directly rather than Ghostty's GPU-native approach (which would require rewriting the entire renderer)
- **Hardcoded font fallbacks initially**: start with a sensible default fallback chain, make configurable via paneflow.json in a later story
- **APCA contrast algorithm**: use Zed's `ensure_minimum_contrast` rather than Ghostty's WCAG 2.0 ratio (more perceptually accurate, already available in GPUI ecosystem)
- **Accept GPUI underline limitations**: GPUI's `UnderlineStyle` supports `wavy: bool` but not dotted/dashed variants — collapse those to solid (same trade-off Zed makes)

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| SGR attribute coverage | 90% of attributes render correctly (DIM, underline variants, inverse, bold, italic) | 100% including rare attributes |
| Emoji/icon rendering | Composed emoji and Nerd Font icons render via fallback chain | User-configurable fallback fonts via paneflow.json |
| Visual parity with Zed terminal | Block cursor, underlines, contrast match Zed's output | Indistinguishable from Zed terminal for standard content |

## Target Users

### Developer (daily-driver user)
- **Role:** Software engineer using PaneFlow as their primary terminal multiplexer
- **Behaviors:** Runs Claude Code, neovim, lazygit, htop, bat, delta — tools that use full SGR attributes, emoji, Nerd Font icons, and undercurl diagnostics
- **Pain points:** Emoji show as tofu, cursor hides text, undercurl looks like plain underline, faint text is not faint, some text is unreadable on dark backgrounds
- **Current workaround:** Uses Ghostty or Kitty for tasks requiring proper rendering, PaneFlow only for multiplexing
- **Success looks like:** Can use PaneFlow exclusively without noticing rendering differences from Ghostty

## Research Findings

Key findings from deep comparative analysis of three codebases:

### Competitive Context
- **Ghostty:** GPU-native renderer with custom Metal/OpenGL shaders, HarfBuzz shaping, skyline-packed glyph atlas, per-row dirty tracking, sprite-based decorations (each underline style is a distinct sprite). Font fallback via `CodepointResolver` with system font discovery. Minimum contrast computed in vertex shader. Cursor character rendered by buffer ordering trick in instanced draw call.
- **Zed terminal:** Same GPUI framework as PaneFlow. Configurable font fallbacks from settings. APCA minimum contrast via `ensure_minimum_contrast()`. DIM flag handled as `fg.a *= 0.7`. Undercurl via `wavy: true` in `UnderlineStyle`. Zero-width chars via `cell.zerowidth()` API. `CursorLayout` renders shaped character text under block cursor. Decorative character detection skips contrast for box-drawing/Powerline. Left gutter of 1 cell width. 2D background region merging.
- **Market gap:** PaneFlow implements none of the above rendering features despite using the same framework as Zed.

### Best Practices Applied
- Block cursor text inversion is universal across all modern terminals — absence is a bug, not a missing feature
- Font fallback chains with at least emoji + symbol coverage are mandatory for Unicode support
- DIM alpha multiplier of 0.7 is the cross-terminal consensus (Ghostty 0.69, Alacritty 0.66, Kitty 0.75, Zed 0.7)
- Minimum contrast prevents accessibility failures without user intervention

*Research based on direct code analysis of PaneFlow (terminal_element.rs:960 lines), Zed (terminal_element.rs:2342 lines), and Ghostty (renderer/generic.zig + shaders: ~3000 lines).*

## Assumptions & Constraints

### Assumptions (to validate)
- GPUI's font resolution handles fallback chains on Linux (Zed validates this, but PaneFlow's font setup may differ)
- `ensure_minimum_contrast` from `ui::utils` is accessible from PaneFlow's crate (may need to reimplement if not exported)
- Noto Color Emoji or a similar emoji font is installed on target systems

### Hard Constraints
- Must use GPUI's `Element` trait (3-phase: request_layout → prepaint → paint) — no custom GPU code
- `alacritty_terminal` is Zed's fork (rev 9d9640d4) — not upstream Alacritty. APIs like `cell.zerowidth()` must be verified against this specific revision
- GPUI's `UnderlineStyle` only supports `wavy: bool` — no dotted/dashed/double distinction at the framework level
- Single file scope: all rendering changes are in `terminal_element.rs` (~960 lines) and `terminal.rs`
- No new crate dependencies — use only what GPUI and alacritty_terminal already provide

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --workspace -- -D warnings` - No warnings (including new code)
- `cargo fmt --check` - Formatting compliance
- `cargo test --workspace` - All existing tests pass
- `cargo build` - Successful compilation

For all stories, additional visual verification:
- Run PaneFlow and execute the relevant SGR test commands to confirm visual correctness
- Compare output side-by-side with Ghostty or Zed terminal for the same content

## Epics & User Stories

### EP-001: Core Text Rendering

Improve the text rendering pipeline to correctly handle font fallbacks, combining characters, faint text, and underline styles. These are the foundation — without them, terminal output from modern CLI tools is visually broken.

**Definition of Done:** Emoji, Nerd Font icons, and combining characters render correctly. DIM text is visually faint. Underlines have correct color and undercurl is wavy.

#### US-001: Font fallback chain
**Description:** As a developer, I want emoji, Nerd Font icons, and CJK characters to render correctly so that CLI tool output (git status icons, prompt decorations, emoji in messages) is readable instead of showing tofu rectangles.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `base_font()` returns a `Font` with a `fallbacks: Some(vec![...])` containing at least: an emoji font family, a Nerd Font / Symbols family, and a generic sans-serif family
- [ ] Running `echo "🦀 🇫🇷 👨‍💻  "` in PaneFlow renders all glyphs (crab emoji, flag, ZWJ emoji, Nerd Font icons) without tofu rectangles
- [ ] Given a system without the primary font (Noto Sans Mono), the terminal still renders text using fallback fonts rather than crashing or showing all-tofu
- [ ] The fallback list is defined as constants (not hardcoded strings scattered in code) for future configurability

#### US-002: Zero-width and combining character support
**Description:** As a developer, I want composed emoji (flags, skin tones, ZWJ sequences) and characters with combining diacritical marks to render correctly so that internationalized content displays properly.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001 (fallback fonts needed for emoji glyphs)

**Acceptance Criteria:**
- [ ] The batch loop in `build_layout` reads `cell.zerowidth()` (from alacritty_terminal's `Cell` API) and appends combining characters to the current `BatchedTextRun` text without incrementing the cell count
- [ ] Space cells following cells with non-empty `zerowidth()` extras are skipped (emoji variation sequence handling, matching Zed's pattern)
- [ ] Running `echo "🇫🇷 👨‍👩‍👧‍👦 é à ñ"` renders correctly: flag as single glyph, family emoji as single glyph, accented chars with proper diacriticals
- [ ] Given a cell with 10+ combining marks (stress test), the terminal does not crash or corrupt the grid — marks are appended and shaped by GPUI's text system
- [ ] `BatchedTextRun.style.len` correctly accounts for UTF-8 byte length of zero-width characters (not just cell count)

#### US-003: DIM (faint) flag handling
**Description:** As a developer, I want text with the SGR 2 (faint/dim) attribute to appear visually dimmed so that CLI tools using faint text for secondary information render correctly.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] In the cell processing loop, when `flags.contains(CellFlags::DIM)` is true, the foreground color's alpha channel is multiplied by 0.7
- [ ] The DIM check is applied AFTER the INVERSE swap (so inverted dim text dims the swapped foreground)
- [ ] Running `printf '\e[2mfaint text\e[0m normal text'` shows visibly dimmer text for "faint text" compared to "normal text"
- [ ] Given text that is both DIM and has a custom foreground color, the dimming is applied to the custom color (not to a default)

#### US-004: Underline color and undercurl
**Description:** As a developer, I want underlines to use the text's foreground color and undercurl (wavy underline) to render as a wavy line so that neovim LSP diagnostics and other tools using SGR 4:3 (undercurl) are visually distinct from regular underlines.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `UnderlineStyle` in `BatchAccumulator::flush()` sets `color: Some(fg_color)` instead of `color: None`
- [ ] When `CellFlags::UNDERCURL` is set, `UnderlineStyle` sets `wavy: true`
- [ ] `StrikethroughStyle` sets `color: Some(fg_color)` instead of `color: None`
- [ ] Running `printf '\e[4:3mwavy\e[0m \e[4mstraight\e[0m'` shows visibly different underline styles (wavy vs straight)
- [ ] Given text with a colored underline SGR sequence (e.g., red undercurl from neovim), the underline renders in the specified color
- [ ] The `wavy` flag does NOT affect non-UNDERCURL underline types (UNDERLINE, DOUBLE_UNDERLINE, DOTTED, DASHED remain straight)

---

### EP-002: Cursor Rendering

Fix the block cursor to show the character underneath and handle wide characters properly. The current opaque quad makes the cursor position unreadable.

**Definition of Done:** Block cursor shows the character in inverted color. Cursor correctly covers wide characters. Unfocused cursor remains hollow block.

#### US-005: Block cursor with inverted text
**Description:** As a developer, I want the block cursor to display the character underneath in the terminal's background color so that I can read what character the cursor is on, matching the behavior of every major terminal emulator.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] In `build_layout`, the character at the cursor position is captured (from `term.grid()[cursor.point].c`) and stored in `CursorInfo`
- [ ] In `paint()`, for `CursorShape::Block`: the cursor quad is painted first, then the character is shaped via `shape_line()` and painted on top using the terminal's background color as text color
- [ ] The shaped character respects the cursor cell's font attributes (bold, italic) — not always normal weight
- [ ] Given a cursor on a space character, only the cursor quad is painted (no text shaping for whitespace)
- [ ] Given a cursor on an emoji or wide character, the text is shaped and rendered correctly within the cursor bounds
- [ ] Unfocused terminals still render `HollowBlock` (outline only, no text) — unchanged from current behavior

#### US-006: Wide character cursor sizing
**Description:** As a developer, I want the cursor to span the full width of wide characters (CJK, emoji) so that the cursor visually covers the entire glyph.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Cursor width for wide characters uses `max(shaped_text_width, cell_width * 2)` rather than always `cell_width * 2` (handles emoji that may be wider than 2 cells)
- [ ] Given a cursor on a CJK character (e.g., `echo "中文"`), the block cursor covers both cells
- [ ] Given a cursor on a single-width ASCII character, cursor width is exactly `cell_width` (no regression)

---

### EP-003: Color & Contrast

Enforce minimum foreground/background contrast and correctly handle decorative characters that should bypass contrast adjustment.

**Definition of Done:** Text is always readable on any background. Box-drawing and Powerline characters preserve their exact colors.

#### US-007: Minimum contrast enforcement
**Description:** As a developer, I want automatic minimum contrast between text and background colors so that text is always readable, even when color schemes produce low-contrast combinations.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] An `ensure_minimum_contrast(fg: Hsla, bg: Hsla, ratio: f32) -> Hsla` function is implemented (port from Zed's APCA-based approach or implement a simplified WCAG 2.0 luminance ratio)
- [ ] The function is called in the cell processing loop for every non-decorative cell, adjusting fg if contrast is below the threshold
- [ ] Default minimum contrast ratio is defined as a constant (e.g., 4.5:1 WCAG AA)
- [ ] Given dark gray text (#333) on a dark background (#1e1e2e), the text color is lightened to meet the contrast threshold
- [ ] Given white text on a light background, the text color is darkened appropriately
- [ ] Given text that already meets the contrast threshold, no color change occurs (idempotent)

#### US-008: Decorative character detection
**Description:** As a developer, I want box-drawing characters, block elements, and Powerline separators to preserve their exact colors without contrast adjustment so that TUI borders, status bars, and prompt decorations render correctly.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007 (only needed when contrast enforcement exists)

**Acceptance Criteria:**
- [ ] An `is_decorative_character(ch: char) -> bool` function identifies: Box Drawing (U+2500–U+257F), Block Elements (U+2580–U+259F), Geometric Shapes (U+25A0–U+25FF), Powerline separators (U+E0B0–U+E0D7)
- [ ] The minimum contrast function is NOT applied to cells where `is_decorative_character(cell.c)` returns true
- [ ] Given a Powerline prompt with colored separators (, ), the separator colors match the adjacent segment colors exactly (no contrast adjustment)
- [ ] Given a TUI app (htop, lazygit) with box-drawing borders, border colors are preserved

---

### EP-004: Layout Precision

Fix sub-pixel rendering artifacts, add left margin, and optimize per-frame overhead.

**Definition of Done:** Background rects are pixel-aligned. Terminal content has a comfortable left margin. Theme is cached per-frame.

#### US-009: Pixel-aligned background rects
**Description:** As a developer, I want cell background rectangles to be pixel-aligned so that there are no visible gaps or bleeding between adjacent colored cells.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] In `paint()`, background rect X positions use `.floor()`: `(origin.x + cell_width * rect.col as f32).floor()`
- [ ] Background rect widths use `.ceil()`: `(cell_width * rect.num_cols as f32).ceil()`
- [ ] Given a terminal with alternating colored cells (e.g., a colored status bar), no thin gaps or color bleeding is visible between cells at any window size

#### US-010: Left gutter margin
**Description:** As a developer, I want a left margin in the terminal area so that text is not flush against the pane edge, matching the visual spacing of Zed's terminal.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A gutter of `cell_width` pixels is added to the left of the terminal content area
- [ ] Grid column calculation accounts for the gutter: `desired_cols = ((bounds.width - gutter) / cell_width).floor()`
- [ ] All paint coordinates (text, backgrounds, cursor, selection) are offset by gutter
- [ ] Mouse-to-grid coordinate conversion in `TerminalView` accounts for gutter offset
- [ ] Given a split with two terminals side by side, both terminals have consistent left gutters

#### US-011: Theme caching
**Description:** As a developer, I want the terminal theme to be cached instead of re-read from disk on every frame so that rendering performance is not impacted by file I/O during paint.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `active_theme()` caches the parsed `TerminalTheme` and only re-reads from disk when the config file's mtime has changed (or on a configurable interval, e.g., 500ms)
- [ ] Theme changes still take effect within 1 second of saving `paneflow.json`
- [ ] Given rapid terminal output (e.g., `cat large_file.txt`), `build_layout` does not trigger file I/O on every frame
- [ ] Given a corrupted or missing config file during a cached period, the last valid theme is used (no crash)

---

### EP-005: Configurability

Make font and line height configurable via paneflow.json instead of compile-time constants.

**Definition of Done:** Users can set font family, font size, and line height in paneflow.json with hot-reload.

#### US-012: Configurable line height
**Description:** As a developer, I want to configure the terminal line height in paneflow.json so that I can adjust vertical spacing to my preference rather than being locked to the hardcoded 1.4x multiplier.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `paneflow.json` schema accepts `"line_height": <float>` (default: 1.4, valid range: 1.0–2.5)
- [ ] `measure_cell()` reads `line_height` from config instead of using the hardcoded `FONT_SIZE * 1.4`
- [ ] Changing `line_height` in paneflow.json and saving triggers a terminal re-layout within 1 second (hot-reload)
- [ ] Given `line_height: 1.0` (minimum), text lines do not overlap and the terminal remains usable
- [ ] Given `line_height: 2.5` (maximum), the terminal is spacious but functional
- [ ] Given an invalid value (e.g., `line_height: -1`), the default 1.4 is used and a warning is logged

#### US-013: Configurable font family and size
**Description:** As a developer, I want to configure the terminal font family and size in paneflow.json so that I can use my preferred coding font (JetBrains Mono, Fira Code, etc.) instead of being locked to Noto Sans Mono 14px.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-012 (line height config infrastructure)

**Acceptance Criteria:**
- [ ] `paneflow.json` schema accepts `"font_family": "<string>"` (default: "Noto Sans Mono") and `"font_size": <float>` (default: 14.0, valid range: 8.0–32.0)
- [ ] `base_font()` and `font_size()` read from config instead of using compile-time constants
- [ ] `measure_cell()` uses the configured font for cell width measurement (the `'m'` advance width)
- [ ] Changing font family or size in paneflow.json triggers a terminal re-layout and PTY resize (SIGWINCH) within 1 second
- [ ] Given a proportional font family (e.g., "Arial"), the terminal still functions (characters are forced to cell width by GPUI's `force_width` parameter)
- [ ] Given a font family not installed on the system, the terminal falls back to the default font and logs a warning

## Functional Requirements

- FR-01: The terminal renderer must support the full SGR attribute set: bold, italic, dim, underline (all variants), strikethrough, inverse, and all ANSI/256/truecolor colors
- FR-02: The font stack must include fallback families for emoji, symbols, and CJK characters
- FR-03: The block cursor must render the character underneath in inverted color
- FR-04: The renderer must enforce minimum foreground/background contrast ratio while exempting decorative characters
- FR-05: All rendering coordinates must be pixel-aligned to prevent sub-pixel artifacts
- FR-06: Font family, font size, and line height must be configurable via paneflow.json with hot-reload

## Non-Functional Requirements

- **Performance:** Paint time for a full 80×24 terminal must remain under 2ms (current baseline: ~1ms). No regression from new features.
- **Performance:** Theme config must not trigger file I/O more than 2 times per second during active rendering
- **Compatibility:** All rendering improvements must work on both Wayland and X11 (the two supported display servers)
- **Memory:** Font fallback chain must not increase per-terminal memory usage by more than 5MB (glyph cache growth)
- **Latency:** Keystroke-to-pixel latency must remain under 16ms (current baseline: ~8ms per PANEFLOW_LATENCY_PROBE)

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Fallback font missing | System has no emoji font installed | Render tofu for emoji but all ASCII text remains functional; log warning at startup | `[warn] No emoji fallback font found — emoji may not render` |
| 2 | Excessive combining marks | A cell with 50+ zero-width combining characters | Append all to the text run; let GPUI's shaper handle truncation. No crash or grid corruption. | — |
| 3 | Resize during rapid output | Window resize while PTY floods output | Resize is processed atomically inside the FairMutex lock; no torn state between grid dimensions and content | — |
| 4 | Corrupted config file | paneflow.json has invalid JSON while cached theme is valid | Continue using last valid cached theme; log parse error | `[warn] Failed to parse paneflow.json: {error}` |
| 5 | Unknown font family | User configures a font not installed on the system | Fall back to "Noto Sans Mono" (or system default); log warning | `[warn] Font "X" not found, using fallback` |
| 6 | Wide char at last column | CJK character placed at column N-1 of an N-column terminal | Character wraps to next line (handled by alacritty_terminal); cursor and background rects follow wrapping | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | `cell.zerowidth()` API may differ in Zed's Alacritty fork vs upstream | Medium | High | Verify API exists in rev 9d9640d4 before implementing US-002; add a test that calls `zerowidth()` on a Cell |
| 2 | `ensure_minimum_contrast` from Zed's `ui` crate may not be importable | Medium | Medium | If not accessible, implement a simplified version using WCAG 2.0 luminance ratio (simpler, well-documented algorithm) |
| 3 | Font fallback chain may not resolve consistently across Linux distros | Medium | Medium | Use generic family names ("emoji", "sans-serif") as final fallbacks; document required font packages in README |
| 4 | Performance regression from per-cell contrast calculation | Low | Medium | Profile with PANEFLOW_LATENCY_PROBE before/after; if regression >1ms, add early-exit for high-contrast cells |
| 5 | GPUI's `wavy` underline rendering may look different from Ghostty's undercurl | Low | Low | Accept visual differences — GPUI's rendering is still clearly wavy and distinct from straight underlines |

## Non-Goals

Explicit boundaries — what this version does NOT include:

- **GPU-native rendering (Ghostty approach)** — no custom shaders, glyph atlases, or instanced draw calls. GPUI's text system is sufficient and maintains framework compatibility.
- **Dotted/dashed underline distinction** — GPUI's `UnderlineStyle` only supports `wavy: bool`. All non-curly underlines render as solid (same limitation as Zed).
- **Subpixel text positioning** — handled internally by GPUI's text system. No custom subpixel logic in PaneFlow.
- **Font ligature support** — terminals traditionally disable ligatures. If needed later, set `FontFeatures::disable_ligatures()` explicitly.
- **Selection rounded corners** — Zed uses `HighlightedRange` with corner radius. Cosmetic improvement deferred to future work.
- **Scrollbar interactivity** — the scrollbar is currently a visual indicator only. Making it interactive is out of scope.

## Files NOT to Modify

- `src-app/src/ipc.rs` — IPC socket server, unrelated to rendering
- `src-app/src/split.rs` — Split tree layout, no rendering logic
- `src-app/src/workspace.rs` — Workspace management, no rendering logic
- `crates/paneflow-config/src/lib.rs` — Config schema changes only via US-012/US-013 (not in earlier stories)

## Reference Implementations

Source files in Zed and Ghostty that implement the features described in this PRD. Use these as blueprints when implementing each user story.

### Zed (`/home/arthur/dev/zed`)

| File | Relevance | Stories |
|------|-----------|---------|
| `crates/terminal_view/src/terminal_element.rs` | **Primary reference.** Full terminal Element impl (2342 lines). `layout_grid()` at :332 — cell batching, zero-width chars, background merging. `cell_style()` at :543 — DIM, underline color/wavy, bold/italic, minimum contrast, decorative char detection. `is_decorative_character()` at :525. Cursor setup at :1155. | US-001–US-013 |
| `crates/terminal/src/terminal.rs` | Terminal state, `make_content()` at :1632 — grid snapshot from alacritty. `get_color_at_index()` at :2467 — 256-color resolution. | US-002, US-007 |
| `crates/gpui/src/text_system/line.rs` | `ShapedLine::paint` → `paint_line()` at :326 — glyph painting, decoration run tracking, emoji dispatch (`paint_glyph` vs `paint_emoji`). | US-001, US-004 |
| `crates/gpui/src/text_system/line_layout.rs` | `LineLayoutCache` — two-frame layout cache. `layout_line()` at :612 — `force_width` enforcement snaps glyphs to cell grid. | US-001, US-002 |
| `crates/gpui/src/window.rs` | `paint_glyph()` at :3332 — subpixel variant positioning. `paint_underline()` at :3261 — wavy height = thickness×3. `paint_strikethrough()` at :3296. | US-004, US-005 |
| `crates/editor/src/element.rs` | `CursorLayout` at :12030 — `paint()` at :12133 paints quad + shaped text on block cursor. | US-005, US-006 |
| `crates/ui/src/utils/apca_contrast.rs` | `ensure_minimum_contrast()` at :147 — APCA contrast algorithm. | US-007 |
| `crates/terminal_view/src/terminal_element.rs:890-906` | Font settings resolution — family, fallbacks, features, weight, size from `TerminalSettings`. | US-001, US-012, US-013 |

### Ghostty (`/home/arthur/dev/ghostty`)

| File | Relevance | Stories |
|------|-----------|---------|
| `src/renderer/generic.zig` | Main renderer (3000+ lines). `rebuildRow()` at :2610 — cell-by-cell glyph resolution with font fallback, underline/overline/strikethrough as separate sprites, DIM via alpha. `drawFrame()` at :1430 — frame lifecycle, atlas upload, render pass ordering. `addGlyph()` — atlas insertion. Cursor layering at :3251 (block cursor drawn BEFORE text in buffer, glyph color swapped in shader). | US-001–US-008 |
| `src/renderer/cell.zig` | `Contents` struct — cell buffer management. Cursor at buffer indices 0 (behind text) and rows+1 (on top). `noMinContrast` flag for decorative chars at :297. `isGraphicsElement()` at :314. Symbol constraint width at :253. | US-005, US-007, US-008 |
| `src/renderer/cursor.zig` | Cursor style decision logic at :36 — password lock, hollow unfocused, blink, preedit. | US-005 |
| `src/renderer/shaders/shaders.metal` | Metal shaders (853 lines). `contrasted_color()` at :111 — WCAG 2.0 contrast in shader. `cell_bg_fragment()` at :451 — full-screen bg with per-cell color lookup. `cell_text_vertex()` at :556 — glyph positioning, cursor color swap at :649. `load_color()` at :136 — sRGB→Display P3, linear blending. | US-007, reference for color pipeline |
| `src/renderer/shaders/glsl/cell_text.v.glsl` | OpenGL vertex shader — min contrast at :133, cursor glyph color swap at :143. | US-007 |
| `src/font/shaper/harfbuzz.zig` | HarfBuzz shaping at :130 — run segmentation, ligature detection heuristic at :178, 26.6 fixed-point → pixel conversion at :231. | US-001, US-002 (reference for shaping quality) |
| `src/font/face/freetype.zig` | FreeType rasterization — synthetic bold via `FT_Outline_Embolden` at :439, synthetic italic via 12° shear at :537, glyph constraint/fitting at :479. | Reference for glyph quality |
| `src/font/sprite/draw/special.zig` | Sprite decorations — each underline variant (single, double, dotted, dashed, curly) as distinct sprites. Underline position at :13, curly path at :74+. | US-004 (reference for underline variants) |
| `src/font/CodepointResolver.zig` | Font fallback chain — priority-ordered collection, deferred lazy loading, system font discovery as last resort. | US-001 (reference for fallback architecture) |
| `src/font/Atlas.zig` | Skyline bin-packing glyph atlas — two textures (grayscale + color emoji). `modified` atomic for lazy GPU upload. | Reference for atlas performance patterns |

### PaneFlow (`/home/arthur/dev/paneflow`) — Files to Modify

| File | Current lines | Scope |
|------|--------------|-------|
| `src-app/src/terminal_element.rs` | 960 | Primary target — all rendering changes (US-001–US-011) |
| `src-app/src/terminal.rs` | ~600 | Cursor char capture, zero-width API verification (US-002, US-005) |
| `src-app/src/theme.rs` | ~200 | Theme caching (US-011) |
| `crates/paneflow-config/src/lib.rs` | ~300 | Config schema for font/line_height (US-012, US-013 only) |

## Technical Considerations

- **Architecture:** All rendering changes are in `terminal_element.rs` (Element trait impl) and `terminal.rs` (state management). No architectural changes needed — same 3-phase GPUI pattern.
- **Font fallback approach:** GPUI's `Font` struct has a `fallbacks: Option<Vec<SharedString>>` field. Setting this is the entirety of the font fallback implementation. Engineering to verify that GPUI resolves fallbacks correctly on Linux (Wayland + X11).
- **Minimum contrast:** Port Zed's `ensure_minimum_contrast` or implement WCAG 2.0 luminance ratio. Trade-off: APCA is more perceptually accurate but more complex. WCAG 2.0 is simpler and well-documented. Engineering to decide.
- **Zero-width chars:** Requires verifying that Zed's Alacritty fork (rev 9d9640d4) exposes `Cell::zerowidth()`. If not available, check `Cell::extra()` or similar API in that revision.
- **Theme caching:** Replace `active_theme()` with a struct that holds `(TerminalTheme, SystemTime)` and checks mtime before re-parsing. No crate dependency needed.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| SGR attributes rendered correctly | ~60% (missing DIM, undercurl, colored underline) | 95%+ | Month-1 | Manual test with SGR test script |
| Emoji rendering success rate | 0% (all tofu) | 90%+ (common emoji) | Month-1 | Run emoji test set, count rendered vs tofu |
| Paint time (80×24 full screen) | ~1ms | <2ms | Month-1 | PANEFLOW_LATENCY_PROBE |
| Keystroke-to-pixel latency | ~8ms | <16ms | Month-1 | PANEFLOW_LATENCY_PROBE |
| User-reported rendering issues | N/A (new) | <5 open issues | Month-6 | GitHub issue tracker |

## Open Questions

- Should we implement APCA (Zed's approach) or WCAG 2.0 luminance ratio for minimum contrast? APCA is more perceptually accurate but harder to implement standalone. Engineering input needed.
- Does Zed's Alacritty fork (rev 9d9640d4) expose `Cell::zerowidth()`? Needs verification before US-002 implementation. If not, what is the equivalent API?
- What emoji font should be the first fallback? "Noto Color Emoji" is standard on most Linux distros but not guaranteed. Should we bundle a fallback or just document the dependency?
[/PRD]
