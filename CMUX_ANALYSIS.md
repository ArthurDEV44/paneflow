# cmux Complete Codebase Analysis — Cross-Platform Port Reference

> Generated 2026-04-01 from deep exploration of 111 Swift files, Go daemon, docs, and test suite.

---

## 1. What Is cmux?

**cmux** is a native macOS terminal multiplexer built for AI coding agent workflows. It wraps the **Ghostty** GPU-accelerated terminal engine (Zig, via C FFI) in a SwiftUI+AppKit application with an embedded WebKit browser, markdown viewer, and 150+ command socket API.

- **License:** AGPL-3.0-or-later
- **Current version:** 0.63.1
- **Target audience:** Developers running parallel AI agent sessions (Claude Code, Codex, OpenCode, Copilot CLI)
- **Core value:** Scriptable terminal+browser environment where agents can be observed and controlled without focus-switching

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     cmuxApp (@main)                          │
│  StateObjects: TabManager, NotificationStore, SidebarState   │
│  NSApplicationDelegateAdaptor → AppDelegate (13K lines)      │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│                   ContentView (14K lines)                     │
│  [Sidebar] | [ZStack of mounted WorkspaceContentViews]       │
│  Command Palette state machine                               │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│              TabManager (ObservableObject)                    │
│  tabs: [Workspace], selectedTabId, tab history               │
│  Git metadata probing, port ordinals, title coalescing       │
└──────────────────────┬──────────────────────────────────────┘
                       │ owns ordered array of
                       ▼
┌─────────────────────────────────────────────────────────────┐
│            Workspace (= Tab, ObservableObject)               │
│  bonsplitController: BonsplitController (tiling engine)      │
│  panels: [UUID: any Panel]                                   │
│  surfaceIdToPanelId: [BonsplitTabID → PanelUUID]            │
│  Per-panel: directories, titles, git, ports, status          │
│  Remote: SSH session controller, relay, proxy                │
└──────────────────────┬──────────────────────────────────────┘
                       │
            ┌──────────┼──────────┐
            ▼          ▼          ▼
     TerminalPanel  BrowserPanel  MarkdownPanel
     (Ghostty FFI)  (WKWebView)   (DispatchSource watch)
            │          │
            ▼          ▼
     Portal System: AppKit NSViews pinned above SwiftUI
     via frame geometry synchronization
```

### Key Patterns

| Pattern | Implementation | Purpose |
|---------|---------------|---------|
| Portal | WindowTerminalPortal, WindowBrowserPortal | Host AppKit views (Metal terminal, WKWebView) above SwiftUI hierarchy |
| Bonsplit | External library (vendor/bonsplit) | Binary-tree split pane layout with tab bars |
| V1+V2 Socket | TerminalController | Unix domain socket IPC (text + JSON-RPC) |
| C FFI | ghostty.h (1168 lines) → GhosttyKit.xcframework | Ghostty terminal engine integration |
| Combine | @Published + sidebarObservationPublisher | Reactive state propagation |
| ObjC Swizzle | NSWindow.sendEvent, performKeyEquivalent, makeFirstResponder | Global keyboard/event interception |

---

## 3. Complete Feature Inventory

### Terminal
- GPU-accelerated rendering (Ghostty/Metal)
- Full VT emulation (Ghostty owns 100% of PTY/parsing)
- Ghostty config compatibility (~/.config/ghostty/config)
- CJK IME support (Korean, Chinese, Japanese)
- Find-in-terminal (Cmd+F)
- Keyboard copy mode (vi-style)
- Clipboard image paste, drag-and-drop files
- Voice dictation text insertion
- Terminal bells (OSC 9/99/777)
- Port scanning per-workspace (lsof/ps, 6-scan burst)
- SSH session detection (sysctl KERN_PROCARGS2)
- Remote file upload via SCP
- Scrollback persistence (4000 lines max)
- `read-screen` / `capture-pane` CLI

### Workspace & Layout
- N workspaces per window, multi-window support
- Binary-tree splits (horizontal + vertical) via Bonsplit
- Pane tabs (multiple surfaces per pane)
- Drag-and-drop: tabs within/across panes, workspaces in sidebar
- Cross-window workspace moves
- Pane zoom/maximize
- Attention flash (Cmd+Shift+H)
- Minimal mode (hidden titlebar)
- Session persistence (autosave every 8s)
- Command palette (Cmd+Shift+P) with fuzzy search

### Browser (WKWebView)
- Full embedded browser with omnibar
- Search engine: Google, DuckDuckGo, Bing, Kagi
- History with frecency scoring
- Browser profiles with isolated WKWebsiteDataStore
- Profile import from Chrome, Firefox, Safari, Arc, 20+ browsers
- Developer Tools (docked or floating)
- 60+ Playwright-style automation commands
- ReactGrab component inspector
- File drag-and-drop, camera/mic permissions
- window.open() popup support
- Insecure HTTP allowlist

### Notifications
- OSC 9/99/777 detection
- Blue ring on panes, sidebar badge, dock badge, menu bar extra
- macOS system notifications (UNUserNotificationCenter)
- Custom notification sounds
- `cmux notify` CLI for agent hooks
- Jump to unread (Cmd+Shift+U)

### SSH / Remote
- `cmux ssh user@host` creates remote workspace
- cmuxd-remote Go daemon (auto-deployed, SHA-256 verified)
- Browser traffic proxied via SOCKS5/HTTP CONNECT
- HMAC-SHA256 relay authentication
- PTY resize: "smallest screen wins" semantics
- Reverse TCP port forwarding

### CLI (100+ commands)
- V1 text + V2 JSON-RPC protocols on same socket
- Window/workspace/pane/surface CRUD
- Text/key sending
- Notification/status/progress/log
- Browser automation (60+ methods)
- tmux compatibility shim
- Agent integrations (Claude Code, Codex, OpenCode)

### Configuration
- JSON config: ~/.config/cmux/cmux.json (global) + per-directory cmux.json (local)
- Hot-reload via kqueue (DispatchSource)
- Directory trust model for local commands
- 34 customizable keyboard shortcuts
- Socket access control (5 modes: off/cmuxOnly/automation/password/allowAll)

---

## 4. Platform-Specific Dependencies (Port Blockers)

### Critical (must replace)

| Component | macOS Implementation | Cross-Platform Alternative |
|-----------|---------------------|---------------------------|
| **Terminal rendering** | Ghostty via Metal + CAMetalLayer + IOSurface | Ghostty Linux (Vulkan/OpenGL) or alternative: alacritty_terminal, wezterm, VTE |
| **Terminal engine FFI** | GhosttyKit.xcframework (Zig → C ABI) | Ghostty has Linux support upstream; or use a Rust terminal emulator library |
| **Browser engine** | WKWebView (WebKit) | WebView2 (Windows), WebKitGTK (Linux), or CEF (Chromium, cross-platform) |
| **UI framework** | SwiftUI + AppKit | egui, iced, Tauri, GTK4, or web-based (Electron/Tauri) |
| **Window management** | NSWindow, NSView, NSToolbar | Platform-native or cross-platform windowing (winit, SDL2) |
| **Auto-update** | Sparkle | WinSparkle, AppImage, Flatpak, or custom |

### Medium (needs adaptation)

| Component | macOS Implementation | Cross-Platform Alternative |
|-----------|---------------------|---------------------------|
| Hot-reload config | kqueue via DispatchSource | inotify (Linux), ReadDirectoryChangesW (Windows), or notify crate (Rust) |
| Keyboard shortcuts | UserDefaults + Carbon TIS/UCKeyTranslate | xkbcommon (Linux), Win32 (Windows), or crossterm |
| Socket security | getsockopt(LOCAL_PEERPID) ancestry check | /proc/<pid>/status ppid (Linux), GetNamedPipeClientProcessId (Windows) |
| Port scanning | /bin/ps + /usr/sbin/lsof | /proc/net/tcp (Linux), netstat (Windows), or ss |
| SSH detection | sysctl(CTL_KERN, KERN_PROCARGS2) | /proc/<pid>/cmdline (Linux) |
| Notifications | UNUserNotificationCenter, NSSound | libnotify/D-Bus (Linux), WinRT Toast (Windows) |
| Clipboard | NSPasteboard | wl-clipboard/xclip (Linux), Win32 clipboard |
| AppleScript | NSScriptCommand / OSAX | D-Bus (Linux), COM (Windows), or socket-only scripting |
| Session persistence path | ~/Library/Application Support/ | $XDG_DATA_HOME (Linux), %APPDATA% (Windows) |
| Password storage | Keychain → file migration | Secret Service/keyring (Linux), Credential Manager (Windows) |

### Already Cross-Platform

| Component | Notes |
|-----------|-------|
| cmuxd-remote (Go daemon) | Pre-built for darwin+linux × arm64+amd64 |
| Socket IPC protocol | POSIX sockets work on all Unix; V1+V2 JSON-RPC is language-agnostic |
| Config format (JSON) | Fully portable |
| CLI command grammar | Language-agnostic; protocol-based |
| Browser find JS | Pure JavaScript string generation |
| RemoteRelayZshBootstrap | Pure string generation |
| Shell integrations (zsh/bash) | Already work on Linux |

---

## 5. Rust vs TypeScript Analysis

### Rust

**Pros:**
- Native performance matching Swift (zero-cost abstractions, no GC)
- Excellent terminal ecosystem: crossterm, alacritty_terminal, wezterm
- Strong cross-platform story: winit, egui, iced, ratatui
- Ghostty is Zig — Rust has excellent C FFI (can call ghostty.h directly)
- Async runtime (tokio) maps well to the event-driven architecture
- Memory safety without GC (matches Swift's ARC model conceptually)
- notify crate for file watching, nix crate for Unix APIs
- Tauri for browser integration (WebView2/WebKitGTK)

**Cons:**
- UI frameworks less mature than AppKit/SwiftUI (egui, iced still evolving)
- Browser integration: CEF bindings exist but are heavy; Tauri is promising but opinionated
- Steeper learning curve for contributors
- Bonsplit-equivalent tiling library would need to be built

**Best for:** Terminal core, daemon, CLI, socket server, config system, session persistence

### TypeScript (Electron/Tauri)

**Pros:**
- Electron gives a full Chromium browser for free (no WKWebView porting needed)
- Tauri gives WebView2/WebKitGTK with smaller footprint
- Fast UI development with React/Vue/Solid
- Huge ecosystem for browser automation (Playwright patterns already match cmux's API)
- Lower barrier for contributors
- xterm.js for terminal (proven, used by VS Code)

**Cons:**
- Electron: heavy memory footprint (contradicts cmux's "not Electron" philosophy)
- Tauri: limited browser API surface compared to WKWebView (no DevTools docking, limited WKWebsiteDataStore equivalent)
- xterm.js performance < Ghostty Metal rendering
- No direct Ghostty FFI (would need to abandon Ghostty for xterm.js)
- Node.js async model is different from Swift's MainActor pattern

**Best for:** Browser panels, UI layer, rapid prototyping

### Recommendation: Hybrid Rust Core + Tauri UI

```
┌────────────────────────────────────────────┐
│              Tauri Shell (UI)               │
│  React/Solid frontend for:                 │
│  - Sidebar, command palette, notifications │
│  - Browser panels (WebView2/WebKitGTK)     │
│  - Settings, omnibar, update UI            │
└──────────────────┬─────────────────────────┘
                   │ IPC (Tauri commands)
                   ▼
┌────────────────────────────────────────────┐
│           Rust Backend Core                 │
│  - Terminal emulation (alacritty_terminal  │
│    or portable_pty + vte)                  │
│  - Workspace/Tab/Panel state management    │
│  - Unix socket server (V1+V2 protocol)    │
│  - Config loading + hot-reload (notify)    │
│  - Session persistence                     │
│  - SSH/remote (reuse cmuxd-remote as-is)  │
│  - Port scanning                           │
│  - Keyboard shortcut registry              │
│  - CLI binary (clap)                       │
└────────────────────────────────────────────┘
```

---

## 6. Porting Strategy (Priority Order)

### Phase 1: Foundation (MVP — Terminal + Workspaces)
1. **Rust workspace model**: Workspace, TabManager, Panel trait
2. **Terminal backend**: portable_pty + alacritty_terminal (or vte)
3. **Tiling engine**: Port Bonsplit concept (binary-tree splits)
4. **Socket server**: V2 JSON-RPC protocol (subset: workspace/pane/surface CRUD)
5. **CLI**: Rust binary with clap (port command grammar)
6. **Config**: JSON config loader with notify-based hot-reload
7. **Tauri shell**: Basic window with sidebar + terminal panes

### Phase 2: Browser + Notifications
8. **Browser panels**: Tauri WebView integration
9. **Notification system**: Desktop notifications (notify-rust)
10. **Session persistence**: JSON snapshot save/restore
11. **Keyboard shortcuts**: Custom shortcut registry
12. **Command palette**: Fuzzy search (skim/nucleo crate)

### Phase 3: Remote + Agent Integration
13. **SSH workspaces**: Reuse cmuxd-remote Go daemon as-is
14. **Shell integrations**: Port zsh/bash scripts (mostly portable)
15. **Agent hooks**: Claude Code, Codex integration
16. **Browser automation API**: Playwright-style commands via WebView

### Phase 4: Polish
17. **Omnibar with history/suggestions**
18. **Browser profiles**
19. **Markdown viewer panel**
20. **AppleScript equivalent** (D-Bus on Linux, COM on Windows, or socket-only)
21. **Auto-update system**
22. **Telemetry** (opt-in)

---

## 7. Key Domain Objects to Port

| cmux Swift Class | Responsibility | Port Priority |
|-----------------|----------------|---------------|
| `Workspace` | Core domain model (10K lines) | P0 |
| `TabManager` | Workspace list + selection (5K lines) | P0 |
| `Panel` (protocol) | Panel abstraction (terminal/browser/markdown) | P0 |
| `TerminalPanel` | Ghostty terminal wrapper | P0 |
| `BonsplitController` | Tiling/split engine | P0 |
| `TerminalController` | Unix socket IPC server | P0 |
| `CmuxConfig` | JSON config + hot-reload | P0 |
| `SessionPersistence` | Session save/restore | P1 |
| `KeyboardShortcutSettings` | Shortcut registry (34 actions) | P1 |
| `BrowserPanel` | WKWebView wrapper (most complex panel) | P1 |
| `TerminalNotificationStore` | Notification state machine | P1 |
| `PortScanner` | TCP port detection | P2 |
| `TerminalSSHSessionDetector` | SSH session detection | P2 |
| `SocketControlSettings` | Socket security (5 modes) | P2 |
| `CmuxDirectoryTrust` | Trust model for local commands | P2 |
| `AppleScriptSupport` | Scripting API | P3 |
| `WorkspaceRemoteSessionController` | SSH daemon management | P3 |

---

## 8. Socket Protocol Reference (V2 JSON-RPC)

### Method Families
- `system.*` — ping, capabilities, identify
- `window.*` — list, current, focus, create, close
- `workspace.*` — list, create, select, current, close, move_to_window, reorder
- `pane.*` — list, focus, surfaces, create, last
- `surface.*` — list, focus, split, create, close, drag_to_split, refresh, health, send_text, send_key, trigger_flash, move, reorder
- `browser.*` — 60+ Playwright-style methods
- `notification.*` — create, list, clear
- `tab.action`, `app.*`, `debug.*`

### Handle Format
- Short refs: `window:1`, `workspace:3`, `pane:5`, `surface:7`
- UUIDs also available
- `--id-format refs|uuids|both`

### Security Modes
| Mode | Socket perms | Auth | Who connects |
|------|-------------|------|-------------|
| off | N/A | — | Nobody |
| cmuxOnly (default) | 0600 | ppid ancestry | cmux child processes only |
| automation | 0600 | None | Same user |
| password | 0600 | Password file | Anyone with password |
| allowAll | 0666 | None | Any local process |

---

## 9. Config Schema Reference

```json
{
  "commands": [
    {
      "name": "My Workspace",
      "description": "Optional description",
      "keywords": ["optional", "search", "terms"],
      "restart": "recreate|ignore|confirm",
      "confirm": false,
      "workspace": {
        "name": "Workspace Title",
        "cwd": "~/projects/myapp",
        "color": "#C0392B",
        "layout": {
          "direction": "horizontal",
          "split": 0.5,
          "children": [
            {
              "pane": {
                "surfaces": [
                  {
                    "type": "terminal",
                    "name": "Server",
                    "command": "npm run dev",
                    "cwd": "./backend",
                    "env": { "PORT": "3000" },
                    "focus": true
                  }
                ]
              }
            },
            {
              "pane": {
                "surfaces": [
                  {
                    "type": "browser",
                    "name": "Preview",
                    "url": "http://localhost:3000"
                  }
                ]
              }
            }
          ]
        }
      }
    }
  ]
}
```

---

## 10. Test-Derived Feature Requirements (from 30+ unit tests + 16 UI tests)

Key behavioral contracts discovered from tests:

1. **Workspace close semantics**: Ctrl+D closes only exited/focused terminal; Cmd+D closes last workspace then window; Cmd+W last tab keeps window open
2. **Focus stability**: Closing selected workspace selects same index; closing last selects previous
3. **Manual unread**: 200ms grace interval for same-panel focus; clears on different panel focus
4. **Command palette fuzzy search**: Exact > prefix > contains; 1-edit-distance tolerance; must be < 1.25x reference performance
5. **CJK IME**: performKeyEquivalent returns false during composition; voice input (nil currentEvent) works
6. **Browser key routing**: Cmd+N/W/F route to app menu, not WebKit, when webview is first responder
7. **Config validation**: Blank names rejected; split must have exactly 2 children; pane must have >= 1 surface
8. **Socket security**: password mode rejects unauthenticated; wrong password doesn't leak state
9. **Session restore**: Skipped under XCTest; skipped with CLI arguments; allowed for Finder launch
10. **Pinned workspaces**: Cannot be closed via socket (returns `protected` error with `pinned: true`)
