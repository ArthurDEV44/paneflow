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

exit 0
