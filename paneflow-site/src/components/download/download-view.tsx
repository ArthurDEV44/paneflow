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
  Terminal,
} from "lucide-react";
import { useTranslations } from "next-intl";
import posthog from "posthog-js";
import { AgentInstall } from "../docs/agent-install";
import { AppleIcon, LinuxIcon, WindowsIcon } from "../os-icons";
import { PrimaryDownloadCTA } from "../primary-download-cta";
import { WaitlistForm } from "../waitlist-form";
import { LATEST_VERSION } from "../../lib/release";
import { useDetectedLinuxArch } from "../../lib/use-detected-arch";
import { useDetectedOS } from "../../lib/use-detected-os";

// LATEST_VERSION is imported from `lib/release` — single source of
// truth shared with the Hero CTA. Bump it there to propagate
// everywhere. Previous releases are linked to GitHub instead of mirrored
// here so the page stays focused on the current build.

const VERSIONS: VersionEntry[] = [
  {
    version: LATEST_VERSION,
    latest: true,
    releaseNotes: `https://github.com/ArthurDEV44/paneflow/releases/tag/v${LATEST_VERSION}`,
  },
];

interface VersionEntry {
  version: string;
  latest?: boolean;
  releaseNotes: string;
}

export function DownloadView() {
  const t = useTranslations("Download");
  // OS + arch detection lives at the page-level so the primary CTA pill
  // and the per-version matrix below stay in sync — both use the same
  // hook outputs to render the right defaults.
  const os = useDetectedOS();
  const arch = useDetectedLinuxArch();

  return (
    <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
      {/* Outer container aligned with the rest of the site
          (hero / navbar / cards / footer) so the page's left edge sits
          at 64px from viewport on lg+. */}
      <div className="max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16">
        {/* Header — Cursor pattern: small h1 + tagline + primary CTA.
            The pitch lives in a second descriptive paragraph below the
            button so the visitor's eye lands on the CTA first. */}
        <div
          data-track-section="download_header"
          className="max-w-2xl mb-10 sm:mb-12"
        >
          <h1 className="text-3xl sm:text-4xl md:text-5xl">
            {t("header.title")}
          </h1>
          <p className="mt-3 text-base sm:text-lg text-text-muted leading-relaxed">
            {t("header.subhead")}
          </p>
          <div className="mt-8">
            <PrimaryDownloadCTA
              os={os}
              arch={arch}
              source="download_page_primary"
            />
          </div>
        </div>

        <div className="max-w-2xl mb-12 sm:mb-16">
          <p className="text-base sm:text-lg leading-relaxed">
            {t("header.pitch")}
          </p>
        </div>

        {/* Release downloads matrix — collapsible version rows. The
            current release is open by default, older ones link to
            GitHub via the "View Previous Releases" link. */}
        <div data-track-section="download_matrix" className="space-y-4">
          <div className="flex flex-col sm:flex-row sm:items-end sm:justify-between gap-2 mb-2">
            <h2 className="text-xl sm:text-2xl">{t("matrix.heading")}</h2>
            <a
              href="https://github.com/ArthurDEV44/paneflow/releases"
              className="text-sm text-text-muted hover:text-text transition-colors"
            >
              {t("matrix.viewPrevious")}
            </a>
          </div>
          <div>
          {VERSIONS.map((entry, i) => (
            <VersionRow key={entry.version} entry={entry} defaultOpen={i === 0} />
          ))}
          </div>
        </div>

        <div data-track-section="download_agent_install" className="mt-16 sm:mt-20">
          <div className="mb-5">
            <h2 className="text-xl sm:text-2xl">
              {t("agentInstall.heading")}
            </h2>
            <p className="mt-2 text-sm sm:text-base text-text-muted leading-relaxed max-w-2xl">
              {t("agentInstall.subhead")}
            </p>
          </div>
          <AgentInstall />
        </div>
      </div>
    </section>
  );
}

// ─── Per-row arch tag for analytics ────────────────────────────────────
//
// The OS + Linux arch detection used by the primary CTA now lives in the
// shared `useDetectedOS` / `useDetectedLinuxArch` hooks (consumed at the
// top of DownloadView). The matrix below doesn't need detection — it
// lists every available binary regardless of the visitor's OS.

type DetectedArch = "x86_64" | "aarch64";

function VersionRow({
  entry,
  defaultOpen,
}: {
  entry: VersionEntry;
  defaultOpen: boolean;
}) {
  const t = useTranslations("Download.matrix");
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border-b border-surface-border last:border-b-0">
      <button
        onClick={() => {
          const next = !open;
          posthog.capture("version_accordion_toggled", {
            version: entry.version,
            action: next ? "open" : "close",
          });
          setOpen(next);
        }}
        className="w-full flex items-center justify-between py-5 text-left"
      >
        <div className="flex items-center gap-3">
          <span className="text-2xl sm:text-3xl">{entry.version}</span>
          {entry.latest && (
            <span className="px-2.5 py-0.5 rounded-full border border-surface-border text-xs text-text-muted">
              {t("latestBadge")}
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
          {/* 3-column platform grid — gap-2 (8px) is Cursor's exact
              measurement (gap-g1 → 10px). On mobile the columns stack. */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-2">
            <PlatformColumn
              Icon={AppleIcon}
              label={t("platforms.macos")}
              items={macOSItems(entry.version, t("items.dmgAppleSilicon"))}
              platform="macos"
              version={entry.version}
            />
            <PlatformColumn
              Icon={WindowsIcon}
              label={t("platforms.windows")}
              items={[]}
              placeholder={t("placeholders.windows")}
              waitlist={{ source: "download_matrix", version: entry.version }}
              platform="windows"
              version={entry.version}
            />
            <PlatformColumn
              Icon={LinuxIcon}
              label={t("platforms.linux")}
              items={linuxItems(entry.version, {
                appImageX64: t("items.appImageX64"),
                appImageArm64: t("items.appImageArm64"),
                debX64: t("items.debX64"),
                debArm64: t("items.debArm64"),
                rpmX64: t("items.rpmX64"),
                rpmArm64: t("items.rpmArm64"),
                tarGzX64: t("items.tarGzX64"),
                tarGzArm64: t("items.tarGzArm64"),
              })}
              platform="linux"
              version={entry.version}
            />
          </div>

          <a
            href={entry.releaseNotes}
            className="inline-flex mt-5 text-sm text-orange-700 dark:text-orange-300 hover:opacity-80 transition-opacity"
          >
            {t("viewReleaseNotes")}
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
  waitlist,
  platform,
  version,
}: {
  Icon: (props: { className?: string }) => ReactElement;
  label: string;
  items: DownloadItem[];
  placeholder?: string;
  // When set on an empty-items column, renders an inline waitlist form
  // that POSTs to /api/waitlist instead of static text. Used by the
  // Windows column to convert the "Q3 2026" placeholder into an
  // actionable signal - PostHog showed 20 Windows desktop sessions /
  // 30 d hitting this page with 0 actionable CTA.
  waitlist?: {
    source: "download_matrix" | "download_primary";
    version: string;
  };
  platform: TrackedPlatform;
  version: string;
}) {
  const t = useTranslations("Download.matrix.placeholders");
  return (
    <div className="rounded-md bg-bg-elevated p-4 sm:p-5">
      {/* Platform header — bold weight 700 matches Cursor's matrix card
          headings (.type-base.type-md). Icon + label inline. */}
      <div className="flex items-center gap-2 mb-3">
        <Icon className="w-4 h-4 text-text" />
        <h3 className="text-base font-bold text-text">
          {label}
        </h3>
      </div>
      {items.length === 0 ? (
        waitlist ? (
          <div className="space-y-3">
            <p className="text-sm text-text-subtle">
              {placeholder ?? t("comingSoon")}
            </p>
            <WaitlistForm source={waitlist.source} platform="windows" />
          </div>
        ) : (
          <p className="text-sm text-text-subtle">
            {placeholder ?? t("empty")}
          </p>
        )
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
  const t = useTranslations("Download.matrix.aria");
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
  // Row styling — Cursor measures padding 13px 0px on each download
  // anchor, no horizontal padding (the card's p-5 owns left/right space)
  // and no hover bg. Just a text color shift + icon to signal action.
  const baseClass =
    "flex items-center justify-between w-full py-3 text-sm text-text hover:text-text-muted transition-colors text-left";
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
        aria-label={t("copyCommand", { command: copyText })}
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
function macOSItems(version: string, dmgLabel: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  return [
    {
      label: dmgLabel,
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
function linuxItems(
  version: string,
  labels: {
    appImageX64: string;
    appImageArm64: string;
    debX64: string;
    debArm64: string;
    rpmX64: string;
    rpmArm64: string;
    tarGzX64: string;
    tarGzArm64: string;
  },
): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  const asset = (name: string) => `${base}/${name}`;
  return [
    {
      label: labels.appImageX64,
      href: asset(`paneflow-${version}-x86_64.AppImage`),
      arch: "x86_64",
    },
    {
      label: labels.appImageArm64,
      href: asset(`paneflow-${version}-aarch64.AppImage`),
      arch: "aarch64",
    },
    {
      label: labels.debX64,
      href: asset(`paneflow-${version}-x86_64.deb`),
      arch: "x86_64",
    },
    {
      label: labels.debArm64,
      href: asset(`paneflow-${version}-aarch64.deb`),
      arch: "aarch64",
    },
    {
      label: labels.rpmX64,
      href: asset(`paneflow-${version}-x86_64.rpm`),
      arch: "x86_64",
    },
    {
      label: labels.rpmArm64,
      href: asset(`paneflow-${version}-aarch64.rpm`),
      arch: "aarch64",
    },
    {
      label: labels.tarGzX64,
      href: asset(`paneflow-${version}-x86_64.tar.gz`),
      arch: "x86_64",
    },
    {
      label: labels.tarGzArm64,
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
function _windowsItems(
  version: string,
  labels: { msi: string; wingetInstall: string },
): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  return [
    {
      label: labels.msi,
      href: `${base}/paneflow-${version}-x86_64-pc-windows-msvc.msi`,
      icon: WindowsIcon,
    },
    {
      label: labels.wingetInstall,
      copyText: "winget install ArthurDev44.PaneFlow",
      icon: Terminal,
    },
  ];
}
