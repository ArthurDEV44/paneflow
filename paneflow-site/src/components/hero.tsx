"use client";

import Image from "next/image";
import { motion } from "framer-motion";
import { Check, Copy, Download, Mail } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import posthog from "posthog-js";
import { AppleIcon, WindowsIcon } from "./os-icons";
import { GitHubIcon } from "./icons";
import { WaitlistForm } from "./waitlist-form";
import { linuxAppImageUrl, macOSDmgUrl } from "../lib/release";
import { useDetectedLinuxArch } from "../lib/use-detected-arch";
import { track } from "../lib/analytics";

// PostHog data showed Windows users land on the page (20 sessions / 30 d)
// but get zero CTAs they can act on — Linux + macOS buttons only — and
// click out at a 5% rate vs 24-27% for Linux/macOS. The mobile audience
// is 37% of traffic with a 0s median time on page and an 82% sub-5s
// bounce rate, because the same desktop CTAs are served to phones that
// can't run a desktop binary. This component adds:
//   - a Windows "soon" CTA that captures intent
//   - a mobile-only panel that explains the desktop-only constraint and
//     gives the user a way to bring the link back to their desktop
//   - section events tied to both (mobile_unsupported_seen,
//     windows_waitlist_clicked)

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
          Three agents. Three branches. One window.
        </motion.h1>

        {/* Subtitle */}
        <motion.p
          className="mt-4 text-sm sm:text-base text-text-muted leading-relaxed max-w-xl"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.25 }}
        >
          Paneflow is the terminal workspace built around how you actually
          work with Claude Code, Codex, and OpenCode - parallel panes per
          agent, live branch and dev-server status, session restore,
          scriptable from any language. Native Linux and macOS.
        </motion.p>

        {/* CTAs — desktop variant. Hidden on coarse-pointer + narrow
            viewport via Tailwind so we avoid a hydration flash and the
            user does not see desktop download buttons on a phone. */}
        <motion.div
          className="mt-8 hidden md:flex flex-wrap items-center gap-3"
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
          <WindowsSoonButton />
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
                One pane per agent
              </strong>
              : detects Claude Code, Codex, and OpenCode sessions and tags
              each pane with the right one
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">
                Branch-aware workspaces
              </strong>
              : live git branch and dev-server ports per workspace, persisted
              across restarts
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">
                Scriptable from anywhere
              </strong>
              : JSON-RPC socket drives splits, prompts, and reads from any
              language
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">
                Splits that nest
              </strong>
              : horizontal and vertical, up to 32 panes per workspace, no
              tmux gymnastics
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none">-</span>
            <span>
              <strong className="text-text font-semibold">
                Native, not Electron
              </strong>
              : Rust on Zed&apos;s GPU engine, Linux and macOS, Windows in
              progress
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
            alt="Paneflow showing parallel panes running Claude Code and Codex agent sessions side by side, each with its git branch and dev-server status"
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

function WindowsSoonButton() {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on outside click. Pointerdown rather than click so the gesture
  // closes the popover before the next click reaches anything else - the
  // form input keeps focus during in-popover clicks because containerRef
  // covers it.
  useEffect(() => {
    if (!open) return;
    const handler = (e: PointerEvent) => {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", handler);
    return () => window.removeEventListener("pointerdown", handler);
  }, [open]);

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => {
          if (!open) track("windows_waitlist_clicked", { source: "hero" });
          setOpen((v) => !v);
        }}
        className="inline-flex items-center gap-2.5 px-5 py-2.5 border border-surface-border text-text-muted rounded-full hover:bg-surface/40 hover:text-text transition-all duration-200"
        aria-expanded={open}
        aria-controls="windows-waitlist-hero"
      >
        <WindowsIcon className="w-4 h-4" />
        Windows · soon
      </button>
      {open && (
        <motion.div
          id="windows-waitlist-hero"
          role="dialog"
          aria-label="Windows waitlist"
          initial={{ opacity: 0, y: -4, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ duration: 0.15, ease: "easeOut" }}
          className="absolute z-50 mt-2 w-[22rem] max-w-[calc(100vw-2rem)] left-0 sm:left-auto sm:right-0 rounded-xl border border-surface-border bg-bg p-5 shadow-2xl"
        >
          <div className="text-sm font-semibold text-text">
            Windows build in progress.
          </div>
          <p className="mt-1 text-xs text-text-muted">
            Drop your email, we&apos;ll let you know when it ships.
          </p>
          <div className="mt-4">
            <WaitlistForm source="hero" platform="windows" />
          </div>
        </motion.div>
      )}
    </div>
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
