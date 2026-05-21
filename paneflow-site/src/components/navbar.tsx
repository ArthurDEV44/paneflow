"use client";

import Link from "next/link";
import Image from "next/image";
import { GitHubIcon } from "./icons";
import { track } from "../lib/analytics";

export function Navbar() {
  return (
    <header className="fixed top-0 left-0 right-0 z-40 bg-bg">
      {/* Outer container — aligned with the hero (max-w-[1440px] +
          matching px-padding) so the navbar logo, the h1, and the
          screenshot all share the same left edge at 64px from viewport.
          Visual rhythm: brand mark left, nav items center (Cursor
          pattern), download CTA + GitHub + theme toggle right. */}
      <div className="mx-auto max-w-[1440px] px-6 sm:px-10 lg:px-16">
        <nav className="flex items-center justify-between h-16 gap-6">
          {/* Left — brand mark */}
          <Link
            href="/"
            className="flex items-center gap-2 text-sm font-semibold tracking-tight shrink-0"
          >
            <Image
              src="/logos/paneflow-web-300.png"
              alt="Paneflow"
              width={28}
              height={28}
              sizes="28px"
              priority
              className="h-7 w-7"
            />
            Paneflow
          </Link>

          {/* Right — primary nav + GitHub icon. All clustered on the
              right side; "Features" was removed (the home page is the
              feature catalog itself) and the GitHub link was reduced to
              its icon to keep the cluster compact. */}
          <div className="hidden sm:flex items-center gap-7 shrink-0">
            <Link
              href="/download"
              onClick={() => track("nav_link_clicked", { label: "download" })}
              className="text-sm text-text-muted hover:text-text transition-colors duration-200"
            >
              Download
            </Link>
            <Link
              href="/docs"
              onClick={() => track("nav_link_clicked", { label: "docs" })}
              className="text-sm text-text-muted hover:text-text transition-colors duration-200"
            >
              Docs
            </Link>
            <Link
              href="/compare"
              onClick={() => track("nav_link_clicked", { label: "compare" })}
              className="text-sm text-text-muted hover:text-text transition-colors duration-200"
            >
              Compare
            </Link>
            <a
              href="https://github.com/ArthurDEV44/paneflow"
              onClick={() => track("github_link_clicked", { source: "navbar" })}
              aria-label="Paneflow on GitHub"
              className="flex items-center text-text-muted hover:text-text transition-colors duration-200"
            >
              <GitHubIcon className="w-4 h-4" />
            </a>
          </div>
        </nav>
      </div>
    </header>
  );
}
