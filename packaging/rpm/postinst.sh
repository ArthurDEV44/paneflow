#!/bin/sh
# RPM `%post` scriptlet — runs after the package files are written.
# Wires /etc/yum.repos.d/paneflow.repo at `pkg.paneflow.dev` so subsequent
# `dnf upgrade paneflow` pulls new releases automatically (US-016).
#
# Scriptlet argument conventions:
#   $1 = 1 on a fresh install
#   $1 = 2 on an upgrade (from any previous version)
#
# We perform the same action on both — the idempotent `[ -f … ]` check
# handles the upgrade case without overwriting a user-customized .repo
# file. No graceful network fallback is needed here: dnf lazy-fetches
# `gpgkey=URL` on first repo use (not at install time), so a temporary
# outage at install has zero impact.

set -e

REPO=/etc/yum.repos.d/paneflow.repo

if [ ! -f "$REPO" ]; then
    # `gpgcheck=1` — every shipped .rpm is signed with the release key
    # (US-017; see docs/release-signing.md). Clients refuse unsigned or
    # wrong-key packages. `repo_gpgcheck=1` covers the metadata
    # signature (`repomd.xml.asc`) produced by US-015's repo-publish.yml.
    cat > "$REPO" <<'EOF'
[paneflow]
name=PaneFlow
baseurl=https://pkg.paneflow.dev/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://pkg.paneflow.dev/gpg
EOF
    chmod 644 "$REPO"
fi

# --- Icon + desktop cache refresh ------------------------------------
# Without this, the freedesktop hicolor icon cache under
# /usr/share/icons/hicolor/icon-theme.cache and the application DB
# under /usr/share/applications/mimeinfo.cache keep pointing at the
# previous version's artwork, so GNOME Shell / KDE Plasma / docks /
# launchers keep showing stale icons after `dnf upgrade paneflow`
# even though the new PNGs are already on disk.
#
# Both commands are safe to re-run on every install and every upgrade:
# they rebuild deterministically from the current filesystem state.
# The `|| true` guard keeps the transaction green on minimal distros
# that don't ship these tools (server installs, some containers) —
# the icons work everywhere else unaffected.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
fi

exit 0
