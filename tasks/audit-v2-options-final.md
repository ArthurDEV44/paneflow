# PaneFlow v2 — Options Architecture Finales

**Date:** 2026-04-03
**Sources:** 8 agents (backend, frontend, deps, cmux, web research, zed-terminal, zed-gpui, gpui-research)
**Contexte:** iced 0.14 + WGPU custom a echoue (instable, glyph atlas trop complexe)

---

## Verdict: Option E — GPUI est la moins complexe pour < 5ms

### Pourquoi GPUI gagne

| Critere | iced (echoue) | GPUI | Raison |
|---------|---------------|------|--------|
| Glyph atlas / text rendering | A construire from scratch (US-004 du PRD v2) | **Natif** — cosmic-text (Linux), Core Text (macOS), DirectWrite (Windows) | C'est ce qui a coule iced |
| Terminal renderer | Custom WGPU pipeline a ecrire | **Blueprint Zed** — `terminal_element.rs` (2342L) utilise `Element` trait + `paint_quad()`/text runs | Pas de shader custom |
| PTY integration | A designer | **Blueprint Zed** — `terminal.rs` (3500L) utilise alacritty EventLoop natif avec `FairMutex` | Zed utilise directement alacritty's event_loop, pas portable-pty |
| Keystroke path | winit → iced → ??? → PTY | **Direct** — GPUI KeyDownEvent → `try_keystroke()` → `pty_tx.notify(Cow<[u8]>)` → `Msg::Input` | Zero allocation pour ASCII |
| Exemples terminaux | Aucun | **termy**, **zTerm**, **gpui-terminal** crate | Preuves que ca marche |
| Production proof | COSMIC desktop (System76) | **Zed editor** — des millions d'utilisateurs | Plus battle-tested pour un terminal |
| Widgets | Insuffisants, API instable | `gpui-component` (60+ widgets) + `div()` Tailwind-style | Sidebar, command palette faisables |

### Ce qui rend GPUI moins complexe qu'iced

Le PRD v2 (iced) avait **19 user stories dont 3 stories titanesques** :
- **US-004** : WGPU Glyph Atlas (L, 5pts) — rasterisation `cosmic-text`/`swash`, bin-packing `etagere`, instanced draw pipeline, 256 colors + CJK
- **US-005** : alacritty_terminal Grid Integration (M, 3pts) — dirty-cell tracking, `renderable_content()` iteration
- **US-010** : Demand-Driven Rendering Pipeline (L, 5pts) — tick coalescing, redraw request, vsync

**Avec GPUI, ces 3 stories disparaissent** — le framework gere text rendering et le trait `Element` permet de peindre directement les cellules terminal comme Zed le fait.

### Architecture GPUI pour PaneFlow

```
gpui_platform::application().run(|cx| {
    cx.open_window(WindowOptions { .. }, |window, cx| {
        cx.new(|cx| PaneFlowApp::new(cx))
    });
});

PaneFlowApp
├── Sidebar (GPUI div() + gpui-component widgets)
│   ├── WorkspaceList (paneflow-core::TabManager)
│   └── CommandPalette (overlay)
└── MainContent
    └── SplitLayout (paneflow-core::SplitTree → GPUI flex layout)
        ├── TerminalPane (GPUI Element trait)
        │   ├── alacritty_terminal::Term<ZedListener> (VT state)
        │   ├── alacritty EventLoop (PTY I/O thread)
        │   └── paint() → BatchedTextRuns + Quads
        └── TerminalPane ...

Keystroke path (< 100us):
  GPUI KeyDownEvent
  → terminal.try_keystroke(&keystroke)
  → to_esc_str() → Cow<[u8]>
  → pty_tx.notify(input) → Msg::Input
  → alacritty EventLoop → write(PTY fd)

Output path (< 3ms to pixel):
  PTY fd → alacritty EventLoop read()
  → Term.process_bytes() (VT parse)
  → wakeup → cx.notify() (GPUI redraw request)
  → terminal.sync() → term.renderable_content()
  → Element::paint() → GPUI Scene → GPU submit
```

### Obstacles GPUI et mitigations

| Obstacle | Severite | Mitigation |
|----------|----------|------------|
| 6 crates Zed non publies (gpui_macros, collections, etc.) | Moyenne | Vendorer depuis le monorepo Zed. `git subtree` ou `[patch]` dans Cargo.toml |
| wgpu = fork Zed (branche v29) | Moyenne | Utiliser le meme fork. Pas un probleme tant qu'on suit les updates Zed |
| API pre-1.0, breaking changes | Moyenne | Pin une version precise (0.2.2). Suivre le changelog Zed |
| Pas de winit (calloop + wayland-client/x11rb) | Faible | C'est un avantage : moins de couches, moins de latence |
| Documentation sparse | Faible | Le code Zed EST la documentation. `terminal.rs` + `terminal_element.rs` sont le guide |
| workspace crate Zed non extractible | Faible | paneflow-core a deja SplitTree et TabManager — reimplementer le layout en div() GPUI |

---

## Plan d'action recommande

### Phase 0 — Spike (2-3 jours)
Valider que GPUI fonctionne standalone avec un terminal basique :
1. `cargo init paneflow-v2-spike`
2. Vendorer GPUI + deps depuis le monorepo Zed
3. Ouvrir une fenetre GPUI avec un terminal alacritty_terminal
4. Taper des caracteres, verifier la latence
5. **Decision gate** : si le spike fonctionne en < 8ms, on continue

### Phase 1 — Terminal Core (2 semaines)
- Adapter `terminal.rs` de Zed → `paneflow-terminal-gpui`
- Adapter `terminal_element.rs` → rendu des cellules
- Wire paneflow-core::SplitTree pour le layout flex
- Keystroke path zero-alloc
- PTY via alacritty EventLoop (pas portable-pty)

### Phase 2 — Multiplexer Chrome (2 semaines)
- Sidebar avec WorkspaceList (TabManager)
- Multi-workspace avec switching (Ctrl+1-9)
- Command palette (nucleo fuzzy search)
- Session persistence (JSON autosave defer pendant typing)

### Phase 3 — IPC & CLI (1 semaine)
- Wire paneflow-ipc SocketServer dans l'app GPUI
- Wire paneflow-cli pour le controle distant
- 15 methodes JSON-RPC minimum

### Phase 4 — Polish (1 semaine)
- Debug instrumentation (`#[cfg(debug_assertions)]` typing latency probes)
- Release profiles (LTO, codegen-units=1, strip)
- Cross-platform testing (Linux X11/Wayland, macOS)

**Total estime : 6-7 semaines** (vs 3-4 mois pour iced + WGPU custom)

---

## Crates preserves de PaneFlow v1

| Crate | Preserve? | Notes |
|-------|-----------|-------|
| `paneflow-core` | **OUI** | SplitTree, TabManager, Workspace, Panel — pur domaine, zero dep UI |
| `paneflow-ipc` | **OUI** | SocketServer, Dispatcher, Handlers — pur tokio, zero dep UI |
| `paneflow-config` | **OUI** | Loader, Schema, Watcher — pur notify/serde |
| `paneflow-cli` | **OUI** | Client CLI — pur clap/libc |
| `paneflow-terminal` | **PARTIEL** | PtyManager et emulator.rs supprimes (alacritty EventLoop les remplace). PtyBridge refonde. |
| `src-tauri/` | **SUPPRIME** | Remplace par GPUI app shell |
| `frontend/` | **SUPPRIME** | Remplace par GPUI rendering |

---

## Comparaison finale complexite

| Tache | iced (PRD v2 echoue) | GPUI (propose) |
|-------|---------------------|----------------|
| Glyph atlas + text rendering | **US-004 (L, 5pts)** — from scratch | **0 pts** — GPUI natif |
| Grid cell rendering | **US-005 (M, 3pts)** — custom WGPU | **S, 2pts** — adapter terminal_element.rs |
| Cursor rendering | **US-006 (S, 2pts)** | **XS, 1pt** — adapter Zed's cursor logic |
| Selection + clipboard | **US-007 (M, 3pts)** | **S, 2pts** — alacritty_terminal selection model |
| PTY I/O threads | **US-008 (M, 3pts)** | **XS, 1pt** — alacritty EventLoop le fait |
| Zero-IPC keystroke | **US-009 (M, 3pts)** | **S, 1pt** — GPUI KeyDownEvent direct |
| Demand-driven render | **US-010 (L, 5pts)** | **S, 2pts** — cx.notify() + Element::paint() |
| Output coalescing | **US-011 (M, 3pts)** | **XS, 1pt** — alacritty EventLoop le fait |
| Split layout | **US-012 (M, 3pts)** | **M, 3pts** — GPUI div() flex + SplitTree |
| **Total story points** | **39 pts** | **~15 pts** |

**Reduction de complexite : ~60%**

---

## References Zed (fichiers a etudier)

| Fichier Zed | Lignes | Role pour PaneFlow |
|-------------|--------|-------------------|
| `crates/terminal/src/terminal.rs` | 3500 | Terminal state, keystroke handling, PTY wiring, sync |
| `crates/terminal_view/src/terminal_element.rs` | 2342 | Element impl, cell painting, text batching |
| `crates/terminal_view/src/terminal_view.rs` | 2710 | GPUI view wrapper, focus, key dispatch |
| `crates/terminal_view/src/terminal_panel.rs` | 2368 | Panel integration (dock, workspace) |
| `crates/terminal/src/mappings/keys.rs` | 449 | Keystroke → ANSI escape translation |
| `crates/terminal/src/mappings/colors.rs` | ~200 | Theme color mapping |
| `crates/workspace/src/pane_group.rs` | 1568 | Split tree layout (reference, not extractable) |
