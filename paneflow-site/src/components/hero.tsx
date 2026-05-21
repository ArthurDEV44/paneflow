"use client";

import Image from "next/image";
import { motion } from "framer-motion";
import { Check, Copy, Mail } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { GitHubIcon } from "./icons";
import { PrimaryDownloadCTA } from "./primary-download-cta";
import { useDetectedLinuxArch } from "../lib/use-detected-arch";
import { useDetectedOS } from "../lib/use-detected-os";
import { track } from "../lib/analytics";

// CTA model:
//   - Primary: <PrimaryDownloadCTA> — OS-aware download pill that swaps
//     between Linux .AppImage / macOS .dmg / Windows · Q3 2026 / mobile-
//     fallback /download link based on useDetectedOS()
//   - Secondary: GitHub pill
// On mobile (md:hidden), both primary pills are replaced by the
// MobileDesktopOnlyPanel which fires mobile_unsupported_seen once per
// mount and offers copy-link + mailto so the visitor can bring the
// link back to a desktop instead of bouncing cold.

export function Hero() {
  const arch = useDetectedLinuxArch();
  const os = useDetectedOS();

  return (
    <section
      data-track-section="hero"
      className="relative pt-32 sm:pt-40 pb-0"
    >
      {/* Outer container — centered + generous padding. Inner content is
          left-aligned (no mx-auto on inner) to match Cursor's hero layout:
          editorial weight settles to the left, screenshot floats below
          full-bleed. Brand row (logo + "Paneflow") is intentionally absent —
          the navbar already carries the brand mark, repeating it here is
          the kind of redundancy Cursor avoids. */}
      {/* Outer container — matches Cursor's hero pattern: a single
          ~1300px max-width wrapper that mx-auto centers in the viewport,
          with generous left/right padding. Cursor uses max-w-[1300px];
          we use Tailwind's max-w-7xl (1280px), close enough that the
          visual rhythm is the same. Inside this container, text and
          screenshot share the same left edge but have *different* widths:
          the text wraps inside a narrower column (editorial cadence),
          while the screenshot fills the full available container width
          for visual impact. This is the actual Cursor structure — not
          "text and image at identical width", which we tried earlier
          and felt cramped. */}
      <div className="relative z-10 max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16">
        {/* Text block — editorial column, intentionally wider than
            Cursor's max-w-prose to give Hanken Grotesk's broader glyphs
            room to land the 44px h1 on 2 lines without feeling cramped. */}
        <div className="max-w-6xl">
          {/* Headline — editorial display style. Headline absorbs what was
              previously a separate subtitle: the agent names live inside
              the h1 so the viewer gets the full pitch in one beat, the way
              Cursor's hero h1 does ("Construit pour vous rendre…"). */}
          <motion.h1
            className="text-3xl sm:text-4xl md:text-[44px] md:leading-[1.08]"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, delay: 0.15 }}
          >
            A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and custom CLI agents.
          </motion.h1>

          {/* CTAs — desktop variant. Hidden on coarse-pointer + narrow
              viewport via Tailwind so we avoid a hydration flash and the
              user does not see desktop download buttons on a phone. Just
              two pills: the OS-aware Download primary + GitHub secondary.
              The Download CTA adapts via useDetectedOS — Linux shows
              AppImage, macOS shows .dmg, Windows shows the Q3 2026
              waitlist link, mobile/unknown route to /download. */}
          <motion.div
            className="mt-10 hidden md:flex flex-wrap items-center gap-3"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, delay: 0.35 }}
          >
            <PrimaryDownloadCTA os={os} arch={arch} source="hero" />
            <a
              href="https://github.com/ArthurDEV44/paneflow"
              onClick={() => track("github_link_clicked", { source: "hero" })}
              className="inline-flex items-center gap-2.5 px-5 py-2.5 border border-surface-border text-text rounded-full hover:bg-surface/60 transition-all duration-200"
            >
              <GitHubIcon className="w-4 h-4" />
              View on GitHub
            </a>
          </motion.div>

          {/* CTAs — mobile variant. Replaces download buttons with a
              "desktop-only" explainer + copy/email affordances so phone
              visitors can bring the link back to a real machine instead
              of bouncing. */}
          <div className="mt-8 md:hidden">
            <MobileDesktopOnlyPanel />
          </div>
        </div>

        {/* Screenshot — pulled OUT of the narrow text wrapper. Sits
            directly under the outer max-w-7xl container so it spans the
            full available width (~1152px on desktop after lg:px-16
            padding). Same left edge as the text above by virtue of
            sharing the same outer container. This is exactly how Cursor
            stages their hero demo block. */}
        <motion.div
          className="mt-12 sm:mt-16"
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.7, delay: 0.7 }}
        >
          <Image
            src="/images/paneflow-hero.png"
            alt="Paneflow showing parallel panes running Claude Code and Codex agent sessions side by side, each with its git branch and dev-server status"
            width={2491}
            height={1361}
            sizes="(max-width: 768px) 100vw, (max-width: 1408px) 90vw, 1152px"
            priority
            className="w-full h-auto rounded-lg"
          />
        </motion.div>
      </div>
    </section>
  );
}

// Mobile fallback. Fires `mobile_unsupported_seen` exactly once per
// mount (the matchMedia gate prevents a desktop refresh into mobile from
// triggering a no-op event). Provides Copy-link + mailto so the visitor
// has a way to follow up on a desktop instead of leaving cold.
function MobileDesktopOnlyPanel() {
  const [copied, setCopied] = useState(false);
  const seenRef = useRef(false);

  useEffect(() => {
    if (typeof window === "undefined" || seenRef.current) return;
    // matchMedia mirrors the Tailwind `md:hidden` breakpoint (Tailwind v4
    // defaults md to 768px). Firing only when the mobile variant is the
    // one actually displayed avoids polluting analytics on desktops
    // resized below the breakpoint by devtools.
    if (window.matchMedia("(max-width: 767px)").matches) {
      seenRef.current = true;
      track("mobile_unsupported_seen", { source: "hero" });
    }
  }, []);

  const link = "https://paneflow.dev";
  const subject = "Paneflow - the terminal workspace for coding agents";
  const body =
    "Paneflow runs Claude Code, Codex, and OpenCode in parallel panes on Linux, macOS, and (soon) Windows. Download it here: https://paneflow.dev";
  const mailtoHref = `mailto:?subject=${encodeURIComponent(
    subject,
  )}&body=${encodeURIComponent(body)}`;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
      track("mobile_link_copied", { source: "hero" });
    } catch {
      // Older iOS / permission-denied paths: surface no error, leave the
      // mailto button as the working fallback.
    }
  };

  return (
    <div className="rounded-xl border border-surface-border bg-surface/40 p-5">
      <div className="text-sm font-semibold text-text">
        Paneflow is a desktop app.
      </div>
      <p className="mt-1.5 text-sm text-text-muted leading-relaxed">
        Open this page on a Mac, Linux or Windows machine to download.
      </p>
      <div className="mt-4 flex flex-wrap items-center gap-2.5">
        <button
          type="button"
          onClick={copy}
          className="inline-flex items-center gap-2 px-4 py-2 bg-accent text-bg text-sm font-semibold rounded-full hover:brightness-110 transition-all duration-200"
        >
          {copied ? (
            <>
              <Check className="w-3.5 h-3.5" />
              Link copied
            </>
          ) : (
            <>
              <Copy className="w-3.5 h-3.5" />
              Copy link
            </>
          )}
        </button>
        <a
          href={mailtoHref}
          onClick={() => track("mobile_email_reminder_clicked", { source: "hero" })}
          className="inline-flex items-center gap-2 px-4 py-2 border border-surface-border text-text text-sm rounded-full hover:bg-surface/60 transition-all duration-200"
        >
          <Mail className="w-3.5 h-3.5" />
          Email reminder
        </a>
        <a
          href="https://github.com/ArthurDEV44/paneflow"
          onClick={() =>
            track("github_link_clicked", { source: "hero_mobile" })
          }
          className="inline-flex items-center gap-2 px-4 py-2 border border-surface-border text-text-muted text-sm rounded-full hover:bg-surface/60 hover:text-text transition-all duration-200"
        >
          <GitHubIcon className="w-3.5 h-3.5" />
          GitHub
        </a>
      </div>
    </div>
  );
}
