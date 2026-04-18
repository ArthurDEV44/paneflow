"use client";

import { type ComponentType, useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Copy, Download, Terminal } from "lucide-react";
import { AppleIcon, LinuxIcon, WindowsIcon } from "../os-icons";
import { MeshHeader } from "./mesh-header";

const VERSIONS: VersionEntry[] = [
  {
    version: "0.1.7",
    latest: true,
    releaseNotes: "https://github.com/ArthurDEV44/paneflow/releases/latest",
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
          <MeshHeader />
        </div>

        <div className="mb-12">
          <h1 className="text-2xl sm:text-3xl font-semibold tracking-tight">
            PaneFlow est disponible pour Linux et Windows.
          </h1>
          <p className="mt-2 text-text-muted">
            macOS arrive prochainement.
          </p>
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
              placeholder="Bientôt disponible"
            />
            <PlatformColumn
              Icon={WindowsIcon}
              label="Windows"
              items={windowsItems(entry.version)}
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
  Icon: (props: { className?: string }) => React.ReactElement;
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
          {placeholder ?? "—"}
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

// US-020. Two rows: (1) direct MSI link, (2) `winget install` copy-command.
// The MSI filename matches US-016's Stage step output
// (paneflow-<ver>-x86_64-pc-windows-msvc.msi — the `v` prefix is in the
// release tag URL segment, NOT the filename, mirroring macOS conventions
// and unlike the Linux `paneflow-v<ver>-...` naming).
function windowsItems(version: string): DownloadItem[] {
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

function linuxItems(version: string): DownloadItem[] {
  const base = `https://github.com/ArthurDEV44/paneflow/releases/download/v${version}`;
  const asset = (name: string) => `${base}/${name}`;
  return [
    {
      label: "Linux .deb (x64)",
      href: asset(`paneflow-v${version}-x86_64.deb`),
    },
    {
      label: "Linux .deb (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.deb`),
    },
    {
      label: "Linux RPM (x64)",
      href: asset(`paneflow-v${version}-x86_64.rpm`),
    },
    {
      label: "Linux RPM (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.rpm`),
    },
    {
      label: "Linux AppImage (x64)",
      href: asset(`paneflow-v${version}-x86_64.AppImage`),
    },
    {
      label: "Linux AppImage (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.AppImage`),
    },
    {
      label: "Linux tar.gz (x64)",
      href: asset(`paneflow-v${version}-x86_64.tar.gz`),
    },
    {
      label: "Linux tar.gz (ARM64)",
      href: asset(`paneflow-v${version}-aarch64.tar.gz`),
    },
  ];
}
