"use client";

import Image from "next/image";
import { motion } from "framer-motion";
import { Download } from "lucide-react";
import posthog from "posthog-js";
import { AppleIcon } from "./os-icons";
import { GitHubIcon } from "./icons";
import { linuxAppImageUrl, macOSDmgUrl } from "../lib/release";
import { useDetectedLinuxArch } from "../lib/use-detected-arch";
import { track } from "../lib/analytics";

export function Hero() {
  // Client-side arch sniff. Defaults to x86_64 on SSR + first client
  // paint; swaps to aarch64 post-mount if Client Hints reports ARM.
  // Gives the "Download for Linux" CTA a direct file URL that matches
  // the user's CPU without an interstitial OS-picker page.
  const arch = useDetectedLinuxArch();

  return (
    <section
      data-track-section="hero"
      className="relative overflow-hidden pt-36 sm:pt-44 pb-24 sm:pb-32"
    >

      <div className="hero-glow" />

      <div className="relative z-10 max-w-4xl mx-auto px-6 text-center">
        {/* Eyebrow */}
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.1 }}
        >
          <span className="inline-flex items-center gap-2 px-4 py-1.5 rounded-full border border-surface-border bg-surface/50 text-sm text-text-muted font-mono">
            <span className="w-1.5 h-1.5 rounded-full bg-accent-green" />
            Built with Rust &amp; GPUI
          </span>
        </motion.div>

        {/* Headline */}
        <motion.h1
          className="mt-8 text-5xl sm:text-6xl md:text-7xl lg:text-8xl font-bold tracking-tight leading-[1.05]"
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.6, delay: 0.2 }}
        >
          Your terminal,
          <br />
          <span className="text-accent">multiplied.</span>
        </motion.h1>

        {/* Subtitle */}
        <motion.p
          className="mt-6 text-lg sm:text-xl text-text-muted max-w-2xl mx-auto leading-relaxed"
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.6, delay: 0.35 }}
        >
          GPU-accelerated terminal multiplexer. Split, organize, and
          control. Powered by Zed&apos;s rendering engine.
        </motion.p>

        {/* CTAs */}
        <motion.div
          className="mt-10 flex flex-col sm:flex-row items-center justify-center gap-4"
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.6, delay: 0.5 }}
        >
          {/*
            Direct AppImage download — NOT a navigation to /download.
            The href resolves to a raw file on the GitHub Releases CDN;
            browser dispatches it as a file save, no github.com
            interstitial. AppImage is the right default for Linux
            because it runs on every modern distro with zero setup.
            Users who want .deb / .rpm / .tar.gz / ARM64 go through
            /download via the smaller "All downloads" link below.
          */}
          <a
            href={linuxAppImageUrl(arch)}
            onClick={() => {
              posthog.capture("download_cta_clicked", {
                source: "hero",
                format: "AppImage",
                platform: "linux",
                arch,
              });
            }}
            className="inline-flex items-center gap-2.5 px-6 py-3 bg-accent text-bg font-semibold rounded-lg hover:brightness-110 transition-all duration-200"
          >
            <Download className="w-4 h-4" />
            Download for Linux
          </a>
          {/*
            macOS .dmg, signed with a Developer ID Application
            certificate and Apple-notarized (ticket stapled). Apple
            Silicon only for v0.2.x; Intel Mac (x86_64-apple-darwin)
            is a closed CI target until later. Same primary CTA
            styling as Linux so both supported platforms get equal
            visual weight on the page.
          */}
          <a
            href={macOSDmgUrl()}
            onClick={() => {
              posthog.capture("download_cta_clicked", {
                source: "hero",
                format: "dmg",
                platform: "macos",
                arch: "aarch64",
              });
            }}
            className="inline-flex items-center gap-2.5 px-6 py-3 bg-accent text-bg font-semibold rounded-lg hover:brightness-110 transition-all duration-200"
          >
            <AppleIcon className="w-4 h-4" />
            Download for macOS
          </a>
          <a
            href="https://github.com/ArthurDEV44/paneflow"
            onClick={() => track("github_link_clicked", { source: "hero" })}
            className="inline-flex items-center gap-2.5 px-6 py-3 border border-surface-border text-text-muted rounded-lg hover:border-surface-border-hover hover:text-text transition-all duration-200"
          >
            <GitHubIcon className="w-4 h-4" />
            View on GitHub
          </a>
        </motion.div>

        {/* Screenshot */}
        <motion.div
          className="mt-16 sm:mt-20 relative max-w-5xl mx-auto"
          initial={{ opacity: 0, y: 40 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.8, delay: 0.65 }}
        >
          <div className="relative rounded-xl border border-surface-border overflow-hidden shadow-2xl shadow-black/50">
            <Image
              src="/images/hero.webp"
              alt="PaneFlow terminal multiplexer: split panes, workspaces, and GPU-accelerated rendering"
              width={1920}
              height={1080}
              sizes="(max-width: 768px) 100vw, 1200px"
              priority
              className="w-full h-auto"
            />
          </div>
        </motion.div>
      </div>
    </section>
  );
}
