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
      <div className="py-24 sm:py-32">
        <FadeIn>
          <div className="max-w-3xl mx-auto px-6 text-center">
            <h2 className="text-3xl sm:text-5xl font-bold tracking-tight mb-6">
              Built for developers who live
              <br />
              <span className="text-accent">in the terminal.</span>
            </h2>
            <p className="text-text-muted text-lg mb-10 max-w-xl mx-auto">
              Open source. Written in Rust. Designed to stay out of your way.
            </p>
            <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
              <a
                href="https://github.com/ArthurDEV44/paneflow"
                onClick={() =>
                  track("github_link_clicked", { source: "footer", label: "star" })
                }
                className="inline-flex items-center gap-2.5 px-6 py-3 bg-accent text-bg font-semibold rounded-lg hover:brightness-110 transition-all duration-200"
              >
                <GitHubIcon className="w-4 h-4" />
                Star on GitHub
              </a>
              <a
                href="https://github.com/ArthurDEV44/paneflow#readme"
                onClick={() =>
                  track("github_link_clicked", { source: "footer", label: "docs" })
                }
                className="inline-flex items-center gap-2.5 px-6 py-3 border border-surface-border text-text-muted rounded-lg hover:border-surface-border-hover hover:text-text transition-all duration-200"
              >
                <ExternalLink className="w-4 h-4" />
                Documentation
              </a>
            </div>
          </div>
        </FadeIn>
      </div>

      {/* Bottom bar */}
      <div className="py-6">
        <div className="max-w-5xl mx-auto px-6 flex flex-col sm:flex-row items-center justify-between gap-4 text-sm text-text-subtle">
          <div className="font-mono">PaneFlow</div>
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
