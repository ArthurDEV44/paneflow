"use client";

import { useEffect, useState } from "react";
import { GitHubIcon } from "./icons";

export function Navbar() {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 32);
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  return (
    <header
      className={`fixed top-0 left-0 right-0 z-40 transition-[backdrop-filter,border-color,background-color] duration-300 ${
        scrolled
          ? "backdrop-blur-md bg-bg/70 border-b border-surface-border/50"
          : "border-b border-transparent"
      }`}
    >
      <div className="mx-auto max-w-6xl px-6">
        <nav className="flex items-center justify-between h-16">
          <a href="/" className="text-sm font-semibold tracking-tight">
            PaneFlow
          </a>

          <div className="flex items-center gap-6">
            <a
              href="#features"
              className="text-sm text-text-muted hover:text-text transition-colors duration-200 hidden sm:block"
            >
              Features
            </a>
            <a
              href="https://github.com/ArthurDEV44/paneflow#readme"
              className="text-sm text-text-muted hover:text-text transition-colors duration-200 hidden sm:block"
            >
              Docs
            </a>
            <a
              href="https://github.com/ArthurDEV44/paneflow"
              className="flex items-center gap-2 text-sm text-text-muted hover:text-text transition-colors duration-200"
            >
              <GitHubIcon className="w-4 h-4" />
              <span className="hidden sm:inline">GitHub</span>
            </a>
          </div>
        </nav>
      </div>
    </header>
  );
}
