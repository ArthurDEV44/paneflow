// Single source of truth for the current release metadata referenced
// across the marketing site (Hero CTA, Download page primary card, and
// any future callsite). Bump LATEST_VERSION on every release cut;
// download URLs derive from it so a single edit propagates everywhere.
//
// Historical versions on the download page are maintained in
// `components/download/download-view.tsx` (VERSIONS array) — this
// module only tracks "latest".

export const LATEST_VERSION = "0.2.10";

export type LinuxArch = "x86_64" | "aarch64";

const RELEASE_BASE = `https://github.com/ArthurDEV44/paneflow/releases/download/v${LATEST_VERSION}`;

/**
 * Direct-download URL for the recommended Linux binary. AppImage is
 * universal (no root, no dep resolution, runs on every modern distro)
 * so it is the default "big green button" target across the site.
 *
 * Callers should only need arch. The URL is on the GitHub Releases
 * CDN (not a redirect page), so `<a href>` triggers an immediate
 * browser download.
 *
 * Filename convention as of v0.3.0: `paneflow-<semver>-<arch>.AppImage`
 * (no `v` prefix on the version segment), matching macOS DMG and
 * Windows MSI naming. Versions <= v0.2.x carry `paneflow-v<semver>-...`;
 * the in-app updater (`update_checker.rs::pick_asset`) is suffix-only,
 * so the rename is transparent across the v0.2 -> v0.3 boundary.
 */
export function linuxAppImageUrl(arch: LinuxArch): string {
  return `${RELEASE_BASE}/paneflow-${LATEST_VERSION}-${arch}.AppImage`;
}

/**
 * Direct-download URL for the macOS Apple Silicon `.dmg`. The bundle is
 * signed with a Developer ID Application certificate and Apple-notarized
 * (the ticket is stapled), so Gatekeeper accepts it on first launch
 * without an "unidentified developer" prompt.
 *
 * Filename convention: `paneflow-<semver>-aarch64-apple-darwin.dmg` —
 * no `v` prefix on the version segment, matching
 * `update_checker.rs::pick_asset` (US-008). Linux assets carry
 * `paneflow-v<semver>-…` because they predate that convention.
 *
 * Apple Silicon only as of v0.2.10. The `x86_64-apple-darwin` (Intel
 * Mac) target is a closed CI target until v0.3.0; Intel users either
 * run the AppImage under a Linux VM or wait for the cut.
 */
export function macOSDmgUrl(): string {
  return `${RELEASE_BASE}/paneflow-${LATEST_VERSION}-aarch64-apple-darwin.dmg`;
}
