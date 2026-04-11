[PRD]
# PRD: Servo WebView Embedding Spike

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-06 | Claude + Arthur | Initial spike PRD from feasibility study |
| 1.1 | 2026-04-06 | Claude + Arthur | Enriched with deep Servo codebase exploration (5-agent swarm) |

## Problem Statement

PaneFlow aims to offer an embedded web browser as a pane type (splittable alongside terminals), matching cmux's browser feature. A comprehensive feasibility study evaluated all options:

1. **cmux uses macOS-native WKWebView** — zero-effort embedding because Apple provides the webview as an NSView. Not portable.
2. **wry/webkit2gtk** — requires GTK event loop, incompatible with GPUI's Vulkan/Wayland rendering pipeline. `build_gtk()` is the only Linux API — no `RawWindowHandle` path.
3. **CEF Rust bindings** — all 5 community projects are abandoned/stale. No production-quality option.
4. **GPUI has zero foreign surface support** — no `NativeView`, no `wl_subsurface`, no XEmbed. `PrimitiveBatch::Surfaces` is an explicit no-op on Linux (`wgpu_renderer.rs:1283`).
5. **Servo** (Rust-native browser engine) is the only remaining candidate. Critical finding: **WebRender uses OpenGL/surfman, NOT wgpu** — the wgpu version conflict (Servo v26 vs GPUI v29) only affects the `webgpu` feature (WebGPU JS API) and can be disabled entirely.

**Why a spike:** Servo's embedding API is pre-production (v0.0.4, Jan 2026). The Slint framework has a working Servo integration (published early 2025), proving the concept is viable. However, GPUI's architecture differs significantly from Slint's — this spike validates whether Servo can work specifically with GPUI before committing to a full implementation.

**Why now:** PaneFlow's core terminal multiplexer is feature-complete (v2 PRDs delivered). Browser embedding is the next differentiating feature. The spike is low-priority but should be ready to execute when bandwidth allows.

## Overview

A time-boxed (2-week) technical spike to validate embedding Servo's browser engine inside a GPUI Element in PaneFlow. The spike progresses through 4 gates:

1. **Build gate** — Servo compiles as a dependency alongside GPUI (wgpu version compatibility)
2. **Render gate** — Servo renders HTML into a pixel buffer via `OffscreenRenderingContext` or `SoftwareRenderingContext`
3. **Display gate** — The pixel buffer is uploaded as a GPUI texture and displayed in an Element
4. **Input gate** — Keyboard and mouse events are forwarded from GPUI to Servo

Each gate has a clear pass/fail criterion. If any gate fails with no workaround, the spike aborts and documents the blocker for future re-evaluation.

Key technical decisions:
- **Software rendering first**: Use `SoftwareRenderingContext` (CPU pixel buffer via OSMesa/surfman) as the primary path — confirmed RGBA format via `glReadPixels(gl::RGBA, gl::UNSIGNED_BYTE)` at `rendering_context.rs:679-723`. Image is returned as `image::RgbaImage` (top-row first, vertically flipped from GL). GPUI's `ImageData` expects RGBA — format matches without byte-swapping.
- **In-process embedding**: Run Servo in the same process as PaneFlow. `Servo` and `WebView` are `Rc`-based (not `Arc`) — they must stay on the GPUI main thread.
- **Disable `webgpu` feature**: WebRender uses OpenGL/surfman, NOT wgpu. The `webgpu` feature (wgpu v26) is only for the WebGPU JS API — disabling it eliminates the wgpu version conflict entirely.
- **Reference blueprint**: `servo/components/servo/examples/winit_minimal.rs` (194 lines) — a complete working embedder. Also reference Slint + Servo integration ([blog](https://slint.dev/blog/using-servo-with-slint)).
- **No UI chrome**: No URL bar, no navigation buttons, no tabs. The spike only validates the rendering pipeline.

## Goals

| Goal | Spike Target | Production Target (if spike passes) |
|------|-------------|-------------------------------------|
| Servo renders HTML in GPUI | Static HTML page visible in a GPUI Element | Full interactive browsing with navigation |
| Input forwarding | Basic click + keypress reach Servo | Complete keyboard, mouse, scroll, touch |
| Performance | < 100ms for static page render, 15+ fps for updates | 60fps, < 16ms input-to-pixel latency |
| Cross-platform architecture | Linux-first PoC, architecture portable | macOS + Windows + Linux production |

## Target Users

### PaneFlow Developer (Arthur)
- **Role:** Solo developer evaluating technical feasibility
- **Behaviors:** Will build the spike, evaluate results, make go/no-go decision
- **Pain points:** Needs to know if Servo embedding is viable before investing months of work
- **Success looks like:** Clear go/no-go with documented evidence (benchmarks, screenshots, blocker list)

## Non-Functional Requirements

| NFR | Requirement | Measurement |
|-----|------------|-------------|
| Build time | Servo dependency adds < 15 min to clean build on 8-core machine | `time cargo build` delta |
| Binary size | Servo adds < 300 MB to debug binary | `ls -la` before/after |
| Memory | Servo instance uses < 500 MB RSS for a static HTML page | `ps aux` RSS column |
| Compilation | Builds on Fedora 43 with stable Rust toolchain | `cargo build` exit code 0 |

## Scope Boundaries

### In Scope
- Servo as a Cargo git dependency alongside GPUI path dependencies
- `SoftwareRenderingContext` or `OffscreenRenderingContext` for offscreen rendering
- Pixel buffer upload to GPUI as a `PolychromeSprite` or custom Element paint
- Basic keyboard/mouse event forwarding via `WebView::notify_input_event()`
- A hardcoded test HTML page (no URL bar input)
- Performance benchmarks (fps, latency, memory)
- Go/no-go decision document

### Out of Scope
- URL bar, navigation (back/forward/reload), tabs, DevTools
- GPU texture sharing via Vulkan FD export (stretch goal only)
- Cross-platform testing (macOS, Windows) — architecture review only
- Production `BrowserView` Entity integration into SplitNode
- Session persistence, bookmarks, history
- Any changes to GPUI itself (no forking Zed's GPUI)

## Research Findings (enriched via 5-agent Servo codebase exploration)

### Servo Rendering Stack — Critical Discovery

**WebRender uses OpenGL/surfman, NOT wgpu.** Servo's wgpu usage is exclusively for the WebGPU JS spec implementation (`components/webgpu/`), not for page rendering.

```
Servo page rendering:  WebRender 0.68 → gleam (GL bindings) → surfman → OpenGL/GLES/EGL
Servo WebGPU JS API:   wgpu-core 26.0.1 → Vulkan/Metal (SEPARATE GPU context, optional)
GPUI rendering:        wgpu 29.0.0 (Zed fork) → Vulkan/Metal

→ Disabling `webgpu` feature eliminates ALL wgpu from Servo. No version conflict.
```

Evidence: `components/paint/painter.rs:154` uses `rendering_context.gleam_gl_api()`, `components/paint/painter.rs:261` logs `"Running on {gl_renderer} with OpenGL version {gl_version}"`, `Cargo.toml:183` declares `surfman = "0.11.0"`.

### Servo Embedding API (verified from source)

| Type | File | Purpose |
|------|------|---------|
| `Servo` | `components/servo/servo.rs:803` | Engine instance. `Rc`-based (main thread only). |
| `ServoBuilder` | `components/servo/servo.rs:1328` | `ServoBuilder::default().event_loop_waker(waker).build()` |
| `WebView` | `components/servo/webview.rs:79` | Handle to one browsing context. Clone = new handle, not new view. |
| `WebViewBuilder` | `components/servo/webview.rs:892` | `WebViewBuilder::new(&servo, rendering_context).url(url).delegate(state).build()` |
| `WebViewDelegate` | `components/servo/webview_delegate.rs:857` | Trait — embedder implements callbacks (e.g., `notify_new_frame_ready`) |
| `ServoDelegate` | `components/servo/servo.rs` | Engine-level delegate (set via `servo.set_delegate()` at line 959) |
| `EventLoopWaker` | `components/shared/embedder/lib.rs:226` | Trait: `wake()` + `clone_box()`. Servo calls `wake()` from background threads to poke the embedder event loop. |

### SoftwareRenderingContext — Pixel Buffer Path

**File:** `components/shared/paint/rendering_context.rs:304-399`

```rust
pub struct SoftwareRenderingContext {
    size: Cell<PhysicalSize<u32>>,
    surfman_rendering_info: SurfmanRenderingContext,
    swap_chain: SwapChain<Device>,
}
```

- Creates a **software adapter** (OSMesa / CPU rasterizer) via `connection.create_software_adapter()` (line 313)
- Surface type: `SurfaceType::Generic { size }` — offscreen, no display connection required
- **Pixel read:** `read_to_image(rect) → Option<RgbaImage>` — calls `glReadPixels(gl::RGBA, gl::UNSIGNED_BYTE)` at lines 679-723
- **Format:** RGBA 8-bit, top-row first (GL bottom-up flipped at lines 709-716)
- **Timing:** Read from back buffer after `webview.paint()`, before `present()` (docstring lines 39-47)
- **Resize:** `swap_chain.resize()` at line 362

### OffscreenRenderingContext — GPU Offscreen Path

**File:** `components/shared/paint/rendering_context.rs:726-888`

- Requires a parent `WindowRenderingContext` (real window with GL context)
- Uses a custom `Framebuffer` (FBO) — RGBA texture + depth renderbuffer
- `present()` is a **no-op** (line 852) — the embedder blits via `render_to_parent_callback()`
- `read_to_image()` works the same way (same `glReadPixels` path)
- Created via `WindowRenderingContext::offscreen_context(size)`

### Frame Notification Chain (verified)

```
WebRender backend thread (async)
  → RenderNotifier::new_frame_ready()           [render_notifier.rs:34]
  → PaintProxy::send(NewWebRenderFrameReady)
  → EventLoopWaker::wake()                      [wakes embedder event loop]

Embedder calls servo.spin_event_loop()          [servo.rs:184]
  → paint.perform_updates()
  → WebViewDelegate::notify_new_frame_ready(webview)  [webview_delegate.rs:891]

Embedder implementation:
  → webview.paint()                              [webview.rs:645]
  → rendering_context.read_to_image(rect)        [rendering_context.rs:344]
  → RgbaImage (RGBA u8 pixels)
```

### Input Event API (verified from source)

**Entry point:** `webview.notify_input_event(InputEvent::*)` — `components/servo/webview.rs`

**`InputEvent` enum** — `components/shared/embedder/input_events.rs:53-64`:
```rust
pub enum InputEvent {
    EditingAction(EditingActionEvent),
    Gamepad(GamepadEvent),        // cfg(feature = "gamepad")
    Ime(ImeEvent),
    Keyboard(KeyboardEvent),
    MouseButton(MouseButtonEvent),
    MouseLeftViewport(MouseLeftViewportEvent),
    MouseMove(MouseMoveEvent),
    Touch(TouchEvent),
    Wheel(WheelEvent),
}
```

**Servoshell keyboard mapping reference:** `ports/servoshell/desktop/keyutils.rs` converts winit `KeyEvent` → Servo `KeyboardEvent`. GPUI also uses winit-style key events — translation should be straightforward.

**Mouse coordinates:** Must be in device pixels, webview-relative (subtract any toolbar offset — zero for spike).

### Build Configuration (verified)

**Minimum viable dependency:**
```toml
servo = { path = "/home/arthur/dev/servo/components/servo", default-features = false, features = [
    "baked-in-resources",
    "js_jit",
] }
```

**Disabling `webgpu` eliminates wgpu v26 entirely.** Other safe-to-disable features: `bluetooth`, `webxr`, `media-gstreamer`, `gamepad`.

**System dependencies (Linux):** `clang`, `llvm-dev`, `cmake` (for SpiderMonkey), `libfreetype6-dev`, `libharfbuzz-dev`, `libx11-dev`, `libxcb-*`, `libxkbcommon*`. No `libvulkan1` needed without `webgpu`.

**Rust toolchain:** Pinned at `1.92.0` in `rust-toolchain.toml`. SpiderMonkey (`mozjs 0.15.7`) is the heaviest build dep (~15-20 min first compile).

### Minimal Embedder Blueprint

**File:** `components/servo/examples/winit_minimal.rs` (194 lines) — complete working example.

```rust
// 1. Crypto setup
rustls::crypto::aws_lc_rs::default_provider().install_default()?;

// 2. Create rendering context (for spike: SoftwareRenderingContext instead)
let rendering_context = Rc::new(SoftwareRenderingContext::new(size));

// 3. Build Servo
let servo = ServoBuilder::default()
    .event_loop_waker(Box::new(waker))
    .build();
servo.setup_logging();

// 4. Create WebView
let webview = WebViewBuilder::new(&servo, rendering_context.clone())
    .url(Url::parse("data:text/html,<h1>Hello</h1>")?)
    .hidpi_scale_factor(Scale::new(1.0))
    .delegate(app_state.clone())  // implements WebViewDelegate
    .build();

// 5. Event loop: on waker event → servo.spin_event_loop()
// 6. On notify_new_frame_ready → webview.paint() → rendering_context.read_to_image(rect)
```

### GPUI Architecture Constraints (unchanged)

| Constraint | Evidence |
|-----------|---------|
| Element trait has no native view hook | `gpui/src/element.rs:51-110` |
| Single wgpu surface per window | `gpui_linux/wayland/window.rs:92-125` |
| `canvas` element allows custom draw | `gpui/src/elements/canvas.rs:8` — only wgpu primitives |
| Surface batch is no-op on Linux | `gpui_wgpu/wgpu_renderer.rs:1283-1287` |
| Polychrome sprites support RGBA textures | `scene.rs` — `PolychromeSprite` |

**Viable path**: `SoftwareRenderingContext::read_to_image()` → `RgbaImage` → upload as GPUI polychrome sprite. Format-compatible (RGBA u8), no GPUI modifications required.

### Risk Matrix (updated after codebase exploration)

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ~~wgpu version conflict~~ | ~~High~~ **Eliminated** | ~~Showstopper~~ | Disable `webgpu` feature — WebRender uses OpenGL, not wgpu |
| SpiderMonkey build fails on Fedora 43 | Medium | Delays | Servo requires clang/llvm/cmake — verify system deps in US-001 |
| `SoftwareRenderingContext::new()` requires display connection | Low | Blocks headless | Uses `Connection::new()` without `DisplayHandle` (line 312) — should work |
| Servo `Rc`-based API conflicts with GPUI threading model | Low | Architecture | Both are main-thread — `Servo`/`WebView` stay on GPUI thread |
| SpiderMonkey conflicts with GPUI allocator | Low | Showstopper | SpiderMonkey runs on separate threads with its own heap |
| Pixel buffer copy too slow for 60fps | Medium | Degrades UX | `glReadPixels` on software adapter is CPU-bound. Acceptable for spike. |
| Binary size > 300 MB | Medium | Ergonomic | Acceptable for spike. Feature gates + stripping for production. |
| Servo HTML/CSS coverage insufficient | Medium | Scope limit | Test in US-009. |

## User Stories

### EP-001: Servo Build Integration

**Definition of done:** `cargo build` succeeds with Servo as a git dependency alongside all existing GPUI path dependencies. No compilation errors, no wgpu version conflicts.

---

#### US-001: Add Servo as a Cargo git dependency

**As a** developer, **I want** Servo to compile alongside GPUI in the PaneFlow workspace **so that** I can validate there are no fundamental build conflicts before writing integration code.

**Priority:** P0 (Must Have) | **Size:** M (3 pts) | **Dependencies:** None

**Servo codebase references:**
- Library crate: `components/servo/Cargo.toml` — `[lib] name = "servo", crate-type = ["rlib"]`
- Public API: `components/servo/lib.rs:33-95` — exports `Servo`, `ServoBuilder`, `WebView`, `WebViewBuilder`, `WebViewDelegate`, `RenderingContext`, `SoftwareRenderingContext`
- Feature flags: `components/servo/Cargo.toml:18-80` — use `default-features = false, features = ["baked-in-resources", "js_jit"]`
- Rust toolchain: `rust-toolchain.toml` — pinned at `1.92.0`

**Acceptance Criteria:**
- [ ] `Cargo.toml` adds `servo = { path = "/home/arthur/dev/servo/components/servo", default-features = false, features = ["baked-in-resources", "js_jit"] }`
- [ ] `cargo build` completes without errors on Fedora 43 with Rust 1.92.0+
- [ ] `webgpu` feature is NOT enabled — confirms no wgpu version conflict with GPUI's wgpu v29 (Zed fork)
- [ ] System deps installed: `clang`, `llvm-dev`, `cmake`, `libfreetype6-dev`, `libharfbuzz-dev`
- [ ] Build time increase is documented (baseline vs with Servo)
- [ ] Binary size increase is documented
- [ ] If build fails: document exact error, attempted workarounds, and whether the conflict is fundamental or resolvable

---

#### US-002: Validate Servo initialization lifecycle

**As a** developer, **I want** to create a `Servo` instance with `SoftwareRenderingContext` inside the PaneFlow process **so that** I can confirm the runtime initializes without panics or conflicts with GPUI's event loop.

**Priority:** P0 (Must Have) | **Size:** S (2 pts) | **Dependencies:** US-001

**Servo codebase references:**
- `ServoBuilder`: `components/servo/servo.rs:1328` — `ServoBuilder::default().event_loop_waker(waker).build()`
- `SoftwareRenderingContext::new()`: `components/shared/paint/rendering_context.rs:304-330` — creates surfman software adapter + `SurfaceType::Generic`
- `EventLoopWaker` trait: `components/shared/embedder/lib.rs:226-234` — implement `wake()` to bridge to GPUI's `cx.notify()`
- Shutdown state: `components/servo/servo.rs:156-158` — `shutdown_state: Rc<Cell<ShutdownState>>`
- Minimal example init: `components/servo/examples/winit_minimal.rs:78-81`

**Acceptance Criteria:**
- [ ] `rustls::crypto::aws_lc_rs::default_provider().install_default()` called before Servo init
- [ ] `SoftwareRenderingContext` created with a fixed size (e.g., 800x600)
- [ ] `ServoBuilder::default().event_loop_waker(Box::new(gpui_waker)).build()` returns successfully
- [ ] `servo.setup_logging()` completes without panic
- [ ] Servo's internal threads (SpiderMonkey, WebRender compositor) start without interfering with GPUI's main thread
- [ ] The `EventLoopWaker::wake()` implementation correctly wakes the GPUI event loop (e.g., via channel + `cx.notify()`)
- [ ] If initialization fails: document exact error and whether it's a threading conflict, allocator conflict, or API issue

---

### EP-002: Offscreen Rendering Pipeline

**Definition of done:** Servo renders a hardcoded HTML page to a pixel buffer, and that buffer is visible as a texture inside a GPUI Element.

---

#### US-003: Render HTML to pixel buffer via SoftwareRenderingContext

**As a** developer, **I want** Servo to render a hardcoded HTML page into an RGBA pixel buffer **so that** I can confirm offscreen rendering works and inspect the output.

**Priority:** P0 (Must Have) | **Size:** M (3 pts) | **Dependencies:** US-002

**Servo codebase references:**
- `WebViewBuilder`: `components/servo/webview.rs:892` — `WebViewBuilder::new(&servo, rendering_context).url(url).delegate(state).build()`
- `WebViewDelegate::notify_new_frame_ready`: `components/servo/webview_delegate.rs:891` — called when a frame is ready
- `WebView::paint()`: `components/servo/webview.rs:645` — triggers WebRender to render into the FBO
- `read_to_image()`: `components/shared/paint/rendering_context.rs:344-346` → calls `glReadPixels(gl::RGBA, gl::UNSIGNED_BYTE)` at lines 679-723
- Pixel format: RGBA 8-bit, `image::RgbaImage`, top-row first (GL flip at lines 709-716)
- Render flow: `webview.paint()` → `Painter::render()` (painter.rs:403-439) → `webrender_renderer.render(size, 0)` → FBO
- Headless example: `ports/servoshell/desktop/headless_window.rs`

**Acceptance Criteria:**
- [ ] `WebViewBuilder::new(&servo, rendering_context.clone()).url(url).delegate(state).build()` creates a WebView
- [ ] Servo loads `data:text/html,<h1>Hello from Servo</h1><p style="color:red">PaneFlow spike</p>`
- [ ] `servo.spin_event_loop()` is called in a loop until `notify_new_frame_ready` fires
- [ ] `webview.paint()` is called, then `rendering_context.read_to_image(rect)` returns `Some(RgbaImage)`
- [ ] The `RgbaImage` contains non-zero pixel data (not all black or all transparent)
- [ ] The buffer is saved to `/tmp/paneflow-servo-spike.png` via `image::RgbaImage::save()` for visual inspection
- [ ] If rendering fails or buffer is empty: document the exact API calls attempted and Servo's error output

---

#### US-004: Display Servo pixel buffer in a GPUI Element

**As a** developer, **I want** the Servo-rendered pixel buffer displayed inside a GPUI Element **so that** I can see web content composited within PaneFlow's window.

**Priority:** P0 (Must Have) | **Size:** L (5 pts) | **Dependencies:** US-003

**Acceptance Criteria:**
- [ ] A new `BrowserElement` (implementing GPUI's `Element` trait) uploads the pixel buffer as a texture
- [ ] The texture is visible in the PaneFlow window at a correct size matching the Servo viewport
- [ ] The element respects GPUI layout (flex sizing, position within a div)
- [ ] Content is not upside-down, color-swapped (BGRA vs RGBA), or otherwise visually corrupted
- [ ] If texture upload fails: document whether GPUI's sprite atlas API supports arbitrary RGBA uploads and what alternative path exists

---

#### US-005: Continuous frame updates from Servo to GPUI

**As a** developer, **I want** the GPUI Element to update when Servo re-renders (e.g., after a CSS animation or page load completion) **so that** I can validate the rendering pipeline supports ongoing updates, not just a single frame.

**Priority:** P1 (Should Have) | **Size:** M (3 pts) | **Dependencies:** US-004

**Acceptance Criteria:**
- [ ] Servo renders a page with a CSS animation (e.g., `@keyframes` color change) or a `<meta http-equiv="refresh">` timer
- [ ] The GPUI Element shows the animation/update visually (frames change over time)
- [ ] Frame rate is measured: document fps achieved with continuous updates
- [ ] No memory leak from repeated buffer uploads (RSS stable over 60 seconds of updates)
- [ ] If updates don't propagate: document whether Servo's callback/delegate notifies on re-render

---

### EP-003: Input Event Bridge

**Definition of done:** Mouse clicks and keyboard input from GPUI reach Servo and produce visible effects in the rendered web page.

---

#### US-006: Forward mouse events from GPUI to Servo

**As a** developer, **I want** mouse clicks on the BrowserElement to be forwarded to Servo **so that** I can validate that interactive web content is feasible.

**Priority:** P1 (Should Have) | **Size:** M (3 pts) | **Dependencies:** US-004

**Servo codebase references:**
- `InputEvent` enum: `components/shared/embedder/input_events.rs:53-64` — `MouseButton`, `MouseMove`, `MouseLeftViewport`, `Wheel`
- `MouseButtonEvent`: same file — `MouseButtonEvent::new(action, button, point)`
- `MouseMoveEvent`: same file — `MouseMoveEvent::new(point)`
- Servoshell mouse handling: `ports/servoshell/desktop/headed_window.rs:291` (click), `:326` (move)
- Coordinate system: device pixels, webview-relative (subtract toolbar offset if any — zero for spike)
- Wheel events: `winit_minimal.rs:132-153` — complete example of `WheelEvent` construction

**Acceptance Criteria:**
- [ ] Mouse click coordinates are translated from GPUI Element-local space to Servo `DevicePoint` (device pixels)
- [ ] Clicks delivered via `webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(...)))`
- [ ] Mouse move delivered via `webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(...)))`
- [ ] A test page with a `<button onclick="document.body.style.background='blue'">` responds to clicks visually
- [ ] If events don't reach Servo: document the event delivery API and what coordinate translation is missing

---

#### US-007: Forward keyboard events from GPUI to Servo

**As a** developer, **I want** keyboard input on the focused BrowserElement to reach Servo **so that** text input in web forms works.

**Priority:** P1 (Should Have) | **Size:** M (3 pts) | **Dependencies:** US-006

**Servo codebase references:**
- `KeyboardEvent`: `components/shared/embedder/input_events.rs` — Servo's keyboard event struct
- Key translation: `ports/servoshell/desktop/keyutils.rs` — converts winit `KeyEvent` → Servo `KeyboardEvent` (reference for GPUI key translation)
- Servoshell keyboard dispatch: `ports/servoshell/desktop/headed_window.rs:664` — `notify_input_event(InputEvent::Keyboard(...))`
- IME support: `ImeEvent` variant in `InputEvent` enum — for text composition

**Acceptance Criteria:**
- [ ] GPUI `KeyDownEvent` is translated to Servo's `KeyboardEvent` format (reference: `keyutils.rs`)
- [ ] `webview.notify_input_event(InputEvent::Keyboard(keyboard_event))` delivers the event
- [ ] A test page with `<input type="text" autofocus>` accepts typed characters
- [ ] Modifier keys (Shift, Ctrl) are correctly mapped
- [ ] If keyboard mapping fails: document the key code format Servo expects vs what GPUI provides

---

### EP-004: Evaluation and Decision

**Definition of done:** A written go/no-go document with benchmarks, screenshots, and blocker analysis.

---

#### US-008: Benchmark rendering performance

**As a** developer, **I want** measured performance data for the Servo → GPUI pipeline **so that** I can assess whether production-quality browser embedding is achievable.

**Priority:** P0 (Must Have) | **Size:** M (3 pts) | **Dependencies:** US-005

**Acceptance Criteria:**
- [ ] Time from `Servo::new()` to first visible frame is measured (cold start latency)
- [ ] Frames per second with continuous CSS animation is measured
- [ ] Input-to-visual-update latency is measured (click → visible change)
- [ ] Memory (RSS) is measured: idle, after page load, after 60s of animation
- [ ] CPU usage is measured during idle and active rendering
- [ ] All measurements documented in a markdown table

---

#### US-009: Test HTML/CSS rendering fidelity

**As a** developer, **I want** to test Servo's rendering against representative web content **so that** I can assess whether the HTML/CSS coverage is sufficient for real-world use.

**Priority:** P1 (Should Have) | **Size:** S (2 pts) | **Dependencies:** US-004

**Acceptance Criteria:**
- [ ] Test pages: basic HTML (headings, paragraphs, links), CSS Flexbox layout, CSS Grid layout, images, SVG, a simple React SPA
- [ ] Each page scored: renders correctly / renders with glitches / fails to render
- [ ] Screenshots saved for each test page
- [ ] If coverage < 70% (fewer than 4/6 pages render acceptably): flag as a blocker for production use

---

#### US-010: Write go/no-go decision document

**As a** developer, **I want** a structured decision document summarizing all spike findings **so that** I can make an informed decision about proceeding with full browser pane implementation.

**Priority:** P0 (Must Have) | **Size:** S (2 pts) | **Dependencies:** US-008, US-009

**Acceptance Criteria:**
- [ ] Document includes: spike objectives vs results (pass/fail per gate), performance benchmarks table, rendering fidelity results, blocker list with severity, architectural notes for production implementation
- [ ] Clear GO or NO-GO recommendation with rationale
- [ ] If NO-GO: document what would need to change (in Servo, GPUI, or PaneFlow) for a future re-evaluation
- [ ] If GO: outline next steps (estimated effort for production browser pane, architectural decisions needed)
- [ ] Document saved at `tasks/spike-servo-webview-results.md`

## Architecture Notes

### Spike Architecture (simplified, in-process)

```
PaneFlow Process
┌──────────────────────────────────────────────────┐
│ Main Thread (GPUI event loop)                     │
│ ├── PaneFlowApp (Entity<Render>)                  │
│ │   ├── TitleBar                                  │
│ │   ├── Sidebar                                   │
│ │   ├── Workspace → SplitNode                     │
│ │   │   └── Leaf(BrowserElement)  ← new           │
│ │   │       ├── request_layout() → fixed size     │
│ │   │       ├── prepaint() → upload pixel buffer  │
│ │   │       └── paint() → draw sprite             │
│ │   └── Event handling                            │
│ │       └── on mouse/key → translate → Servo      │
│ │                                                  │
│ ├── Servo Compositor Thread (spawned by Servo)    │
│ ├── SpiderMonkey JS Thread (spawned by Servo)     │
│ └── Servo Layout Thread (spawned by Servo)        │
│                                                    │
│ Shared: pixel buffer (Arc<Mutex<Vec<u8>>>)        │
│         or Servo callback → cx.notify()           │
└──────────────────────────────────────────────────┘
```

### Rendering Stack (corrected after codebase exploration)

```
Servo (OpenGL/surfman)                    GPUI (wgpu/Vulkan)
┌─────────────────────────┐              ┌─────────────────────────┐
│ WebRender 0.68           │              │ wgpu 29 (Zed fork)      │
│   → gleam (GL bindings)  │              │   → Vulkan/Metal        │
│   → surfman (surfaces)   │              │   → single surface/wnd  │
│   → OSMesa (software)    │              │                         │
│                          │              │                         │
│ glReadPixels(RGBA, u8)   │──RgbaImage──→│ PolychromeSprite upload │
│ image::RgbaImage         │              │ BrowserElement::paint() │
└─────────────────────────┘              └─────────────────────────┘
    ↑ No wgpu at all                         ↑ No OpenGL at all
    (webgpu feature disabled)                (pure wgpu pipeline)
```

### Data Flow: Servo frame → GPUI pixel (verified)

```
1. Layout thread builds display list → PaintMessage::SendDisplayList
2. WebRender backend thread rasterizes → RenderNotifier::new_frame_ready()  [render_notifier.rs:34]
3. EventLoopWaker::wake() pokes GPUI main thread
4. GPUI main thread calls servo.spin_event_loop()                           [servo.rs:184]
5. Servo calls WebViewDelegate::notify_new_frame_ready(webview)             [webview_delegate.rs:891]
6. BrowserView calls webview.paint()                                        [webview.rs:645]
7. Painter::render() → rendering_context.make_current() → prepare_for_rendering() → webrender_renderer.render(size, 0)  [painter.rs:403-439]
8. rendering_context.read_to_image(rect) → glReadPixels → RgbaImage         [rendering_context.rs:679-723]
9. BrowserElement::prepaint() uploads RgbaImage bytes to GPUI sprite atlas
10. BrowserElement::paint() draws sprite quad → wgpu render pass → pixels
```

### Production Architecture (future, if spike passes)

```
PaneFlow Process (GPUI)              Browser Process (Servo)
┌────────────────────┐               ┌────────────────────┐
│ BrowserView Entity │◄── shm/FD ──►│ Servo + WebRender  │
│ BrowserElement     │               │ OffscreenRendering │
│ Event forwarding   │── IPC msg ──►│ notify_input_event │
└────────────────────┘               └────────────────────┘

Linux:   Vulkan external memory FD (zero-copy GPU texture sharing)
macOS:   IOSurface (zero-copy via Surfman)
Windows: Shared memory (CPU copy fallback until GPU path available)
```

### Cross-Platform Texture Sharing Strategy

| Platform | Spike (US-003/004) | Production (future) |
|----------|-------------------|---------------------|
| Linux (Vulkan) | CPU pixel buffer copy | Vulkan FD export (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT`) |
| macOS (Metal) | CPU pixel buffer copy | IOSurface via Surfman → Metal texture import |
| Windows | CPU pixel buffer copy | Shared memory (WebView2 has no GPU export path yet) |

## Dependencies

### External Dependencies (new)

| Dependency | Type | Version | Purpose |
|-----------|------|---------|---------|
| `servo` | Cargo path dep | `path = "/home/arthur/dev/servo/components/servo"` | Browser engine (rlib) |
| SpiderMonkey (`mozjs`) | Transitive (via Servo) | 0.15.7 | JavaScript engine (~15-20 min first build) |
| WebRender | Transitive (via Servo) | 0.68 | OpenGL 2D compositor |
| Surfman | Transitive (via Servo) | 0.11.0 | Cross-platform OpenGL surface management |
| `rustls` | Direct | Latest | Required for `aws_lc_rs` crypto provider init before Servo |
| `image` | Direct | Latest | `RgbaImage` for pixel buffer handling |
| `url` | Direct | Latest | URL parsing for `WebViewBuilder::url()` |

### Internal Dependencies

| Dependency | Location | Purpose |
|-----------|----------|---------|
| GPUI Element trait | `/home/arthur/dev/zed/crates/gpui/src/element.rs` | Custom rendering |
| GPUI canvas element | `/home/arthur/dev/zed/crates/gpui/src/elements/canvas.rs` | Reference for custom draw |
| PaneFlow SplitNode | `src-app/src/split.rs` | Future browser pane integration (not modified in spike) |
| PaneFlow TerminalElement | `src-app/src/terminal_element.rs` | Reference for Element implementation pattern |

## Files NOT to Modify

| File | Reason |
|------|--------|
| `src-app/src/split.rs` | Spike does not integrate into split system |
| `src-app/src/workspace.rs` | Spike does not add browser to workspace tabs |
| `src-app/src/terminal.rs` | Terminal functionality unchanged |
| `src-app/src/terminal_element.rs` | Terminal rendering unchanged |
| `src-app/src/ipc.rs` | IPC not extended for spike |
| Any file in `/home/arthur/dev/zed/` | GPUI must not be forked for this spike |

## Quality Gates

```bash
# Gate 1: Build
cargo build 2>&1 | tail -1    # must show "Finished"

# Gate 2: Servo initializes
RUST_LOG=info cargo run 2>&1 | grep -i "servo"    # Servo logs visible, no panics

# Gate 3: Pixel buffer produced
ls /tmp/paneflow-servo-spike.png    # Debug PNG output exists and is non-empty

# Gate 4: Visible in GPUI
# Manual: launch PaneFlow, verify HTML content visible in BrowserElement

# Gate 5: Input works
# Manual: click button on test page, verify visual change

# Performance
# Documented in tasks/spike-servo-webview-results.md with measured values
```

## Story Dependency Graph

```
US-001 (Cargo dep)
  └── US-002 (Servo init)
        └── US-003 (Render to buffer)
              ├── US-004 (Display in GPUI)
              │     ├── US-005 (Continuous updates)
              │     │     └── US-008 (Benchmarks)
              │     ├── US-006 (Mouse events)
              │     │     └── US-007 (Keyboard events)
              │     └── US-009 (Fidelity testing)
              │
              └── (US-008, US-009 feed into)
                    └── US-010 (Go/No-Go document)
```

## Abort Criteria

The spike should be **immediately aborted** (with findings documented) if:

1. **US-001 fails and cannot be resolved in 2 days** — SpiderMonkey build fails on Fedora 43 or surfman/gleam conflicts with GPUI's deps (wgpu conflict eliminated by disabling `webgpu` feature)
2. **US-002 fails** — Servo's runtime conflicts with GPUI's event loop or allocator at a fundamental level
3. **US-003 produces no output after 3 days** — Servo's `SoftwareRenderingContext` API is too unstable or undocumented to produce a pixel buffer
4. **Memory exceeds 1 GB** for a static HTML page — unacceptable resource footprint

In all abort cases, document findings in `tasks/spike-servo-webview-results.md` with NO-GO recommendation and conditions for re-evaluation.

## Success Metrics

| Metric | Baseline | Spike Target | Measurement |
|--------|----------|-------------|-------------|
| Build succeeds | N/A (no Servo dep) | `cargo build` exit 0 | CI-reproducible command |
| Static page renders | N/A | HTML visible in GPUI | Screenshot evidence |
| Frame rate | N/A | > 15 fps continuous updates | Measured via frame counter |
| Input latency | N/A | < 200ms click-to-visual | Measured via timestamp delta |
| Memory overhead | PaneFlow RSS (~50 MB) | < 500 MB with Servo | `ps aux` RSS |
| Binary size increase | ~80 MB (debug) | < 300 MB increase | `ls -la` delta |

[/PRD]
