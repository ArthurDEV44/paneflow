"use client";

import Link from "next/link";
import { Download } from "lucide-react";
import { useTranslations } from "next-intl";
import posthog from "posthog-js";
import { AppleIcon, WindowsIcon } from "./os-icons";
import { linuxAppImageUrl, macOSDmgUrl } from "../lib/release";
import type { useDetectedLinuxArch } from "../lib/use-detected-arch";
import type { useDetectedOS } from "../lib/use-detected-os";

type Arch = ReturnType<typeof useDetectedLinuxArch>;
type OS = ReturnType<typeof useDetectedOS>;

/**
 * OS-aware primary "Download" pill, used by the hero and the closer CTA.
 *
 *   - Linux  → AppImage direct link, auto-detected arch
 *   - macOS  → .dmg direct link
 *   - Windows → "Windows · Q3 2026" pill linking to /download (waitlist)
 *   - mobile/unknown → generic "Download Paneflow" → /download
 *
 * `source` is passed to PostHog so the same component can be reused
 * across surfaces with distinct attribution. `className` lets callers
 * override the pill style if needed (e.g. a slightly different padding
 * on a smaller card).
 */
export function PrimaryDownloadCTA({
  os,
  arch,
  source,
  className,
}: {
  os: OS;
  arch: Arch;
  source: string;
  className?: string;
}) {
  const t = useTranslations("DownloadCTA");
  const pillCls =
    className ??
    "inline-flex items-center gap-2.5 px-5 py-2.5 bg-accent text-bg font-semibold rounded-full hover:brightness-110 transition-all duration-200";

  if (os === "macos") {
    return (
      <a
        href={macOSDmgUrl()}
        onClick={() => {
          posthog.capture("download_cta_clicked", {
            source,
            format: "dmg",
            platform: "macos",
            arch: "aarch64",
          });
        }}
        className={pillCls}
      >
        <AppleIcon className="w-4 h-4" />
        {t("macos")}
      </a>
    );
  }

  if (os === "windows") {
    return (
      <Link
        href="/download"
        onClick={() => {
          posthog.capture("windows_waitlist_clicked", { source });
        }}
        className={pillCls}
      >
        <WindowsIcon className="w-4 h-4" />
        {t("windowsBadge")}
      </Link>
    );
  }

  if (os === "mobile" || os === "unknown") {
    return (
      <Link
        href="/download"
        onClick={() => {
          posthog.capture("download_cta_clicked", { source, platform: os });
        }}
        className={pillCls}
      >
        <Download className="w-4 h-4" />
        {t("generic")}
      </Link>
    );
  }

  // Linux fallback (default after detection or when uaData says Linux).
  return (
    <a
      href={linuxAppImageUrl(arch)}
      onClick={() => {
        posthog.capture("download_cta_clicked", {
          source,
          format: "AppImage",
          platform: "linux",
          arch,
        });
      }}
      className={pillCls}
    >
      <Download className="w-4 h-4" />
      {t("linux")}
    </a>
  );
}
