"use client";

import { useState } from "react";
import { ChevronDown, Download } from "lucide-react";
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
            PaneFlow est disponible pour Linux.
          </h1>
          <p className="mt-2 text-text-muted">
            macOS et Windows arrivent prochainement.
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
              items={[]}
              placeholder="Bientôt disponible"
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

interface DownloadItem {
  label: string;
  href: string;
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
              <a
                href={item.href}
                className="flex items-center justify-between px-2 py-2.5 rounded-md text-sm text-text-muted hover:text-text hover:bg-bg-elevated transition-colors"
              >
                <span>{item.label}</span>
                <Download className="w-4 h-4 text-text-subtle" />
              </a>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
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
