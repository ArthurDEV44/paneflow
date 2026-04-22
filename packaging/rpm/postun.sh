#!/bin/sh
# RPM `%postun` scriptlet — runs after the package files are removed.
# Rebuilds the freedesktop icon + desktop caches so the vanishing
# `/usr/share/icons/hicolor/*/apps/paneflow.png` entries and the
# removed `paneflow.desktop` stop being surfaced by GNOME Shell / KDE
# Plasma / docks / launchers.
#
# Scriptlet argument conventions:
#   $1 = 0  → final uninstall (the package is going away for good)
#   $1 = 1  → upgrade in progress (a newer version was just written;
#             its own %post already refreshed the caches, so this run
#             would be redundant but still harmless)
#
# We unconditionally refresh: the `-f` flag makes rebuild idempotent
# and the extra ~50 ms on upgrade is cheaper than a cache-state bug.
# The `|| true` guard keeps the transaction green on minimal distros
# that don't ship these tools.

set -e

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
fi

exit 0
