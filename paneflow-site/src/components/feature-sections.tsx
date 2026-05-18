"use client";

import Image from "next/image";
import { FadeIn } from "./fade-in";

interface FeatureSection {
  badge: string;
  title: string;
  description: string;
}

const sections: FeatureSection[] = [
  {
    badge: "Layouts",
    title: "Terminal layouts for parallel agent work",
    description:
      "Give each agent, test runner, server, or review pane the space it needs. Split horizontal, vertical, zoom to fullscreen, or pick a preset.",
  },
  {
    badge: "Context-aware",
    title: "Know what each agent is touching",
    description:
      "Paneflow keeps branches, diff stats, working directories, and running HTTP servers attached to the workspace. You can see which agent owns which task.",
  },
  {
    badge: "Agent-aware",
    title: "Built to orchestrate CLI coding agents",
    description:
      "Paneflow detects Claude Code, Codex CLI, and OpenCode sessions, tags each pane, and keeps branch context in view. Use JSON-RPC to script splits, send prompts, and read agent output from any language.",
  },
  {
    badge: "Sessions",
    title: "Pick up any agent thread",
    description:
      "Paneflow reads each agent's native session history for the current project, groups Claude Code, Codex, and OpenCode in one popover, and resumes the selected thread in the terminal.",
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
      src: "/images/session-agents.png",
      alt: "Paneflow AI agent sessions popover showing Claude Code, Codex, and OpenCode history for the current project",
      width: 764,
      height: 588,
      priority: true,
    },
  ];
  const visual = visuals[index] ?? visuals[visuals.length - 1];

  return (
    <div className="rounded-lg border border-surface-border overflow-hidden">
      <Image
        src={visual.src}
        alt={visual.alt}
        width={visual.width ?? 1920}
        height={visual.height ?? 1080}
        sizes="(max-width: 768px) 100vw, (max-width: 1024px) 50vw, 600px"
        priority={visual.priority ?? false}
        className="w-full h-auto"
      />
    </div>
  );
}
