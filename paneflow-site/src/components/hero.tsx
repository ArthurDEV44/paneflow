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
  const arch = useDetectedLinuxArch();

  return (
    <section
      data-track-section="hero"
      className="relative pt-32 sm:pt-40 pb-16 sm:pb-20"
    >
      <div className="relative z-10 max-w-2xl mx-auto px-6">
        {/* Brand row */}
        <motion.div
          className="flex items-center gap-3"
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4, delay: 0.05 }}
        >
          <Image
            src="/logos/paneflow-web-300.png"
            alt=""
            width={40}
            height={40}
            priority
            className="rounded-lg"
          />
          <span className="text-xl font-semibold tracking-tight">
            Paneflow
          </span>
        </motion.div>

        {/* Headline */}
        <motion.h1
          className="mt-10 text-2xl sm:text-3xl font-semibold tracking-tight leading-[1.15]"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.15 }}
        >
          Your terminal, multiplied.
        </motion.h1>

        {/* Subtitle */}
        <motion.p
          className="mt-4 text-sm sm:text-base text-text-muted leading-relaxed max-w-xl"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.25 }}
        >
          GPU-accelerated terminal multiplexer built in Rust on Zed&apos;s
          GPUI engine. Split panes, vertical workspaces, AI-aware
          notifications, and a socket API for automation.
        </motion.p>

        {/* CTAs */}
        <motion.div
          className="mt-8 flex flex-wrap items-center gap-3"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.35 }}
        >
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
            className="inline-flex items-center gap-2.5 px-5 py-2.5 bg-accent text-bg font-semibold rounded-full hover:brightness-110 transition-all duration-200"
          >
            <Download className="w-4 h-4" />
            Download for Linux
          </a>
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
            className="inline-flex items-center gap-2.5 px-5 py-2.5 bg-accent text-bg font-semibold rounded-full hover:brightness-110 transition-all duration-200"
          >
            <AppleIcon className="w-4 h-4" />
            Download for macOS
          </a>
          <a
            href="https://github.com/ArthurDEV44/paneflow"
            onClick={() => track("github_link_clicked", { source: "hero" })}
            className="inline-flex items-center gap-2.5 px-5 py-2.5 border border-surface-border text-text rounded-full hover:bg-surface/60 transition-all duration-200"
          >
            <GitHubIcon className="w-4 h-4" />
            View on GitHub
          </a>
        </motion.div>

        {/* Feature list */}
        <motion.ul
          className="mt-12 space-y-3 text-sm sm:text-[15px] text-text-muted"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.55 }}
        >
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">
                GPU-accelerated
              </strong>
              : rendered on Zed&apos;s GPUI engine, no Electron, no JIT
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">Split panes</strong>
              : horizontal and vertical splits in any workspace
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">Workspaces</strong>
              : keyboard switching, persisted across sessions
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">AI-aware</strong>
              : panes surface agent activity and notifications
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">Scriptable</strong>
              : JSON-RPC socket API for automation
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">Native</strong>:
              pure Rust, Linux and macOS, Windows in progress
            </span>
          </li>
        </motion.ul>
      </div>

      {/* Screenshot */}
      <motion.div
        className="mt-10 sm:mt-14 w-full px-4 sm:px-8"
        initial={{ opacity: 0, y: 24 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.7, delay: 0.7 }}
      >
        <div className="max-w-[1440px] mx-auto">
          <Image
            src="/images/paneflow-hero.png"
            alt="Paneflow terminal multiplexer: split panes, workspaces, and GPU-accelerated rendering"
            width={2491}
            height={1361}
            sizes="(max-width: 768px) 100vw, 1440px"
            priority
            className="w-full h-auto rounded-lg"
          />
        </div>
      </motion.div>
    </section>
  );
}
