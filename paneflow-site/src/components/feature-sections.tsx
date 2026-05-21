"use client";

import Image from "next/image";
import { FadeIn } from "./fade-in";

interface FeatureSection {
  title: string;
  description: string;
}

const sections: FeatureSection[] = [
  {
    title: "Terminal layouts for parallel agent work",
    description:
      "Give each agent, test runner, server, or review pane the space it needs. Split horizontal, vertical, zoom to fullscreen, or pick a preset.",
  },
  {
    title: "Know what each agent is touching",
    description:
      "Paneflow keeps branches, diff stats, working directories, and running HTTP servers attached to the workspace. You can see which agent owns which task.",
  },
  {
    title: "Built to orchestrate CLI coding agents",
    description:
      "Paneflow detects Claude Code, Codex CLI, and OpenCode sessions, tags each pane, and keeps branch context in view. Use JSON-RPC to script splits, send prompts, and read agent output from any language.",
  },
  {
    title: "Pick up any agent thread",
    description:
      "Paneflow reads each agent's native session history for the current project, groups Claude Code, Codex, and OpenCode in one popover, and resumes the selected thread in the terminal.",
  },
];

export function FeatureSections() {
  return (
    <section className="pt-12 sm:pt-16 pb-16 sm:pb-20" id="features">
      {/* Outer container aligned with hero & navbar so the cards' left edge
          sits at the same 64px-from-viewport line as the h1 above. */}
      <div className="max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16 space-y-12 sm:space-y-16">
        {sections.map((section, i) => (
          <FadeIn key={i}>
            {/* Cursor-style card: warm-dark elevated bg, subtle radius,
                tight inner padding. Measured on cursor.com: their
                .card--feature ships padding:17.5px, border-radius:4px,
                bg:rgb(27,25,19), and the column split is 1fr:2fr —
                text takes 33%, image takes 67% of the row. We flip
                that ratio per parity so even cards have text left /
                image right (1fr 2fr) and odd cards have image left /
                text right (2fr 1fr). */}
            <article className="relative overflow-hidden rounded-md bg-bg-elevated">
              <div
                className={`grid gap-6 lg:gap-8 items-center p-[18px] ${
                  i % 2 === 0
                    ? "lg:grid-cols-[1fr_2fr]"
                    : "lg:grid-cols-[2fr_1fr]"
                }`}
              >
                {/* Text column. Narrow max-w gives the editorial column
                    feel from cursor.com. On odd cards (i=1,3) it moves
                    to the right via lg:order-2 + sits in the 1fr (narrow)
                    track on the right. */}
                <div
                  className={`space-y-5 max-w-md ${
                    i % 2 === 1 ? "lg:order-2 lg:justify-self-end" : ""
                  }`}
                >
                  <h3 className="text-3xl sm:text-4xl">
                    {section.title}
                  </h3>
                  <p className="text-base sm:text-lg text-text-muted leading-relaxed">
                    {section.description}
                  </p>
                </div>

                {/* Visual column. Takes the 2fr (wide) track regardless
                    of side; on odd cards lg:order-1 places it on the
                    left while the 2fr track is also on the left. */}
                <div className={i % 2 === 1 ? "lg:order-1" : ""}>
                  <FeatureVisual index={i} />
                </div>
              </div>
            </article>
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
    <div className="rounded border border-surface-border overflow-hidden">
      <Image
        src={visual.src}
        alt={visual.alt}
        width={visual.width ?? 1920}
        height={visual.height ?? 1080}
        sizes="(max-width: 768px) 100vw, (max-width: 1280px) 67vw, 850px"
        priority={visual.priority ?? false}
        className="w-full h-auto"
      />
    </div>
  );
}
