"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import Image from "next/image";
import { Menu, X } from "lucide-react";
import { GitHubIcon } from "./icons";
import { track } from "../lib/analytics";

const NAV_LINKS = [
  { href: "/download", label: "Download" },
  { href: "/docs", label: "Docs" },
  { href: "/compare", label: "Compare" },
] as const;

export function Navbar() {
  const [mobileOpen, setMobileOpen] = useState(false);

  // Lock body scroll while the mobile menu is open so the page behind
  // the overlay doesn't drift. The cleanup restores the previous value
  // so other scroll-locking surfaces (modals, drawers) stay correct.
  useEffect(() => {
    if (!mobileOpen) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [mobileOpen]);

  // Close on Escape — standard a11y for full-screen modal menus.
  useEffect(() => {
    if (!mobileOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMobileOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mobileOpen]);

  return (
    <header className="fixed top-0 left-0 right-0 z-40 bg-bg">
      {/* Outer container — aligned with the hero (max-w-[1440px] +
          matching px-padding) so the navbar logo, the h1, and the
          screenshot all share the same left edge at 64px from viewport. */}
      <div className="mx-auto max-w-[1440px] px-6 sm:px-10 lg:px-16">
        <nav className="flex items-center justify-between h-16 gap-6">
          {/* Left — brand mark */}
          <Link
            href="/"
            onClick={() => setMobileOpen(false)}
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

          {/* Right (desktop) — primary nav + GitHub icon. Hidden under
              sm; replaced by the burger trigger below at that breakpoint. */}
          <div className="hidden sm:flex items-center gap-7 shrink-0">
            {NAV_LINKS.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                onClick={() => track("nav_link_clicked", { label: link.label.toLowerCase() })}
                className="text-sm text-text-muted hover:text-text transition-colors duration-200"
              >
                {link.label}
              </Link>
            ))}
            <a
              href="https://github.com/ArthurDEV44/paneflow"
              onClick={() => track("github_link_clicked", { source: "navbar" })}
              aria-label="Paneflow on GitHub"
              className="flex items-center text-text-muted hover:text-text transition-colors duration-200"
            >
              <GitHubIcon className="w-4 h-4" />
            </a>
          </div>

          {/* Right (mobile) — burger trigger. sm:hidden so it never
              competes with the desktop cluster. Aria attributes match
              the WAI-ARIA disclosure pattern for menu buttons. */}
          <button
            type="button"
            onClick={() => {
              const next = !mobileOpen;
              if (next) track("mobile_nav_opened", { source: "navbar" });
              setMobileOpen(next);
            }}
            aria-label={mobileOpen ? "Close menu" : "Open menu"}
            aria-expanded={mobileOpen}
            aria-controls="mobile-nav"
            className="sm:hidden flex items-center justify-center w-9 h-9 -mr-2 text-text-muted hover:text-text transition-colors"
          >
            {mobileOpen ? (
              <X className="w-5 h-5" />
            ) : (
              <Menu className="w-5 h-5" />
            )}
          </button>
        </nav>
      </div>

      {/* Mobile menu overlay — sits below the 64px-tall navbar, covers
          the rest of the viewport. Plain solid bg-bg (no blur) so
          content behind disappears. Closes on link click + Escape +
          re-tap of the burger (now an X). */}
      {mobileOpen && (
        <div
          id="mobile-nav"
          role="dialog"
          aria-modal="true"
          aria-label="Site navigation"
          className="sm:hidden fixed inset-x-0 top-16 bottom-0 z-30 bg-bg flex flex-col"
        >
          <nav className="flex flex-col gap-1 px-6 pt-8 pb-6">
            {NAV_LINKS.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                onClick={() => {
                  track("nav_link_clicked", {
                    label: link.label.toLowerCase(),
                    source: "mobile_nav",
                  });
                  setMobileOpen(false);
                }}
                className="text-2xl py-3 text-text"
              >
                {link.label}
              </Link>
            ))}
            <a
              href="https://github.com/ArthurDEV44/paneflow"
              onClick={() => {
                track("github_link_clicked", { source: "mobile_nav" });
                setMobileOpen(false);
              }}
              className="inline-flex items-center gap-3 text-2xl py-3 text-text"
            >
              <GitHubIcon className="w-5 h-5" />
              GitHub
            </a>
          </nav>
        </div>
      )}
    </header>
  );
}
