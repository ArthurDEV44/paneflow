"use client";

import { FadeIn } from "./fade-in";
import Link from "next/link";
import { ExternalLink } from "lucide-react";
import { GitHubIcon } from "./icons";
import { track } from "../lib/analytics";

export function Footer() {
  return (
    <footer data-track-section="footer" className="relative overflow-hidden">
      {/* CTA section */}
      <div className="py-20 sm:py-24">
        <FadeIn>
          <div className="max-w-2xl mx-auto px-6">
            <h2 className="text-2xl sm:text-3xl font-semibold tracking-tight leading-[1.15]">
              Built for developers who live in the terminal.
            </h2>
            <p className="mt-4 text-sm sm:text-base text-text-muted leading-relaxed max-w-xl">
              Open source. Written in Rust. Designed to stay out of your way.
            </p>
            <div className="mt-8 flex flex-wrap items-center gap-3">
              <a
                href="https://github.com/ArthurDEV44/paneflow"
                onClick={() =>
                  track("github_link_clicked", {
                    source: "footer",
                    label: "star",
                  })
                }
                className="inline-flex items-center gap-2.5 px-5 py-2.5 bg-accent text-bg font-semibold rounded-full hover:brightness-110 transition-all duration-200"
              >
                <GitHubIcon className="w-4 h-4" />
                Star on GitHub
              </a>
              <a
                href="https://github.com/ArthurDEV44/paneflow#readme"
                onClick={() =>
                  track("github_link_clicked", {
                    source: "footer",
                    label: "docs",
                  })
                }
                className="inline-flex items-center gap-2.5 px-5 py-2.5 border border-surface-border text-text rounded-full hover:bg-surface/60 transition-all duration-200"
              >
                <ExternalLink className="w-4 h-4" />
                Documentation
              </a>
            </div>
          </div>
        </FadeIn>
      </div>

      {/* Bottom bar */}
      <div className="py-6 border-t border-surface-border/50">
        <div className="max-w-5xl mx-auto px-6 flex flex-col sm:flex-row items-center justify-between gap-4 text-sm text-text-subtle">
          <div className="font-mono">Paneflow</div>
          <div className="flex items-center gap-6">
            <Link
              href="/about"
              className="hover:text-text-muted transition-colors"
            >
              About
            </Link>
            <Link
              href="/legal/privacy"
              className="hover:text-text-muted transition-colors"
            >
              Confidentialité
            </Link>
            <span>MIT License</span>
          </div>
        </div>
      </div>
    </footer>
  );
}
