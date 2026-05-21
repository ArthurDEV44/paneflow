"use client";

import Image from "next/image";
import { motion } from "framer-motion";
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
// Mobile visitors see the same pair: PrimaryDownloadCTA detects OS as
// "mobile" and routes to /download with the full matrix + waitlist;
// GitHub link works everywhere.

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

          {/* CTAs — same pair on every viewport. PrimaryDownloadCTA
              detects mobile OS internally and routes those visitors to
              /download (full matrix + Windows waitlist) instead of
              serving a desktop binary they can't run. */}
          <motion.div
            className="mt-10 flex flex-wrap items-center gap-3"
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
