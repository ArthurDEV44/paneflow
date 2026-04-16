# PaneFlow

A GPU-accelerated terminal multiplexer for Linux, built in Rust with [Zed's GPUI framework](https://github.com/zed-industries/zed/tree/main/crates/gpui).

PaneFlow aims to be a modern alternative to tmux/screen with native GPU rendering, split panes, workspaces, and session persistence — all without requiring a terminal emulator host.

## Features

- **GPU-accelerated rendering** via Vulkan (GPUI)
- **Split panes** — horizontal and vertical, drag-to-resize, up to 32 panes
- **Workspaces** — up to 20 workspaces with rename, quick-switch (Ctrl+1-9)
- **Session persistence** — save/restore layouts, CWD, and workspace names
- **Themes** — 5 bundled themes with hot-reload (Catppuccin Mocha, One Dark, Dracula, Gruvbox Dark, Solarized Dark)
- **Custom keybindings** — configurable via JSON
- **IPC** — Unix socket JSON-RPC 2.0 for scripting and AI agent integration
- **Client-side decorations** — custom title bar with drag-to-move
- **Wayland + X11** support

## Prerequisites

### System dependencies

Install the required development libraries for your distribution:

**Debian/Ubuntu:**
```bash
sudo apt install build-essential libvulkan-dev libwayland-dev libxkbcommon-dev \
  libx11-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-x11-dev
```

**Fedora:**
```bash
sudo dnf install gcc vulkan-loader-devel wayland-devel libxkbcommon-devel \
  libX11-devel libxcb-devel
```

**Arch Linux:**
```bash
sudo pacman -S base-devel vulkan-icd-loader wayland libxkbcommon libx11 libxcb
```

### Vulkan GPU driver

PaneFlow requires a GPU with Vulkan support:

```bash
# AMD/Intel (Mesa)
sudo apt install mesa-vulkan-drivers        # Debian/Ubuntu
sudo dnf install mesa-vulkan-drivers        # Fedora
sudo pacman -S vulkan-radeon vulkan-intel    # Arch

# NVIDIA
sudo apt install nvidia-vulkan-icd          # Debian/Ubuntu
sudo pacman -S nvidia-utils                 # Arch

# Verify Vulkan works
vulkaninfo --summary
```

### Rust toolchain

Install via [rustup](https://rustup.rs/):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Install from release

Download the latest release from [GitHub Releases](https://github.com/ArthurDEV44/paneflow/releases), then:

```bash
tar xzf paneflow-v0.1.0-x86_64-linux.tar.gz
cd paneflow
./install.sh
```

This installs the binary, desktop entry, and icons to `~/.local/`. PaneFlow will appear in your application launcher.

## Build from source

```bash
git clone https://github.com/ArthurDEV44/paneflow.git
cd paneflow
cargo build --release
./scripts/install.sh
```

## Usage

```bash
# Launch
paneflow

# With logging
RUST_LOG=info paneflow
```

## Keybindings

### Window & workspace management

| Key | Action |
|-----|--------|
| `Ctrl+Shift+N` | New workspace |
| `Ctrl+Shift+Q` | Close workspace |
| `Ctrl+Tab` | Next workspace |
| `Ctrl+Shift+Tab` | Previous workspace |
| `Ctrl+1-9` | Switch to workspace N |

### Pane management

| Key | Action |
|-----|--------|
| `Ctrl+Shift+D` | Split horizontal (top/bottom) |
| `Ctrl+Shift+E` | Split vertical (left/right) |
| `Ctrl+Shift+W` | Close pane |
| `Alt+Arrow` | Focus adjacent pane |

### Terminal

| Key | Action |
|-----|--------|
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste |
| `Shift+PageUp` | Scroll up |
| `Shift+PageDown` | Scroll down |
| `Ctrl+Shift+F` | Search |

## Configuration

Config file location: `~/.config/paneflow/paneflow.json`

```json
{
  "default_shell": "/bin/zsh",
  "theme": "Catppuccin Mocha",
  "font_family": "JetBrains Mono",
  "font_size": 14,
  "line_height": 1.3,
  "window_decorations": "client",
  "option_as_meta": false,
  "shortcuts": {}
}
```

### Available themes

`Catppuccin Mocha` (default), `One Dark`, `Dracula`, `Gruvbox Dark`, `Solarized Dark`

Theme changes are hot-reloaded. Window decorations require a restart (`"client"` = CSD, `"server"` = SSD).

## IPC

PaneFlow exposes a Unix socket at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` using JSON-RPC 2.0.

```bash
# Ping
echo '{"jsonrpc":"2.0","method":"system.ping","id":1}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/paneflow/paneflow.sock

# List workspaces
echo '{"jsonrpc":"2.0","method":"workspace.list","id":1}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/paneflow/paneflow.sock

# Send text to active pane
echo '{"jsonrpc":"2.0","method":"surface.send_text","params":{"text":"ls\n"},"id":1}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/paneflow/paneflow.sock
```

Available methods: `system.ping`, `system.capabilities`, `system.identify`, `workspace.list`, `workspace.current`, `workspace.create`, `workspace.select`, `workspace.close`, `surface.list`, `surface.send_text`, `surface.split`.

## License

[MIT](LICENSE)
