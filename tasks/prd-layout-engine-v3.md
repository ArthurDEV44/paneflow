[PRD]
# PRD: PaneFlow Layout Engine v3 — Hybrid N-ary Tree Layout

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-04 | Arthur (via Claude) | Initial draft — based on deep tmux analysis + competitive research |

## Problem Statement

1. **Incorrect drag-to-resize math.** The current layout engine uses a hardcoded 800px container size estimate (`split.rs:139`) to convert mouse pixel deltas into ratio changes. On any window that isn't exactly 800px wide, resize drags overshoot or undershoot, making precise pane sizing impossible.

2. **Binary tree creates artificial nesting.** The current `SplitNode` is a strict binary tree — every split creates a new depth level. Four side-by-side panes produce a 3-level deep tree (`Split(Split(Split(A,B),C),D)`) instead of a flat `Container{A,B,C,D}`. This makes layout presets impossible, complicates focus navigation, and creates "phantom container" bugs when closing panes.

3. **No constraint propagation.** When a pane is resized, the current system clamps the ratio to 0.1–0.9 but does not check whether child subtrees can actually absorb the change. This allows panes to be crushed below the 80px minimum in nested layouts.

4. **Missing power-user features.** No zoom (tmux `Ctrl+Z`), no layout presets (tmux `even-horizontal`/`tiled`), no layout serialization (save/restore sessions). These are expected features in every major terminal multiplexer (tmux, Zellij, Kitty, WezTerm).

**Why now:** PaneFlow's v2 GPUI migration is complete (19 stories delivered). The layout system is the last major architectural debt before the app can compete with tmux/Zellij on feature parity. Zellij's PR #4021 confirms that float-ratio layout systems accumulate irreversible rounding corruption — fixing this now prevents the problem from growing with the codebase.

## Overview

Replace PaneFlow's binary-tree flexbox split system (`split.rs`, ~200 lines) with a hybrid N-ary tree layout engine inspired by tmux's architecture. The new engine borrows three key concepts from tmux:

1. **N-ary tree structure** — containers hold 2+ children in a `Vec`, not a pair of `Box<SplitNode>`. Splitting in the same direction as the parent adds a sibling rather than nesting. Closing a pane collapses single-child containers automatically.

2. **Constraint propagation** — a recursive `resize_check()` function computes how much space each subtree can yield before any resize is applied. This prevents minimum-size violations and enables correct drag-to-resize.

3. **Zoom via layout swap** — the entire layout tree is saved and replaced with a single full-window pane. Un-zoom restores the saved tree exactly.

The engine keeps GPUI's Taffy-backed flexbox for rendering (no custom coordinate computation) but adds a constraint layer on top. Layout presets destroy and rebuild the tree (tmux's proven approach). Serialization leverages the existing `LayoutNode` schema in `paneflow-config`.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Drag-to-resize accuracy | Pixel-accurate on any window size | Same |
| Maximum pane count without layout bugs | 32 panes, 0 constraint violations | 32 panes, 0 violations |
| Layout restore fidelity | Zoom round-trips exactly | Full session save/restore |
| Layout preset coverage | 0 presets | 4 presets (even-h, even-v, main-v, tiled) |

## Target Users

### Power Developer
- **Role:** Software engineer using PaneFlow as daily terminal multiplexer
- **Behaviors:** Runs 4-8 terminal panes simultaneously (editor, server, logs, tests). Resizes panes frequently. Expects keyboard-driven workflow.
- **Pain points:** Drag-to-resize is inaccurate. Cannot zoom a pane temporarily. Cannot save/restore workspace layouts. Pane nesting gets confusing after many splits.
- **Current workaround:** Uses tmux inside PaneFlow for zoom/presets, defeating the purpose of a native GPU-accelerated multiplexer.
- **Success looks like:** Can split, resize, zoom, and apply presets entirely within PaneFlow with the same efficiency as tmux.

### Casual Developer
- **Role:** Developer who uses 2-3 panes occasionally
- **Behaviors:** Splits once or twice, rarely resizes. Expects "it just works."
- **Pain points:** Pane resize feels broken (over/undershooting). Closing a pane sometimes leaves weird empty space.
- **Current workaround:** Closes all panes and re-splits from scratch.
- **Success looks like:** Split, resize, and close work correctly without surprises.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **tmux:** N-ary layout tree, integer cell arithmetic, 7 named presets, zoom via tree swap, layout serialization as compact strings. The gold standard for layout correctness.
- **Zellij:** N-ary tree with KDL declarative layouts, swap-layouts triggered by pane count, stacked panes. PR #4021 confirms float-ratio systems cause irreversible rounding corruption.
- **WezTerm:** Binary tree with float ratios — same architecture as PaneFlow's current system. No presets. Acknowledged as a limitation.
- **Zed editor:** Binary tree flexbox via GPUI/Taffy — identical to PaneFlow. No zoom, no presets.
- **Market gap:** No GPU-accelerated terminal multiplexer offers tmux-level layout sophistication. Ghostty has no multiplexer. WezTerm's is basic. Zellij is not GPU-accelerated.

### Best Practices Applied
- N-ary tree is the consensus direction across tmux, Zellij, and i3wm
- Single layout engine for all geometry (Zellij PR #4021: split-brain across code paths causes "state borked" errors)
- Constraint checking before mutation (tmux's `layout_resize_check` pattern)
- Zoom as full layout swap with save/restore (proven in tmux for 30+ years)

*Full research sources: 20+ web sources including Zellij DeepWiki, tmux source, WezTerm discussions, Taffy docs, Zellij PR #4021.*

## Assumptions & Constraints

### Assumptions (to validate)
- GPUI's `flex_basis(relative(r))` distributes correctly across N > 2 children with ratios summing to 1.0 (to validate in US-001)
- The existing `LayoutNode` schema in `paneflow-config/src/schema.rs` can be extended for the N-ary model without breaking config backward compatibility
- 80px minimum pane size is sufficient (tmux uses 1 character cell; PaneFlow's GUI needs more)

### Hard Constraints
- Must use GPUI's Taffy-backed flexbox for rendering — no custom coordinate/pixel computation for layout
- Must compile with the local Zed GPUI path dependency at `/home/arthur/dev/zed`
- Linux-only (Wayland + X11) — no macOS/Windows considerations
- Maximum 32 panes per workspace, maximum 20 workspaces (existing limits)
- All UI state on main thread via GPUI Entity/Context model — no Arc/Mutex for layout state

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --workspace -- -D warnings` - No lint warnings
- `cargo test --workspace` - All tests pass
- `cargo fmt --check` - Code is formatted

For UI stories:
- Run `cargo run`, create 4+ panes, verify split/resize/close visually
- Verify no pane goes below 80px minimum during resize

## Epics & User Stories

### EP-001: N-ary Tree Core

Replace the binary `SplitNode` with an N-ary `LayoutTree` that supports containers with 2+ children, automatic sibling insertion for same-direction splits, and single-child collapse on close.

**Definition of Done:** All existing split/close/focus operations work identically to v2, but using the N-ary tree internally. No user-visible regression.

#### US-001: Validate GPUI N-child flexbox
**Description:** As a developer, I want to validate that GPUI's `flex_basis(relative(r))` works correctly with N > 2 children so that the N-ary tree can render without a custom layout engine.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A test div with 3 children using `flex_basis(relative(0.33))`, `flex_basis(relative(0.33))`, `flex_basis(relative(0.34))` renders correctly in a flex_row container
- [ ] A test div with 5 children renders with equal proportions
- [ ] Given ratios that don't sum to exactly 1.0 (e.g., 0.33+0.33+0.33=0.99), the layout does not leave a visible gap or overflow
- [ ] If validation fails, document the workaround (e.g., last child uses flex_grow instead of flex_basis)

#### US-002: Define N-ary LayoutTree data structure
**Description:** As a developer, I want to replace the binary `SplitNode` enum with an N-ary `LayoutTree` enum so that containers can hold 2+ children without artificial nesting.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] New `LayoutTree` enum with `Leaf(Entity<Pane>)` and `Container { direction, children: Vec<LayoutChild> }` variants
- [ ] `LayoutChild` struct holds `node: LayoutTree`, `ratio: f32`, and `computed_size: Cell<f32>` for post-layout bounds
- [ ] `SplitDirection` enum preserved (reused by `pane.rs:61`)
- [ ] All 10 call sites in `main.rs` compile with the new type (may use stub implementations)
- [ ] `workspace.rs` type updated from `Option<SplitNode>` to `Option<LayoutTree>`
- [ ] Given a container with only 1 child remaining after a close, the container collapses to promote the child (no phantom containers)

#### US-003: Render N-ary tree via GPUI flexbox
**Description:** As a user, I want the N-ary layout tree to render correctly via GPUI's flexbox so that panes display at their specified ratios.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] `LayoutTree::render()` produces nested `div().flex()` elements with `flex_basis(relative(ratio))` per child
- [ ] Dividers (4px) rendered between each pair of adjacent children with `flex_shrink_0()`
- [ ] Horizontal containers use `flex_col()`, Vertical containers use `flex_row()` (matching existing convention)
- [ ] Given a 3-child container with ratios [0.33, 0.33, 0.34], all three panes are visible with roughly equal size
- [ ] Given a deeply nested tree (4 levels), rendering completes without stack overflow or visual glitch

#### US-004: Split operations for N-ary tree
**Description:** As a user, I want to split panes so that same-direction splits add siblings to the existing container rather than nesting a new binary split.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] `split_at_focused(direction, new_pane)` adds a sibling when the parent container matches the split direction
- [ ] `split_at_focused(direction, new_pane)` creates a new 2-child container when the parent direction differs
- [ ] `split_at_pane(target, direction, new_pane)` works identically for pane-button-triggered splits
- [ ] `split_first_leaf(direction, new_pane)` works for IPC-triggered splits (no Window context)
- [ ] Given 4 consecutive vertical splits from a single pane, the tree is 1 level deep (flat `Container{A,B,C,D,E}`), not 4 levels deep
- [ ] Given a split that would exceed 32 panes, the split is rejected (existing guard preserved)

#### US-005: Close operations with sibling collapse
**Description:** As a user, I want closing a pane to redistribute space to siblings and collapse single-child containers so that the tree stays minimal.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] `close_focused()` removes the focused pane and redistributes its ratio equally among remaining siblings
- [ ] `remove_pane(target)` removes a specific pane by entity identity
- [ ] Given a container reduced to 1 child, the container node is removed and the child promoted to its parent's position
- [ ] Given the last pane in a workspace is closed, `workspace.root` becomes `None` and the workspace is destroyed
- [ ] Focus transfers to the previous sibling (or next if no previous) after close

#### US-006: Focus navigation in N-ary tree
**Description:** As a user, I want `Alt+Arrow` focus navigation to work correctly in N-ary containers so that I can move between panes directionally.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] `focus_in_direction(Left/Right/Up/Down)` traverses N-ary containers correctly (not just binary first/second)
- [ ] Given a 4-child horizontal container [A,B,C,D] with focus on B, `Right` focuses C and `Left` focuses A
- [ ] Given focus on the last child, the direction propagates up to the parent container (cross-container navigation)
- [ ] `focus_first()` and `focus_last()` work on N-ary containers (focus leftmost/rightmost leaf)
- [ ] Given a single-pane workspace, all focus directions are no-ops (no crash or unexpected behavior)

---

### EP-002: Constraint-Based Resize

Replace the hardcoded 800px container estimate with actual element bounds and add recursive constraint propagation to prevent minimum-size violations.

**Definition of Done:** Drag-to-resize is pixel-accurate on any window size. No pane can be resized below 80px. Constraint propagation prevents invalid layouts.

#### US-007: Capture actual container bounds
**Description:** As a developer, I want the layout container to expose its actual pixel bounds to drag handlers so that mouse delta → ratio conversion is accurate.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Each container div stores its computed pixel size (from GPUI layout) in a `Cell<f32>` accessible by drag handlers
- [ ] The 800px hardcode at `split.rs:139` is removed entirely
- [ ] Given a 1200px wide window with a vertical split, dragging the divider 120px to the right changes the ratio by exactly 0.1 (120/1200)
- [ ] Given a 600px wide window, the same 120px drag changes the ratio by 0.2 (120/600)
- [ ] Given a window resize during drag, the drag is cancelled (drag_start reset to None)

#### US-008: Recursive constraint checking
**Description:** As a developer, I want a `resize_check(node, direction)` function that computes how much space a subtree can yield so that resize operations never violate minimum pane sizes.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] `resize_check(node, direction) -> f32` returns the maximum pixels that can be removed from the subtree
- [ ] For a leaf node: returns `current_size - MIN_PANE_SIZE` (80px)
- [ ] For a same-direction container: returns sum of children's `resize_check` values
- [ ] For a cross-direction container: returns minimum of children's `resize_check` values
- [ ] Given a drag that would shrink a pane below 80px, the drag is clamped to the available space (no visual glitch, no constraint violation)

#### US-009: Drag-to-resize with constraint clamping
**Description:** As a user, I want drag-to-resize to feel smooth and never allow panes to collapse below minimum size so that the layout remains usable.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] Dragging a divider between two siblings adjusts their ratios proportionally using the actual container size
- [ ] In an N-ary container, dragging a divider between children `i` and `i+1` only affects those two children's ratios (other siblings unchanged)
- [ ] Given a drag that would violate the minimum size of a nested subtree, the drag stops at the constraint boundary
- [ ] Cursor changes to `row_resize` (horizontal divider) or `col_resize` (vertical divider) on hover
- [ ] `mouse_up` and `mouse_up_out` both end the drag cleanly

---

### EP-003: Zoom

Implement tmux-style zoom: the focused pane temporarily takes the full workspace area, and un-zoom restores the exact previous layout.

**Definition of Done:** `Ctrl+Shift+Z` toggles zoom. Layout round-trips perfectly.

#### US-010: Zoom toggle action
**Description:** As a user, I want to press `Ctrl+Shift+Z` to zoom the focused pane to fill the entire workspace so that I can temporarily focus on one task.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] New `ToggleZoom` action registered in `actions!` macro and bound to `Ctrl+Shift+Z`
- [ ] Zoom saves the current `LayoutTree` and replaces it with a single `Leaf` containing the focused pane
- [ ] The zoomed pane fills the entire workspace area (minus sidebar/titlebar)
- [ ] Given a workspace with 1 pane, zoom is a no-op (no crash, no visual change)
- [ ] Given a workspace with no focused pane, zoom is a no-op

#### US-011: Un-zoom with exact layout restore
**Description:** As a user, I want pressing `Ctrl+Shift+Z` again to restore the exact previous layout so that my pane arrangement is preserved.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] Un-zoom restores `saved_layout_root` and all pane positions exactly as before zoom
- [ ] Pane focus returns to the previously-zoomed pane
- [ ] Given a window resize while zoomed, un-zoom restores the layout proportions at the new window size (ratios preserved, absolute sizes recomputed)
- [ ] Given `close_pane` while zoomed, the zoom exits and the pane is closed from the saved layout (no orphan pane)

#### US-012: Zoom visual indicator
**Description:** As a user, I want a visual indicator when a pane is zoomed so that I know the layout is temporarily hidden.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] A `[Z]` badge or highlight appears in the tab bar or pane header when zoomed
- [ ] The indicator disappears immediately on un-zoom
- [ ] Given a theme change while zoomed, the indicator respects the new theme colors

---

### EP-004: Layout Presets

Implement tmux-style named layout presets that destroy and rebuild the tree to arrange panes in predefined patterns.

**Definition of Done:** 4 layout presets available via keybinding or IPC. Presets work with any number of panes.

#### US-013: Even-horizontal preset
**Description:** As a user, I want to apply an "even-horizontal" preset so that all panes are arranged side by side with equal widths.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] New `LayoutEvenHorizontal` action bound to a keybinding (e.g., `Ctrl+Shift+1`)
- [ ] Destroys the current tree and creates a single `Container { direction: Vertical, children: [all panes with equal ratios] }`
- [ ] Given 4 panes and a 1200px window, each pane gets ~297px (1200 - 3*4px dividers) / 4
- [ ] Given 1 pane, the preset is a no-op
- [ ] Given the preset is applied while zoomed, zoom exits first, then the preset applies

#### US-014: Even-vertical preset
**Description:** As a user, I want an "even-vertical" preset that stacks all panes top-to-bottom with equal heights.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] New `LayoutEvenVertical` action
- [ ] Creates a single `Container { direction: Horizontal, children: [all panes with equal ratios] }`
- [ ] Given 3 panes and a 900px tall workspace, each pane gets ~296px
- [ ] Given the current layout is already even-vertical, re-applying is idempotent (no flicker, no reorder)

#### US-015: Main-vertical preset
**Description:** As a user, I want a "main-vertical" preset that gives one pane 60% width on the left and stacks the rest on the right.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] New `LayoutMainVertical` action
- [ ] Creates a `Container { direction: Vertical, children: [main_pane (0.6), sub_container (0.4)] }` where sub_container is `Container { direction: Horizontal, children: [remaining panes, equal ratios] }`
- [ ] The main pane is the currently focused pane (or the first pane if no focus)
- [ ] Given 2 panes, creates a simple 60/40 vertical split
- [ ] Given 1 pane, the preset is a no-op

#### US-016: Tiled preset
**Description:** As a user, I want a "tiled" preset that arranges panes in a grid (rows x columns) filling the workspace evenly.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] New `LayoutTiled` action
- [ ] Computes optimal rows/columns: increment rows and columns alternately until `rows * cols >= N` (tmux algorithm)
- [ ] Creates a `Container { direction: Horizontal, children: [row containers with equal ratios] }` where each row is `Container { direction: Vertical, children: [panes with equal ratios] }`
- [ ] Given 4 panes, produces a 2x2 grid
- [ ] Given 5 panes, produces a 2x3 grid with one empty cell position (last row has 2 panes instead of 3)
- [ ] Given 1 pane, the preset is a no-op

---

### EP-005: Layout Serialization

Serialize and deserialize the layout tree to enable save/restore of workspace layouts.

**Definition of Done:** Layout can be serialized to JSON and restored from JSON via IPC. Config schema updated.

#### US-017: Serialize LayoutTree to JSON
**Description:** As a developer, I want to serialize the current `LayoutTree` to a JSON structure so that layouts can be saved to disk or transmitted via IPC.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] `LayoutTree::serialize() -> serde_json::Value` produces a JSON representation matching the `LayoutNode` schema in `paneflow-config/src/schema.rs`
- [ ] The schema is extended to support N children (remove the "exactly 2 children" constraint at `schema.rs:68`)
- [ ] Serialized output includes direction, ratios, and pane metadata (shell command, cwd) for each leaf
- [ ] Given a zoomed workspace, serialization captures the saved (un-zoomed) layout, not the zoom state
- [ ] Given round-trip serialize → deserialize on a 6-pane layout, the tree structure is identical

#### US-018: Deserialize and apply layout from JSON
**Description:** As a user, I want to restore a previously saved layout from JSON so that I can resume my workspace arrangement.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] `LayoutTree::deserialize(json, panes) -> LayoutTree` rebuilds the tree from JSON, assigning existing panes to leaves in order
- [ ] If the saved layout has more leaves than current panes, extra leaves spawn new panes
- [ ] If the saved layout has fewer leaves than current panes, extra panes are closed
- [ ] IPC method `workspace.restore_layout(json)` applies a layout to the current workspace
- [ ] Given invalid JSON, the operation returns an error via IPC without crashing or corrupting the current layout

## Functional Requirements

- FR-01: The system must support containers with 2 or more child nodes (N-ary tree).
- FR-02: When a pane is split in the same direction as its parent container, the system must add a sibling to the existing container rather than creating a nested binary split.
- FR-03: When a container is reduced to a single child, the system must collapse the container and promote the child.
- FR-04: The system must prevent any pane from being resized below 80px in either dimension.
- FR-05: The system must convert mouse pixel deltas to ratio changes using the actual container pixel size, not a hardcoded estimate.
- FR-06: Zoom must preserve the complete layout tree and restore it exactly on un-zoom.
- FR-07: Layout presets must work with any number of panes (1 to 32).
- FR-08: Layout serialization must round-trip without loss of structure or ratios.

## Non-Functional Requirements

- **Performance:** Layout tree operations (split, close, resize-check) must complete in < 1ms for 32 panes. Render method must produce the GPUI element tree in < 2ms.
- **Memory:** Layout tree overhead must be < 10KB for 32 panes (excludes terminal buffers).
- **Responsiveness:** Drag-to-resize must update at 60fps with no visible lag or stutter.
- **Correctness:** Zero constraint violations (pane < 80px) across all automated tests and manual verification.
- **Compatibility:** All existing keybindings (`Ctrl+Shift+D/E/W`, `Alt+Arrow`, `Ctrl+Shift+N/Q`, etc.) must continue to work identically.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Single pane workspace | User attempts zoom/preset/resize | No-op, no crash | — |
| 2 | Maximum panes reached | 33rd split attempted | Split rejected | "Maximum 32 panes reached" (existing behavior) |
| 3 | Window shrink below minimum | Terminal window resized very small | Panes clamp to 80px minimum; some may be hidden if space is insufficient | — |
| 4 | Close during zoom | User closes the zoomed pane | Exit zoom, remove pane from saved layout, focus next pane | — |
| 5 | Split during zoom | User splits while zoomed | Exit zoom first, then apply split to the saved layout | — |
| 6 | Drag overshoot | Mouse dragged far beyond container edge | Ratio clamped by constraint check; drag stops at boundary | Cursor remains resize cursor |
| 7 | Window resize during drag | Terminal window resized while dragging a divider | Cancel drag (reset drag_start to None), recompute layout | — |
| 8 | Deserialize with mismatched pane count | Saved layout has 5 leaves but workspace has 3 panes | Spawn 2 new panes to fill the layout | — |
| 9 | Invalid layout JSON via IPC | Malformed JSON in `workspace.restore_layout` | Return JSON-RPC error, keep current layout intact | Error in IPC response |
| 10 | Concurrent IPC split + keyboard split | Two split requests arrive simultaneously | Second split sees the tree after the first; both succeed if under 32 panes | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI flexbox doesn't distribute N children correctly | Low | High | US-001 validates this first; fallback: last child uses `flex_grow` instead of `flex_basis` |
| 2 | Float ratio rounding corruption over many resize operations | Medium | Medium | Constraint checking prevents sub-minimum panes; ratios re-normalized after each mutation |
| 3 | Focus navigation regression in N-ary tree | Medium | Medium | Port existing `FocusDirection`/`FocusNav` enums; test with 4+ child containers |
| 4 | Layout serialization breaks on schema evolution | Low | Low | Version field in JSON schema; deserializer handles missing fields with defaults |
| 5 | Performance regression with 32 panes | Low | Medium | Layout operations are O(n) tree walks; 32 nodes is trivial. Benchmark in US-003 |
| 6 | Zoom + close interaction state corruption | Medium | High | US-011 explicitly tests close-during-zoom; zoom exits before any structural mutation |

## Non-Goals

Explicit boundaries — what this version does NOT include:

- **Floating panes** — overlay panes positioned above the tiled layout (Zellij feature). Deferred to v4.
- **Stacked panes** — accordion/tab-group within a container node (Zellij v0.35.1). Deferred to v4.
- **Declarative layout files** — KDL/TOML/YAML layout definition files for dotfile sharing. Deferred to v4.
- **Swap layouts** — automatic preset switching based on pane count (Zellij feature). Deferred to v4.
- **Integer cell arithmetic** — storing sizes as integer character cells instead of float pixels. The constraint system mitigates rounding issues; a full integer rewrite is a v4 consideration.
- **Undo/redo for layout operations** — split and close are not reversible (same as tmux).
- **Cross-workspace pane movement** — moving a pane from one workspace to another (tmux `join-pane`).

## Files NOT to Modify

- `src-app/src/terminal.rs` — PTY management, TerminalView, TerminalState — no layout dependency
- `src-app/src/terminal_element.rs` — GPUI Element impl — receives bounds from flexbox automatically
- `src-app/src/ipc.rs` — JSON-RPC infrastructure — only `main.rs` handler code changes
- `src-app/src/title_bar.rs` — CSD window controls — no layout dependency
- `src-app/src/theme.rs` — Theme resolution — no layout dependency
- `src-app/src/keys.rs` — Key-to-escape-string mapping — no layout dependency
- `crates/paneflow-config/src/loader.rs` — Config file loader — no layout code
- `crates/paneflow-config/src/watcher.rs` — File watcher — no layout code

## Technical Considerations

- **Tree type:** N-ary `Vec<LayoutChild>` — recommended based on tmux, Zellij, and i3wm consensus. Engineering to confirm GPUI compatibility in US-001.
- **Ratio storage:** `f32` ratios per child, re-normalized after each mutation to sum to 1.0. Alternative: integer pixel counts. Trade-off: floats are simpler with GPUI's `relative()` API but risk rounding drift. Constraint checking mitigates.
- **Container bounds capture:** Use `Rc<Cell<f32>>` populated during `on_mouse_move` from the container div's implicit size. Alternative: custom GPUI `Element` with `prepaint` phase bounds. Trade-off: `Element` is more accurate but more code. Recommend starting with the simpler approach.
- **Zoom state:** `saved_layout: Option<LayoutTree>` field on `Workspace`. Alternative: `saved_layout_root` on a dedicated `LayoutEngine` struct. Trade-off: workspace field is simpler; engine struct is cleaner if more layout features are added.
- **Preset implementation:** Destroy-and-rebuild (tmux approach). Alternative: in-place tree restructuring. Trade-off: destroy-and-rebuild is simpler, O(n) pane reassignment; in-place is more complex but preserves pane order. Recommend destroy-and-rebuild.
- **Serialization format:** JSON via `serde`, extending `paneflow-config/src/schema.rs`. Alternative: compact string format (tmux-style `{110x50,0,0,1}`). Trade-off: JSON is human-readable and tool-friendly; compact strings are smaller. JSON recommended for v3.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Drag-to-resize accuracy | ~33% error on non-800px windows | < 1% error | Month-1 | Manual test: drag 100px, verify ratio change matches expected |
| Constraint violations | Possible (no checking) | 0 violations | Month-1 | Automated: resize all dividers to extremes, verify no pane < 80px |
| Layout tree depth for N same-dir splits | N-1 levels (binary nesting) | 1 level (flat container) | Month-1 | Automated: `assert_eq!(tree.depth(), 1)` after N same-dir splits |
| Zoom round-trip fidelity | N/A (no zoom) | 100% identical layout | Month-1 | Manual: zoom → resize window → un-zoom → verify |
| Layout presets available | 0 | 4 (even-h, even-v, main-v, tiled) | Month-6 | Feature exists and is bound to keybindings |
| Layout save/restore | N/A (no serialization) | Round-trip without loss | Month-6 | Automated: serialize → deserialize → compare tree structure |

## Open Questions

- Should layout preset keybindings use `Ctrl+Shift+1/2/3/4` (conflicts with workspace select `Ctrl+1-9`)? Alternative: `Ctrl+Alt+1/2/3/4` or a prefix key. Engineering to decide — blocked by US-013.
- Should the N-ary tree support mixed-direction children in a single container (i3wm-style), or enforce uniform direction per container (tmux-style)? Recommendation: uniform direction (simpler). To confirm during US-002.
- Should zoom preserve scroll position and cursor state for all non-zoomed panes? Currently panes continue to receive PTY output while not rendered. To validate during US-011.

[/PRD]
