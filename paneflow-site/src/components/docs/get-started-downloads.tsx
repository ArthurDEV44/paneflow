"use client";

import { Download } from "lucide-react";
import posthog from "posthog-js";
import { AppleIcon, LinuxIcon } from "../os-icons";
import {
  LATEST_VERSION,
  linuxAppImageUrl,
  macOSDmgUrl,
} from "../../lib/release";

/**
 * Dual-platform download CTA for the docs "Get started" page. Renders
 * the Apple Silicon `.dmg` and the Linux x86_64 AppImage as the two
 * primary download paths. ARM64 Linux and the macOS Gatekeeper notes
 * are documented in the per-OS install guides linked alongside.
 *
 * Tracked through PostHog as `download_cta_clicked` with
 * `source: "docs_get_started"` so the funnel separates docs-led
 * downloads from the landing/download pages.
 */
export function GetStartedDownloads(): React.ReactElement {
  const macHref = macOSDmgUrl();
  const linuxHref = linuxAppImageUrl("x86_64");

  function track(platform: "macos" | "linux", format: string): void {
    if (typeof window === "undefined") return;
    try {
      if (typeof posthog?.capture !== "function") return;
      posthog.capture("download_cta_clicked", {
        source: "docs_get_started",
        format,
        platform,
        arch: platform === "macos" ? "aarch64" : "x86_64",
        version: LATEST_VERSION,
      });
    } catch {
      // Silent: a broken analytics path must never crash docs.
    }
  }

  return (
    <div className="not-prose my-6 grid gap-3 sm:grid-cols-2">
      <a
        href={macHref}
        onClick={() => track("macos", "DMG (Apple Silicon)")}
        className="group flex items-center gap-4 rounded-lg border border-fd-border bg-fd-card p-4 transition-colors hover:bg-fd-accent hover:text-fd-accent-foreground"
      >
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-fd-muted text-fd-foreground group-hover:bg-fd-background">
          <AppleIcon className="h-5 w-5" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-xs font-mono uppercase tracking-wider text-fd-muted-foreground group-hover:text-fd-accent-foreground/80">
            macOS - Apple Silicon
          </div>
          <div className="mt-0.5 text-sm font-semibold">
            Download .dmg ({LATEST_VERSION})
          </div>
        </div>
        <Download className="h-4 w-4 shrink-0 text-fd-muted-foreground group-hover:text-fd-accent-foreground" />
      </a>

      <a
        href={linuxHref}
        onClick={() => track("linux", "AppImage (x86_64)")}
        className="group flex items-center gap-4 rounded-lg border border-fd-border bg-fd-card p-4 transition-colors hover:bg-fd-accent hover:text-fd-accent-foreground"
      >
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-fd-muted text-fd-foreground group-hover:bg-fd-background">
          <LinuxIcon className="h-5 w-5" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-xs font-mono uppercase tracking-wider text-fd-muted-foreground group-hover:text-fd-accent-foreground/80">
            Linux - x86_64
          </div>
          <div className="mt-0.5 text-sm font-semibold">
            Download AppImage ({LATEST_VERSION})
          </div>
        </div>
        <Download className="h-4 w-4 shrink-0 text-fd-muted-foreground group-hover:text-fd-accent-foreground" />
      </a>
    </div>
  );
}
