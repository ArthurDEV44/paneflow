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

## Install

Pick the format that matches your distro. Every format ships the same
binary — the differences are how updates arrive and where files land on
disk. All artifacts are attached to the [latest GitHub
release](https://github.com/ArthurDEV44/paneflow/releases/latest).

> **Substitute placeholders before running.** The commands below use
> two placeholders:
> - `vX.Y.Z` — replace with the release tag on the GitHub Releases
>   page (e.g., `v0.2.0`). Copying the command verbatim will 404.
> - `<ARCH>` — replace with `x86_64` or `aarch64` depending on your
>   machine (check with `uname -m`).
>
> To auto-resolve the latest version:
> ```bash
> VER=$(curl -fsSL https://api.github.com/repos/ArthurDEV44/paneflow/releases/latest \
>       | grep -oE '"tag_name":\s*"v[^"]+"' | cut -d\" -f4)
> ARCH=$(uname -m)
> ```
> Then paste `$VER` and `$ARCH` in place of the placeholders below.

### Ubuntu / Debian / Mint (apt + repo)

One-shot install of the `.deb`. The package's `postinst` automatically
wires `pkg.paneflow.dev` into `/etc/apt/sources.list.d/paneflow.list`,
so `apt upgrade` pulls subsequent releases:

```bash
curl -LO https://github.com/ArthurDEV44/paneflow/releases/latest/download/paneflow-vX.Y.Z-<ARCH>.deb
# Verify the signature BEFORE `apt install` — postinst runs as root,
# so an unsigned/tampered .deb could write arbitrary repo sources.
sudo apt install -y dpkg-sig
dpkg-sig --verify paneflow-vX.Y.Z-<ARCH>.deb       # expect: GOODSIG
sudo apt install ./paneflow-vX.Y.Z-<ARCH>.deb
paneflow --version
```

Future updates:

```bash
sudo apt update && sudo apt upgrade paneflow
```

### Fedora / RHEL / Rocky (dnf + repo)

Same pattern with `.rpm`. The `%post` scriptlet drops
`/etc/yum.repos.d/paneflow.repo` pointing at `pkg.paneflow.dev/rpm`:

```bash
curl -LO https://github.com/ArthurDEV44/paneflow/releases/latest/download/paneflow-vX.Y.Z-<ARCH>.rpm
# Verify the signature BEFORE `dnf install` — %post runs as root.
sudo rpm --import https://pkg.paneflow.dev/gpg      # see Troubleshooting for the TOFU caveat
rpm --checksig paneflow-vX.Y.Z-<ARCH>.rpm           # expect: digests signatures OK
sudo dnf install ./paneflow-vX.Y.Z-<ARCH>.rpm
paneflow --version
```

Future updates:

```bash
sudo dnf upgrade paneflow
```

### AppImage (portable, any distro)

Single-file download. No install, no root — double-click to run or
launch from the terminal. Best for distros with no official `.deb` /
`.rpm` and for trying PaneFlow on a machine you don't manage:

```bash
curl -LO https://github.com/ArthurDEV44/paneflow/releases/latest/download/paneflow-vX.Y.Z-<ARCH>.AppImage
chmod +x paneflow-vX.Y.Z-<ARCH>.AppImage
./paneflow-vX.Y.Z-<ARCH>.AppImage --version   # `paneflow --version` equivalent (AppImage is not on $PATH)
```

Ubuntu 24.04+ and Fedora Silverblue ship without FUSE 2 by default —
see [Troubleshooting → AppImage won't run on Ubuntu
24.04](#appimage-wont-run-on-ubuntu-2404) if the command above fails.

### Tarball (immutable distros, no-root installs)

Use this on Fedora Silverblue, SteamOS, NixOS, or any machine where
you'd rather not touch `/usr`. Installs to `~/.local/paneflow.app/`
and symlinks `~/.local/bin/paneflow`:

```bash
curl -LO https://github.com/ArthurDEV44/paneflow/releases/latest/download/paneflow-vX.Y.Z-<ARCH>.tar.gz
curl -LO https://github.com/ArthurDEV44/paneflow/releases/latest/download/paneflow-vX.Y.Z-<ARCH>.tar.gz.sha256
sha256sum --check paneflow-vX.Y.Z-<ARCH>.tar.gz.sha256
tar xzf paneflow-vX.Y.Z-<ARCH>.tar.gz
./paneflow.app/install.sh
~/.local/bin/paneflow --version
```

The in-app updater atomically swaps `~/.local/paneflow.app/` on new
releases (no package manager involved).

## Troubleshooting

### AppImage won't run on Ubuntu 24.04

Ubuntu 24.04 and newer don't ship `libfuse2` by default. The error
looks like `dlopen(): error loading libfuse.so.2`. Two fixes:

- **Preferred:** skip FUSE by mounting the AppImage in-process:
  ```bash
  ./paneflow-vX.Y.Z-x86_64.AppImage --appimage-extract-and-run
  ```
- **Alternative:** install the compatibility shim
  ```bash
  sudo apt install libfuse2t64
  ```
  On Ubuntu 22.04 the package is `libfuse2` instead. Installing
  `libfuse2` on 24.04 can remove `ubuntu-session` as a transitive
  conflict — `--appimage-extract-and-run` avoids that risk.

### Verify GPG signatures

Every `.deb` and `.rpm` is signed with `paneflow-release@paneflow.dev`.
The public key is published in two places:

1. `https://pkg.paneflow.dev/gpg` (served from Cloudflare R2 alongside
   the packages).
2. [`packaging/paneflow-release.asc`](packaging/paneflow-release.asc)
   inside this repository.

**Trust model:** the repo-committed copy is signed by Arthur's Git
commit history and is not reachable from an R2-bucket compromise —
prefer it over the `pkg.paneflow.dev/gpg` endpoint for first-import,
then cross-check the fingerprint. Both keys must report the same
fingerprint:

```bash
# Fetch the repo-committed copy and print its fingerprint
curl -fsSL https://raw.githubusercontent.com/ArthurDEV44/paneflow/main/packaging/paneflow-release.asc \
  | gpg --with-fingerprint --with-colons \
  | awk -F: '/^fpr:/ {print $10; exit}'
# Then fetch the repo-served copy and repeat; the two fingerprints
# MUST match. If they differ, stop — the package repo is compromised.
curl -fsSL https://pkg.paneflow.dev/gpg \
  | gpg --with-fingerprint --with-colons \
  | awk -F: '/^fpr:/ {print $10; exit}'
```

**.deb:**

```bash
# One-time: install the repo-committed key as the APT trust anchor.
# Using the GitHub-hosted raw file insulates the import from an
# R2-bucket compromise (see Trust model above).
curl -fsSL https://raw.githubusercontent.com/ArthurDEV44/paneflow/main/packaging/paneflow-release.asc \
  | gpg --dearmor \
  | sudo tee /usr/share/keyrings/paneflow-archive.gpg >/dev/null
# Verify a downloaded .deb
sudo apt install dpkg-sig
dpkg-sig --verify paneflow-vX.Y.Z-x86_64.deb   # expect: GOODSIG
```

**.rpm:**

```bash
# Prefer the repo-committed copy for the same TOFU reason as above.
sudo rpm --import https://raw.githubusercontent.com/ArthurDEV44/paneflow/main/packaging/paneflow-release.asc
rpm --checksig paneflow-vX.Y.Z-x86_64.rpm      # expect: digests signatures OK
```

**.tar.gz:** integrity-only check against the sibling `.sha256` file
attached to the release:

```bash
sha256sum --check paneflow-vX.Y.Z-x86_64.tar.gz.sha256
```

**AppImage:** self-validating — `appimageupdatetool` verifies the
zsync metadata before delta updates.

### Uninstall

**.deb (Ubuntu/Debian):**

```bash
sudo apt remove paneflow                              # keep config
sudo apt purge paneflow                               # remove config too
sudo rm /etc/apt/sources.list.d/paneflow.list         # remove APT source
sudo rm /usr/share/keyrings/paneflow-archive.gpg      # remove signing key
```

**.rpm (Fedora/RHEL):**

```bash
sudo dnf remove paneflow
sudo rm /etc/yum.repos.d/paneflow.repo                # remove DNF source
```

**AppImage:** just `rm paneflow-vX.Y.Z-<ARCH>.AppImage`. Nothing else
is installed on disk (PaneFlow's AppImage doesn't write to `~/.local`
or `/etc`).

**.tar.gz:**

```bash
rm -rf ~/.local/paneflow.app ~/.local/bin/paneflow \
       ~/.local/share/applications/paneflow.desktop
for s in 16 32 48 128 256 512; do
    rm -f ~/.local/share/icons/hicolor/${s}x${s}/apps/paneflow.png
done
# Optional: remove config and cache
rm -rf ~/.config/paneflow ~/.cache/paneflow
```

## Build from source

```bash
git clone https://github.com/ArthurDEV44/paneflow.git
cd paneflow
cargo build --release
bash scripts/bundle-tarball.sh
tar xzf target/bundle/paneflow-*.tar.gz -C /tmp
/tmp/paneflow.app/install.sh
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
