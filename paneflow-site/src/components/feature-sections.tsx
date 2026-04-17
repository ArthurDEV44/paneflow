"use client";

import Image from "next/image";
import { FadeIn } from "./fade-in";
import {
  PanelTopDashed,
  Terminal,
  Bot,
  Palette,
  ArrowUpDown,
  Search,
  MousePointer2,
  Link2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

interface FeatureDetail {
  icon: LucideIcon;
  label: string;
}

interface FeatureSection {
  badge: string;
  title: string;
  description: string;
  details: FeatureDetail[];
}

const sections: FeatureSection[] = [
  {
    badge: "Layouts",
    title: "Splits that just work",
    description:
      "A binary tree layout engine gives you arbitrary nesting. Split horizontal, vertical, zoom to fullscreen, or pick a preset \u2014 the layout adapts to your workflow, not the other way around.",
    details: [
      { icon: PanelTopDashed, label: "4 preset layouts" },
      { icon: ArrowUpDown, label: "Drag-to-resize dividers" },
      { icon: Search, label: "In-buffer regex search" },
      { icon: Terminal, label: "Undo close with Ctrl+Shift+T" },
    ],
  },
  {
    badge: "Context-aware",
    title: "Dev server detection",
    description:
      "PaneFlow detects running HTTP servers in each workspace automatically. Frontend and backend ports are labeled and displayed in the sidebar \u2014 no configuration, no guessing which port is which.",
    details: [
      { icon: Search, label: "Auto-detect HTTP ports" },
      { icon: Terminal, label: "Frontend / backend labels" },
      { icon: Link2, label: "Per-workspace service list" },
      { icon: MousePointer2, label: "Live git branch & diff stats" },
    ],
  },
  {
    badge: "Programmable",
    title: "AI-ready",
    description:
      "Built-in IPC server exposes a JSON-RPC 2.0 API over Unix sockets. Claude Code and Codex sessions are detected automatically. Script your terminal from any language.",
    details: [
      { icon: Bot, label: "Claude Code & Codex detection" },
      { icon: Terminal, label: "JSON-RPC 2.0 via Unix socket" },
      { icon: PanelTopDashed, label: "Programmatic splits & text send" },
      { icon: Search, label: "Workspace management API" },
    ],
  },
  {
    badge: "Polish",
    title: "Everything you expect",
    description:
      "24-bit color, IME input, drag-and-drop, regex URL detection, copy mode, and 6 hand-tuned themes with hot-reload. APCA perceptual contrast ensures readability across all themes.",
    details: [
      { icon: Palette, label: "6 themes, hot-reload" },
      { icon: Terminal, label: "24-bit ANSI color" },
      { icon: MousePointer2, label: "Drag-and-drop files" },
      { icon: Link2, label: "Regex URL auto-detection" },
    ],
  },
];

export function FeatureSections() {
  return (
    <section className="py-24 sm:py-32">
      <div className="max-w-5xl mx-auto px-6 space-y-32">
        {sections.map((section, i) => (
          <FadeIn key={i}>
            <div
              className={`flex flex-col ${
                i % 2 === 1 ? "md:flex-row-reverse" : "md:flex-row"
              } gap-12 md:gap-16 items-center`}
            >
              {/* Text */}
              <div className="flex-1 space-y-6">
                <span className="inline-block px-3 py-1 rounded-full text-xs font-mono text-accent border border-accent/20 bg-accent-dim">
                  {section.badge}
                </span>
                <h3 className="text-3xl sm:text-4xl font-bold tracking-tight">
                  {section.title}
                </h3>
                <p className="text-text-muted leading-relaxed">
                  {section.description}
                </p>
                <div className="grid grid-cols-2 gap-3 pt-2">
                  {section.details.map((detail, j) => (
                    <div
                      key={j}
                      className="flex items-center gap-2.5 text-sm text-text-muted"
                    >
                      <detail.icon className="w-4 h-4 text-text-subtle shrink-0" />
                      {detail.label}
                    </div>
                  ))}
                </div>
              </div>

              {/* Visual — abstract terminal representation */}
              <div className="flex-1 w-full">
                <FeatureVisual index={i} />
              </div>
            </div>
          </FadeIn>
        ))}
      </div>
    </section>
  );
}

function FeatureVisual({ index }: { index: number }) {
  if (index === 0) {
    // Splits visual — real screenshot
    return (
      <div className="rounded-xl border border-surface-border overflow-hidden">
        <Image
          src="/images/layouts.png"
          alt="PaneFlow split layout with multiple panes and workspaces"
          width={1920}
          height={1080}
          className="w-full h-auto"
        />
      </div>
    );
  }

  if (index === 1) {
    // Context-aware — real screenshot
    return (
      <div className="rounded-xl border border-surface-border overflow-hidden">
        <Image
          src="/images/context-aware.png"
          alt="PaneFlow sidebar showing detected dev servers and git branch per workspace"
          width={1920}
          height={1080}
          className="w-full h-auto"
        />
      </div>
    );
  }

  if (index === 2) {
    // AI-ready — real screenshot
    return (
      <div className="rounded-xl border border-surface-border overflow-hidden">
        <Image
          src="/images/ai-ready.png"
          alt="PaneFlow with Claude Code session detected and AI agent running"
          width={1920}
          height={1080}
          className="w-full h-auto"
        />
      </div>
    );
  }

  // Appearance — real screenshot
  return (
    <div className="rounded-xl border border-surface-border overflow-hidden">
      <Image
        src="/images/appearance.png"
        alt="PaneFlow settings window with theme and font customization"
        width={1920}
        height={1080}
        className="w-full h-auto"
      />
    </div>
  );
}
