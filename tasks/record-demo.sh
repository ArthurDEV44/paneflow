#!/usr/bin/env bash
# Chorégraphie IPC pour le GIF démo du README (cf. tasks/hn-launch-playbook.md §1.1).
#
# Usage :
#   1. Lance Paneflow avec une fenêtre PROPRE (pas de workspaces persos visibles).
#   2. Démarre l'enregistrement (Kooha → format MP4/WebM, région = fenêtre Paneflow).
#   3. ./tasks/record-demo.sh /chemin/vers/un/repo/de/demo
#   4. Sur caméra : Enter sur le prompt préremplie de Claude, idem Codex,
#      réponds à une question d'agent, Ctrl+Shift+J pour le jump.
#   5. Stoppe l'enregistrement, convertis (commande affichée à la fin).
#
# Les prompts sont PRÉREMPLIS, jamais soumis — c'est toi qui presses Enter
# (human-in-the-loop, c'est le pitch).

set -euo pipefail

DEMO_REPO="${1:?usage: record-demo.sh /path/to/demo/repo}"
SOCK="${XDG_RUNTIME_DIR:-/run/user/$UID}/paneflow/paneflow.sock"

[ -S "$SOCK" ] || { echo "socket Paneflow introuvable: $SOCK (l'app tourne ?)"; exit 1; }
command -v socat >/dev/null || { echo "socat requis (sudo dnf install socat)"; exit 1; }

rpc() {
  printf '%s\n' "$1" | socat - UNIX-CONNECT:"$SOCK"
  sleep "${2:-1.2}"
}

echo ">> ping"
rpc '{"jsonrpc":"2.0","method":"system.ping","id":0}' 0.3

echo ">> workspace démo"
rpc "{\"jsonrpc\":\"2.0\",\"method\":\"workspace.create\",\"params\":{\"name\":\"demo\",\"cwd\":\"$DEMO_REPO\"},\"id\":1}" 1.5

echo ">> pane Claude Code (prompt préremplie, NE PAS soumettre via script)"
rpc "{\"jsonrpc\":\"2.0\",\"method\":\"surface.split\",\"params\":{\"direction\":\"vertical\",\"cwd\":\"$DEMO_REPO\",\"command\":\"claude\",\"name\":\"claude\",\"prompt\":\"Add a --json flag to the export command, with tests\"},\"id\":2}" 2.5

echo ">> pane Codex (prompt préremplie)"
rpc "{\"jsonrpc\":\"2.0\",\"method\":\"surface.split\",\"params\":{\"direction\":\"horizontal\",\"cwd\":\"$DEMO_REPO\",\"command\":\"codex\",\"name\":\"codex\",\"prompt\":\"Profile the startup path and list the 3 slowest spans\"},\"id\":3}" 2.5

cat <<'EOF'

Chorégraphie lancée. À toi : Enter sur chaque prompt, laisse tourner,
réponds à une question, Ctrl+Shift+J, 2 s sur la vue d'ensemble, coupe.

Conversion (vise < 10 Mo) :
  gifski --fps 12 --quality 80 --width 1600 -o assets/images/demo.gif capture.mp4

Puis dans README.md, remplace hero-paneflow.png par demo.gif.
EOF
