"use client";

import Image from "next/image";
import { FadeIn } from "./fade-in";

interface FeatureSection {
  badge: string;
  title: string;
  description: string;
  details: string[];
}

const sections: FeatureSection[] = [
  {
    badge: "Layouts",
    title: "Splits that just work",
    description:
      "A binary tree layout engine gives you arbitrary nesting. Split horizontal, vertical, zoom to fullscreen, or pick a preset. The layout adapts to your workflow, not the other way around.",
    details: [
      "4 preset layouts",
      "Drag-to-resize dividers",
      "In-buffer regex search",
      "Undo close with Ctrl+Shift+T",
    ],
  },
  {
    badge: "Context-aware",
    title: "Dev server detection",
    description:
      "Paneflow detects running HTTP servers in each workspace automatically. Frontend and backend ports are labeled and displayed in the sidebar. No configuration, no guessing which port is which.",
    details: [
      "Auto-detect HTTP ports",
      "Frontend / backend labels",
      "Per-workspace service list",
      "Live git branch & diff stats",
    ],
  },
  {
    badge: "Programmable",
    title: "AI-ready",
    description:
      "Built-in IPC server exposes a JSON-RPC 2.0 API over Unix sockets. Claude Code and Codex sessions are detected automatically. Script your terminal from any language.",
    details: [
      "Claude Code & Codex detection",
      "JSON-RPC 2.0 via Unix socket",
      "Programmatic splits & text send",
      "Workspace management API",
    ],
  },
  {
    badge: "Polish",
    title: "Everything you expect",
    description:
      "24-bit color, IME input, drag-and-drop, regex URL detection, copy mode, and 6 hand-tuned themes with hot-reload. APCA perceptual contrast ensures readability across all themes.",
    details: [
      "6 themes, hot-reload",
      "24-bit ANSI color",
      "Drag-and-drop files",
      "Regex URL auto-detection",
    ],
  },
];

export function FeatureSections() {
  return (
    <section className="py-16 sm:py-20">
      <div className="max-w-5xl mx-auto px-6 space-y-20 sm:space-y-24">
        {sections.map((section, i) => (
          <FadeIn key={i}>
            <div
              className={`flex flex-col ${
                i % 2 === 1 ? "md:flex-row-reverse" : "md:flex-row"
              } gap-10 md:gap-14 items-center`}
            >
              {/* Text */}
              <div className="flex-1 space-y-4">
                <span className="inline-block text-xs font-mono text-text-muted uppercase tracking-wider">
                  — {section.badge}
                </span>
                <h3 className="text-2xl sm:text-3xl font-semibold tracking-tight">
                  {section.title}
                </h3>
                <p className="text-sm sm:text-base text-text-muted leading-relaxed">
                  {section.description}
                </p>
                <ul className="pt-2 space-y-2 text-sm text-text-muted">
                  {section.details.map((detail, j) => (
                    <li key={j} className="flex gap-3">
                      <span className="text-text-muted/60 select-none">-</span>
                      <span>{detail}</span>
                    </li>
                  ))}
                </ul>
              </div>

              {/* Visual */}
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
  const visuals = [
    {
      src: "/images/layouts.webp",
      alt: "Paneflow split layout with multiple panes and workspaces",
    },
    {
      src: "/images/context-aware.webp",
      alt: "Paneflow sidebar showing detected dev servers and git branch per workspace",
    },
    {
      src: "/images/ai-ready.webp",
      alt: "Paneflow with Claude Code session detected and AI agent running",
    },
    {
      src: "/images/appearance.webp",
      alt: "Paneflow settings window with theme and font customization",
    },
  ];
  const visual = visuals[index] ?? visuals[visuals.length - 1];

  return (
    <div className="rounded-lg border border-surface-border overflow-hidden">
      <Image
        src={visual.src}
        alt={visual.alt}
        width={1920}
        height={1080}
        sizes="(max-width: 768px) 100vw, (max-width: 1024px) 50vw, 600px"
        className="w-full h-auto"
      />
    </div>
  );
}
