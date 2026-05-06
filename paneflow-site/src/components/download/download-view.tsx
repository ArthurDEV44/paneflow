"use client";

import {
  type ComponentType,
  type ReactElement,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Check,
  ChevronDown,
  Copy,
  Download,
  Package,
  Terminal,
} from "lucide-react";
import posthog from "posthog-js";
import { AppleIcon, LinuxIcon, WindowsIcon } from "../os-icons";
import { LATEST_VERSION } from "../../lib/release";

// LATEST_VERSION is imported from `lib/release` — single source of
// truth shared with the Hero CTA. Bump it there to propagate
// everywhere. Historical versions below stay local to this component
// since they're only surfaced on the download page.

const VERSIONS: VersionEntry[] = [
  {
    version: LATEST_VERSION,
    latest: true,
    releaseNotes: `https://github.com/ArthurDEV44/paneflow/releases/tag/v${LATEST_VERSION}`,
  },
  {
    version: "0.2.11",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.11",
  },
  {
    version: "0.2.10",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.10",
  },
  {
    version: "0.2.9",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.9",
  },
  {
    version: "0.2.8",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.8",
  },
  {
    version: "0.2.7",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.7",
  },
  {
    version: "0.2.6",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.6",
  },
  {
    version: "0.2.5",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.5",
  },
  {
    version: "0.2.4",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.4",
  },
  {
    version: "0.2.3",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.3",
  },
  {
    version: "0.2.2",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.2",
  },
  {
    version: "0.2.1",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.1",
  },
  {
    version: "0.2.0",
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/tag/v0.2.0",
  },
];

interface VersionEntry {
  version: string;
  latest?: boolean;
  releaseNotes: string;
}

export function DownloadView() {
  return (
    <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
      <div className="max-w-5xl mx-auto px-6">
        <div className="max-w-2xl mb-10 sm:mb-12">
          <h1 className="text-2xl sm:text-3xl font-semibold tracking-tight">
            Paneflow est disponible pour Linux et macOS.
          </h1>
          <p className="mt-3 text-sm sm:text-base text-text-muted leading-relaxed">
            Windows arrive très prochainement.
          </p>
        </div>

        {/* Primary "download for your system" card — client-side OS sniff
            picks the best format per platform and renders one big button.
            Users who want a different format scroll to the matrix below. */}
        <div className="mb-12">
          <PrimaryDownloadCard version={LATEST_VERSION} />
        </div>

        <div
          data-track-section="download_matrix"
          className="divide-y divide-surface-border border-y border-surface-border"
        >
          {VERSIONS.map((entry, i) => (
            <VersionRow key={entry.version} entry={entry} defaultOpen={i === 0} />
          ))}
        </div>
      </div>
    </section>
  );
}

// ─── OS + arch detection ──────────────────────────────────────────────────
//
// Runs after mount (SSR has no navigator). Until the detection resolves,
// the primary card renders a neutral fallback so the server-rendered
// HTML matches the first client paint (no hydration flash).
//
// Arch detection uses `navigator.userAgentData.architecture` when the
// Client Hints API is available (Chrome 90+ / Edge 90+ on HTTPS origins).
// Firefox and Safari don't expose it; fallback is x86_64 — ARM64 users
// still get correct packages from the matrix below.

type DetectedOs = "linux" | "macos" | "windows" | "unknown";
type DetectedArch = "x86_64" | "aarch64";

interface DetectedPlatform {
  os: DetectedOs;
  arch: DetectedArch;
}

type UserAgentDataLike = {
  architecture?: string;
  platform?: string;
  getHighEntropyValues?: (hints: string[]) => Promise<Record<string, string>>;
};

function useDetectedPlatform(): DetectedPlatform | null {
  const [platform, setPlatform] = useState<DetectedPlatform | null>(null);

  useEffect(() => {
    let cancelled = false;

    const uaData = (navigator as unknown as { userAgentData?: UserAgentDataLike })
      .userAgentData;
    const ua = navigator.userAgent || "";

    const os: DetectedOs = (() => {
      // Order matters: /Android/ contains "Linux", so Android must be
      // ruled out first to avoid falsely matching Linux for mobile users
      // (irrelevant audience for PaneFlow but correct-is-better-than-
      // lucky on the sniff).
      if (/Android/i.test(ua)) return "unknown";
      if (/Linux/i.test(ua)) return "linux";
      if (/Mac OS X|Macintosh/i.test(ua)) return "macos";
      if (/Windows/i.test(ua)) return "windows";
      return "unknown";
    })();

    // Low-entropy userAgentData.platform is exposed synchronously on
    // supporting browsers. `getHighEntropyValues(["architecture"])`
    // returns "arm" for aarch64 on an async promise — we use it when
    // available to upgrade the default.
    let arch: DetectedArch = "x86_64";
    if (uaData?.architecture === "arm") arch = "aarch64";

    if (uaData?.getHighEntropyValues) {
      uaData
        .getHighEntropyValues(["architecture"])
        .then((values: Record<string, string>) => {
          if (cancelled) return;
          const upgraded: DetectedArch =
            values.architecture === "arm" ? "aarch64" : "x86_64";
          setPlatform({ os, arch: upgraded });
        })
        .catch(() => {
          // Fall back to the sync guess if the async call rejects
          // (permission policy, transient error, etc.).
          if (!cancelled) setPlatform({ os, arch });
        });
    } else {
      // Defer to the next microtask so the setState call does not land
      // synchronously inside the effect body — the Next.js-strict
      // react-hooks/set-state-in-effect rule rejects the sync variant,
      // and calling setState in an effect callback is the documented
      // way to satisfy it (react.dev "You Might Not Need an Effect").
      queueMicrotask(() => {
        if (!cancelled) setPlatform({ os, arch });
      });
    }

    return () => {
      cancelled = true;
    };
  }, []);

  return platform;
}

// For each OS, the "primary format" is the one most users should pick.
// Linux → .AppImage is universal (no root needed, no apt/dnf, just run).
// macOS + Windows → placeholders until signed builds ship in v0.3.0.
type PrimaryDownload =
  | {
      available: true;
      href: string;
      format: string;
      icon: ComponentType<{ className?: string }>;
    }
  | {
      available: false;
      reason: string;
      icon: ComponentType<{ className?: string }>;
    };

function primaryDownload(
  version: string,
  platform: DetectedPlatform,
): PrimaryDownload {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  if (platform.os === "linux") {
    return {
      available: true,
      href: `${base}/paneflow-${version}-${platform.arch}.AppImage`,
      format: `AppImage (${platform.arch})`,
      icon: LinuxIcon,
    };
  }
  if (platform.os === "macos") {
    // Signed Developer ID + Apple-notarized .dmg. Apple Silicon only:
    // the `x86_64-apple-darwin` target is a closed CI matrix entry
    // until Intel-Mac CI is reactivated. Intel Mac users on a
    // 2020-or-earlier laptop still see this card with the aarch64
    // .dmg href, which will fail to launch on their hardware; the
    // matrix below remains the recovery path until the cut.
    return {
      available: true,
      href: `${base}/paneflow-${version}-aarch64-apple-darwin.dmg`,
      format: "DMG (Apple Silicon)",
      icon: AppleIcon,
    };
  }
  if (platform.os === "windows") {
    return {
      available: false,
      reason: "Windows MSI signé arrive très prochainement.",
      icon: WindowsIcon,
    };
  }
  return {
    available: false,
    reason: "OS non détecté. Choisis un format dans la liste ci-dessous.",
    icon: Package,
  };
}

function PrimaryDownloadCard({ version }: { version: string }) {
  const platform = useDetectedPlatform();

  // Server-render a neutral placeholder that matches the first client
  // paint so hydration doesn't flash. Same container dimensions as the
  // ready state.
  if (!platform) {
    return (
      <div className="rounded-lg border border-surface-border p-5 sm:p-6 flex flex-col sm:flex-row sm:items-center gap-4 sm:gap-5">
        <div className="w-9 h-9 rounded-md bg-bg-elevated" />
        <div className="flex-1">
          <div className="h-4 w-40 rounded bg-bg-elevated mb-2" />
          <div className="h-3 w-28 rounded bg-bg-elevated" />
        </div>
      </div>
    );
  }

  const pick = primaryDownload(version, platform);
  const Icon = pick.icon;

  return (
    <div className="rounded-lg border border-surface-border p-5 sm:p-6 flex flex-col sm:flex-row sm:items-center gap-4 sm:gap-5">
      <div className="w-9 h-9 rounded-md bg-bg-elevated flex items-center justify-center shrink-0">
        <Icon className="w-4 h-4 text-text" />
      </div>

      <div className="flex-1 min-w-0">
        <div className="text-xs text-text-subtle font-mono uppercase tracking-wider">
          Téléchargement recommandé
        </div>
        <div className="text-sm sm:text-base font-semibold mt-1">
          {pick.available ? `Paneflow ${version}, ${pick.format}` : pick.reason}
        </div>
        {!pick.available && platform.os !== "unknown" && (
          <div className="text-sm text-text-muted mt-1">
            En attendant, utilise Linux ou choisis un format dans la liste ci-dessous.
          </div>
        )}
      </div>

      {pick.available ? (
        <a
          href={pick.href}
          onClick={() => {
            posthog.capture("download_cta_clicked", {
              source: "primary_card",
              format: pick.format,
              platform: platform.os,
              arch: platform.arch,
              version,
            });
          }}
          className="inline-flex items-center justify-center gap-2 px-5 py-2.5 rounded-full bg-accent text-bg font-semibold hover:brightness-110 transition-all duration-200 shrink-0"
        >
          <Download className="w-4 h-4" />
          Télécharger
        </a>
      ) : null}
    </div>
  );
}

function VersionRow({
  entry,
  defaultOpen,
}: {
  entry: VersionEntry;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div>
      <button
        onClick={() => {
          const next = !open;
          posthog.capture("version_accordion_toggled", {
            version: entry.version,
            action: next ? "open" : "close",
          });
          setOpen(next);
        }}
        className="w-full flex items-center justify-between py-4 text-left"
      >
        <div className="flex items-center gap-3">
          <span className="text-base font-semibold">{entry.version}</span>
          {entry.latest && (
            <span className="px-2 py-0.5 rounded-full border border-surface-border text-[11px] text-text-muted font-mono">
              Latest
            </span>
          )}
        </div>
        <ChevronDown
          className={`w-4 h-4 text-text-muted transition-transform duration-200 ${
            open ? "rotate-180" : ""
          }`}
        />
      </button>

      {open && (
        <div className="pb-6">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-x-8 gap-y-6">
            <PlatformColumn
              Icon={AppleIcon}
              label="macOS"
              items={macOSItems(entry.version)}
              platform="macos"
              version={entry.version}
            />
            <PlatformColumn
              Icon={WindowsIcon}
              label="Windows"
              items={[]}
              placeholder="Arrive très prochainement"
              platform="windows"
              version={entry.version}
            />
            <PlatformColumn
              Icon={LinuxIcon}
              label="Linux"
              items={linuxItems(entry.version)}
              platform="linux"
              version={entry.version}
            />
          </div>

          <a
            href={entry.releaseNotes}
            className="inline-flex mt-6 text-sm text-text-muted hover:text-text transition-colors"
          >
            Voir les notes de version &rarr;
          </a>
        </div>
      )}
    </div>
  );
}

// Items are either a regular download link (`href`) or a copy-to-clipboard
// command (`copyText`) — exactly one is set. Per-item `icon` overrides the
// default download arrow on the right; useful for copy-items that should
// signal "this is a command, not a download".
interface DownloadItem {
  label: string;
  href?: string;
  copyText?: string;
  // Optional per-item architecture for matrix-row analytics — set on
  // Linux items so `download_cta_clicked` can attribute x64 vs ARM64.
  arch?: DetectedArch;
  // ComponentType accepts both lucide-react forward-refs (which return
  // ReactNode) and the plain-function icon components in `../os-icons`.
  icon?: ComponentType<{ className?: string }>;
}

type TrackedPlatform = "linux" | "macos" | "windows";

function PlatformColumn({
  Icon,
  label,
  items,
  placeholder,
  platform,
  version,
}: {
  Icon: (props: { className?: string }) => ReactElement;
  label: string;
  items: DownloadItem[];
  placeholder?: string;
  platform: TrackedPlatform;
  version: string;
}) {
  return (
    <div className="rounded-lg border border-surface-border bg-bg-elevated p-4">
      <div className="flex items-center gap-2 mb-3 px-1">
        <Icon className="w-3.5 h-3.5 text-text-muted" />
        <span className="text-xs font-mono text-text-muted uppercase tracking-wider">
          {label}
        </span>
      </div>
      {items.length === 0 ? (
        <p className="px-1 py-2 text-sm text-text-subtle">
          {placeholder ?? "-"}
        </p>
      ) : (
        <ul>
          {items.map((item) => (
            <li key={item.label}>
              <DownloadRow item={item} platform={platform} version={version} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// Renders one row in a PlatformColumn. Link items become <a>, copy items
// become <button> that writes to the clipboard + swaps the trailing icon
// to a Check for 2s (the "toast confirmation" per US-020 AC-4). Pattern
// matches install.tsx:58 — kept inline here rather than factored out so
// the component file stays self-contained.
function DownloadRow({
  item,
  platform,
  version,
}: {
  item: DownloadItem;
  platform: TrackedPlatform;
  version: string;
}) {
  const [copied, setCopied] = useState(false);
  // timerRef lets us cancel the pending setCopied(false) on rapid
  // re-clicks (so the badge re-resets 2s after the LAST click, not the
  // first) and on unmount (so React 18+ doesn't whine about setting
  // state on an unmounted component).
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    return () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    };
  }, []);

  const LeadingIcon = item.icon;
  const baseClass =
    "flex items-center justify-between w-full px-2 py-2.5 rounded-md text-sm text-text-muted hover:text-text hover:bg-bg-elevated transition-colors text-left";
  // Linux rows carry no per-item icon — keep the pre-US-020 layout (just
  // a label on the left, download arrow on the right). Windows rows
  // carry WindowsIcon / Terminal which render as a leading glyph.
  const Label = (
    <span className="flex items-center gap-2">
      {LeadingIcon && <LeadingIcon className="w-4 h-4 text-text-subtle" />}
      <span>{item.label}</span>
    </span>
  );

  if (item.copyText) {
    const copyText = item.copyText;
    const handleCopy = () => {
      // Fire analytics on click intent, before the clipboard call —
      // captures the event even if the write rejects (Firefox permission
      // quirks, non-secure context fallback). posthog.capture is
      // fire-and-forget and never blocks the UX.
      posthog.capture("install_command_copied", {
        command: copyText,
        platform,
      });
      // navigator.clipboard is only available in secure contexts (https
      // or localhost). On the production site both hold. Flip the UI to
      // "copied" state ONLY after the write resolves — a rejection (e.g.
      // Firefox's occasional permission quirk) leaves the Copy icon
      // visible so the user knows to try again instead of seeing a false
      // success check.
      navigator.clipboard.writeText(copyText).then(
        () => {
          if (timerRef.current !== null) clearTimeout(timerRef.current);
          setCopied(true);
          timerRef.current = setTimeout(() => setCopied(false), 2000);
        },
        () => {
          // Swallow — the button stays in "Copy" state, user retries.
        },
      );
    };
    return (
      <button
        type="button"
        onClick={handleCopy}
        aria-label={`Copier la commande: ${copyText}`}
        className={baseClass}
      >
        {Label}
        {copied ? (
          <Check className="w-4 h-4 text-accent-green" />
        ) : (
          <Copy className="w-4 h-4 text-text-subtle" />
        )}
      </button>
    );
  }

  return (
    <a
      href={item.href}
      onClick={() => {
        posthog.capture("download_cta_clicked", {
          source: "matrix",
          format: item.label,
          platform,
          arch: item.arch,
          version,
        });
      }}
      className={baseClass}
    >
      {Label}
      <Download className="w-4 h-4 text-text-subtle" />
    </a>
  );
}

// macOS items. Apple Silicon (.dmg) only for now. Intel Mac
// (`x86_64-apple-darwin`) is a closed CI target until Intel-Mac CI
// is reactivated, so the matrix has just one row here. Filename uses
// the v0.2.10+ convention (commit f2a0c96):
// `paneflow-<semver>-aarch64-apple-darwin.dmg` (no `v` prefix on the
// version segment), matching `update_checker.rs::pick_asset` (US-008).
// When the Intel Mac leg ships, add a second row with
// `paneflow-<semver>-x86_64-apple-darwin.dmg`, same hyphen pattern.
function macOSItems(version: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  return [
    {
      label: "DMG (Apple Silicon)",
      href: `${base}/paneflow-${version}-aarch64-apple-darwin.dmg`,
    },
  ];
}

// Linux asset filenames switched from `paneflow-v<version>-<arch>.<ext>`
// (used pre-v0.2.10) to `paneflow-<version>-<arch>.<ext>` (no `v` prefix,
// used from v0.2.10+ via commit f2a0c96) to align with the macOS DMG /
// Windows MSI convention. The in-app updater matcher
// (`update_checker.rs::pick_asset`) is suffix-only, so existing pre-rename
// clients still discover post-rename assets correctly. If you need to
// point at a pre-v0.2.10 release, add the `v` back into the asset name.
function linuxItems(version: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  const asset = (name: string) => `${base}/${name}`;
  return [
    {
      label: "AppImage (x64)",
      href: asset(`paneflow-${version}-x86_64.AppImage`),
      arch: "x86_64",
    },
    {
      label: "AppImage (ARM64)",
      href: asset(`paneflow-${version}-aarch64.AppImage`),
      arch: "aarch64",
    },
    {
      label: ".deb (x64)",
      href: asset(`paneflow-${version}-x86_64.deb`),
      arch: "x86_64",
    },
    {
      label: ".deb (ARM64)",
      href: asset(`paneflow-${version}-aarch64.deb`),
      arch: "aarch64",
    },
    {
      label: ".rpm (x64)",
      href: asset(`paneflow-${version}-x86_64.rpm`),
      arch: "x86_64",
    },
    {
      label: ".rpm (ARM64)",
      href: asset(`paneflow-${version}-aarch64.rpm`),
      arch: "aarch64",
    },
    {
      label: "tar.gz (x64)",
      href: asset(`paneflow-${version}-x86_64.tar.gz`),
      arch: "x86_64",
    },
    {
      label: "tar.gz (ARM64)",
      href: asset(`paneflow-${version}-aarch64.tar.gz`),
      arch: "aarch64",
    },
  ];
}

// Unused on the current download page (Windows artifacts arrive with
// v0.3.0 signed builds). Kept as a reference for the future Windows
// release cut — the `windowsItems(entry.version)` call site inside
// `VersionRow` is the hook-point to re-enable; also drop the
// `items={[]}` on the Windows PlatformColumn there. Filename
// convention mirrors cargo-wix output from release.yml's US-016 stage
// step: `paneflow-<ver>-x86_64-pc-windows-msvc.msi` — the `v` prefix
// lives in the tag URL segment, not the filename.
//
// `Terminal` import is used by this function's winget row; if the
// function stays dead-code past v0.3.0's release cut, delete it + the
// Terminal import together.
// eslint-disable-next-line @typescript-eslint/no-unused-vars
function _windowsItems(version: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  return [
    {
      label: "Windows x86_64 MSI",
      href: `${base}/paneflow-${version}-x86_64-pc-windows-msvc.msi`,
      icon: WindowsIcon,
    },
    {
      label: "winget install",
      copyText: "winget install ArthurDev44.PaneFlow",
      icon: Terminal,
    },
  ];
}
