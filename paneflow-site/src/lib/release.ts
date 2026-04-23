// Single source of truth for the current release metadata referenced
// across the marketing site (Hero CTA, Download page primary card, and
// any future callsite). Bump LATEST_VERSION on every release cut;
// download URLs derive from it so a single edit propagates everywhere.
//
// Historical versions on the download page are maintained in
// `components/download/download-view.tsx` (VERSIONS array) — this
// module only tracks "latest".

export const LATEST_VERSION = "0.2.6";

export type LinuxArch = "x86_64" | "aarch64";

const RELEASE_BASE = `https://github.com/ArthurDEV44/paneflow/releases/download/v${LATEST_VERSION}`;

/**
 * Direct-download URL for the recommended Linux binary. AppImage is
 * universal (no root, no dep resolution, runs on every modern distro)
 * so it is the default "big green button" target across the site.
 *
 * Callers should only need arch — the URL is on the GitHub Releases
 * CDN (not a redirect page), so `<a href>` triggers an immediate
 * browser download.
 */
export function linuxAppImageUrl(arch: LinuxArch): string {
  return `${RELEASE_BASE}/paneflow-v${LATEST_VERSION}-${arch}.AppImage`;
}
