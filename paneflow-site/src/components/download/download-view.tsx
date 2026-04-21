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
    <section className="pt-36 sm:pt-40 pb-24">
      <div className="max-w-5xl mx-auto px-6">
        <div className="mb-10">
          <h1 className="text-2xl sm:text-3xl font-semibold tracking-tight">
            PaneFlow est disponible pour Linux.
          </h1>
          <p className="mt-2 text-text-muted">
            macOS et Windows arrivent très prochainement.
          </p>
        </div>

        {/* Primary "download for your system" card — client-side OS sniff
            picks the best format per platform and renders one big button.
            Users who want a different format scroll to the matrix below. */}
        <div className="mb-10">
          <PrimaryDownloadCard version={LATEST_VERSION} />
        </div>

        <div className="divide-y divide-surface-border border-y border-surface-border">
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
      href: `${base}/paneflow-v${version}-${platform.arch}.AppImage`,
      format: `AppImage (${platform.arch})`,
      icon: LinuxIcon,
    };
  }
  if (platform.os === "macos") {
    return {
      available: false,
      reason: "macOS signé + notarisé arrive très prochainement.",
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
      <div className="rounded-2xl border border-surface-border bg-surface/40 p-6 sm:p-8 flex flex-col sm:flex-row sm:items-center gap-4 sm:gap-6">
        <div className="w-12 h-12 rounded-xl bg-bg-elevated" />
        <div className="flex-1">
          <div className="h-5 w-48 rounded bg-bg-elevated mb-2" />
          <div className="h-4 w-32 rounded bg-bg-elevated" />
        </div>
      </div>
    );
  }

  const pick = primaryDownload(version, platform);
  const Icon = pick.icon;

  return (
    <div className="rounded-2xl border border-surface-border bg-surface/40 p-6 sm:p-8 flex flex-col sm:flex-row sm:items-center gap-4 sm:gap-6">
      <div className="w-12 h-12 rounded-xl bg-bg-elevated flex items-center justify-center shrink-0">
        <Icon className="w-6 h-6 text-text" />
      </div>

      <div className="flex-1 min-w-0">
        <div className="text-sm text-text-subtle font-mono uppercase tracking-wider">
          Téléchargement recommandé
        </div>
        <div className="text-lg sm:text-xl font-medium mt-0.5">
          {pick.available ? `PaneFlow ${version}, ${pick.format}` : pick.reason}
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
          className="inline-flex items-center justify-center gap-2 px-5 py-3 rounded-lg bg-accent text-bg font-medium hover:bg-accent-warm transition-colors shrink-0"
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
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center justify-between py-5 text-left"
      >
        <div className="flex items-center gap-3">
          <span className="text-lg font-medium">{entry.version}</span>
          {entry.latest && (
            <span className="px-2 py-0.5 rounded-full border border-surface-border text-xs text-text-muted font-mono">
              Latest
            </span>
          )}
        </div>
        <ChevronDown
          className={`w-5 h-5 text-text-muted transition-transform duration-200 ${
            open ? "rotate-180" : ""
          }`}
        />
      </button>

      {open && (
        <div className="pb-8">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
            <PlatformColumn
              Icon={AppleIcon}
              label="macOS"
              items={[]}
              placeholder="Arrive très prochainement"
            />
            <PlatformColumn
              Icon={WindowsIcon}
              label="Windows"
              items={[]}
              placeholder="Arrive très prochainement"
            />
            <PlatformColumn
              Icon={LinuxIcon}
              label="Linux"
              items={linuxItems(entry.version)}
            />
          </div>

          <a
            href={entry.releaseNotes}
            className="inline-flex mt-6 text-sm text-accent-warm hover:text-accent transition-colors"
          >
            Voir les notes de version →
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
  // ComponentType accepts both lucide-react forward-refs (which return
  // ReactNode) and the plain-function icon components in `../os-icons`.
  icon?: ComponentType<{ className?: string }>;
}

function PlatformColumn({
  Icon,
  label,
  items,
  placeholder,
}: {
  Icon: (props: { className?: string }) => ReactElement;
  label: string;
  items: DownloadItem[];
  placeholder?: string;
}) {
  return (
    <div className="rounded-xl border border-surface-border bg-surface/30 p-4">
      <div className="flex items-center gap-2 mb-3 px-2">
        <Icon className="w-4 h-4 text-text-muted" />
        <span className="text-sm font-medium">{label}</span>
      </div>
      {items.length === 0 ? (
        <p className="px-2 py-3 text-sm text-text-subtle">
          {placeholder ?? "-"}
        </p>
      ) : (
        <ul>
          {items.map((item) => (
            <li key={item.label}>
              <DownloadRow item={item} />
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
function DownloadRow({ item }: { item: DownloadItem }) {
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
    <a href={item.href} className={baseClass}>
      {Label}
      <Download className="w-4 h-4 text-text-subtle" />
    </a>
  );
}

function linuxItems(version: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  const asset = (name: string) => `${base}/${name}`;
  return [
    {
      label: "AppImage (x64)",
      href: asset(`paneflow-v${version}-x86_64.AppImage`),
    },
    {
      label: "AppImage (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.AppImage`),
    },
    {
      label: ".deb (x64)",
      href: asset(`paneflow-v${version}-x86_64.deb`),
    },
    {
      label: ".deb (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.deb`),
    },
    {
      label: ".rpm (x64)",
      href: asset(`paneflow-v${version}-x86_64.rpm`),
    },
    {
      label: ".rpm (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.rpm`),
    },
    {
      label: "tar.gz (x64)",
      href: asset(`paneflow-v${version}-x86_64.tar.gz`),
    },
    {
      label: "tar.gz (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.tar.gz`),
    },
  ];
}

// Unused on the v0.2.1 download page (Windows artifacts arrive in
// v0.3.0). Kept as a reference for the future Windows release cut — the
// `windowsItems(entry.version)` call site inside `VersionRow` is the
// hook-point to re-enable; also drop the `items={[]}` on the Windows
// PlatformColumn there. Filename convention mirrors cargo-wix output
// from release.yml's US-016 stage step:
// `paneflow-<ver>-x86_64-pc-windows-msvc.msi` — the `v` prefix lives in
// the tag URL segment, not the filename.
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
