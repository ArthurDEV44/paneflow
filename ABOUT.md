# À propos de PaneFlow

PaneFlow est un espace de travail terminal natif, écrit en Rust, conçu pour faire tourner des agents de codage en parallèle.

## En une phrase

Un multiplexeur de terminaux GPU-natif (via le framework GPUI de Zed) où tu lances, surveilles et orchestres plusieurs agents IA (Claude Code, Codex, Gemini, opencode) côte à côte dans une seule fenêtre.

## Ce qui le distingue

- **Natif, pas TUI.** Rendu GPU via GPUI + émulation VT par `alacritty_terminal`, là où la plupart des multiplexeurs bricolent en TUI.
- **Pensé pour les agents.** Workspaces, splits N-aires, détection de serveurs de dev, badges de branche git, et un pont MCP (`paneflow-mcp`) qui laisse un agent lire la sortie des autres panes.
- **Cross-platform.** Linux (Wayland + X11), macOS (Intel + Apple Silicon), Windows (en cours).
- **OSS et gratuit par design.** GPL-3.0-or-later.

## Démarrer

```bash
cargo run                 # build debug (nécessite un GPU avec Vulkan)
RUST_LOG=info cargo run    # avec logs
```

Détails d'architecture, conventions et commandes : voir [`CLAUDE.md`](CLAUDE.md), [`ARCHITECTURE.md`](ARCHITECTURE.md) et [`README.md`](README.md).
Site : [paneflow.dev](https://paneflow.dev)
