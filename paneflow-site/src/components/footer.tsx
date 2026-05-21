"use client";

import { FadeIn } from "./fade-in";
import Link from "next/link";
import { PrimaryDownloadCTA } from "./primary-download-cta";
import { ThemeSelector } from "./theme-selector";
import { useDetectedLinuxArch } from "../lib/use-detected-arch";
import { useDetectedOS } from "../lib/use-detected-os";

export function Footer() {
  const os = useDetectedOS();
  const arch = useDetectedLinuxArch();

  return (
    <footer data-track-section="footer" className="relative overflow-hidden">
      {/* CTA section — minimal Cursor-style closer. Centered headline,
          single primary download button (OS-aware), massive vertical
          breathing. Matches cursor.com's "Essayez Cursor dès maintenant"
          minimal closer. GitHub + docs links remain in the bottom bar. */}
      <div className="py-32 sm:py-40">
        <FadeIn>
          <div className="max-w-3xl mx-auto px-6 text-center">
            <h2 className="text-4xl sm:text-5xl md:text-6xl">
              Try Paneflow today.
            </h2>
            <div className="mt-10 flex justify-center">
              <PrimaryDownloadCTA os={os} arch={arch} source="footer" />
            </div>
          </div>
        </FadeIn>
      </div>

      {/* Bottom bar — two-column layout in the same max-w-[1440px] outer
          container as hero / navbar. Left column stacks brand → tagline →
          copyright; right column stacks nav links (top) → theme selector
          (bottom), aligned via justify-between so the nav links sit at
          the same baseline as the brand mark and the theme selector sits
          at the same baseline as the copyright line. */}
      <div className="py-10 sm:py-12 bg-bg-elevated">
        <div className="max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16 flex flex-col sm:flex-row justify-between gap-8 text-sm">
          {/* Left — brand stack */}
          <div className="flex flex-col justify-between gap-4">
            <div className="flex flex-col gap-2">
              <span className="font-mono font-semibold text-text">
                Paneflow
              </span>
              <p className="text-text-muted max-w-xs leading-relaxed">
                The terminal workspace for orchestrating coding agents.
              </p>
            </div>
            <p className="text-text-subtle text-xs">
              © 2026 Strivex. All rights reserved.
            </p>
          </div>

          {/* Right — nav links on top, theme selector on bottom */}
          <div className="flex flex-col items-start sm:items-end justify-between gap-4">
            <div className="flex items-center gap-6 text-text-muted">
              <Link
                href="/about"
                className="hover:text-text transition-colors"
              >
                About
              </Link>
              <Link
                href="/compare"
                className="hover:text-text transition-colors"
              >
                Compare
              </Link>
              <Link
                href="/legal/privacy"
                className="hover:text-text transition-colors"
              >
                Privacy
              </Link>
            </div>
            <ThemeSelector />
          </div>
        </div>
      </div>
    </footer>
  );
}
