"use client";

import { FadeIn } from "./fade-in";

interface Feature {
  title: string;
  description: string;
}

const features: Feature[] = [
  {
    title: "One pane per agent",
    description:
      "Run Claude Code, Codex, OpenCode, and custom CLIs side by side. Resize panes, keep reviews close, and navigate from the keyboard.",
  },
  {
    title: "One workspace per task",
    description:
      "Keep agent panes, git branch, working directory, and local services tied to the same task. Restore everything after a restart.",
  },
  {
    title: "Native, long-running by design",
    description:
      "Built on Zed's GPU rendering engine and an Alacritty fork. No embedded Chromium around agents that run for hours.",
  },
];

export function FeatureTriptych() {
  return (
    <section className="pt-12 sm:pt-16 pb-0">
      {/* Outer container aligned with hero / navbar / feature cards so
          the heading and the card grid share the same left edge at 64px
          from the viewport. */}
      <div className="max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16">
        {/* Header — left-aligned heading + tagline, Cursor pattern.
            Narrow max-w on the wrapper keeps the editorial column feel
            from the rest of the page. */}
        <FadeIn>
          <div className="max-w-3xl">
            <h2 className="text-3xl sm:text-4xl md:text-5xl">
              Supervise the work, not the tabs.
            </h2>
            <p className="mt-5 text-base sm:text-lg text-text-muted leading-relaxed max-w-2xl">
              Paneflow turns terminal sessions into an agent control room: every
              session stays visible, labeled, and recoverable while you decide
              what needs attention.
            </p>
          </div>
        </FadeIn>

        {/* Card grid — 3 columns on md+, stacked on mobile. Each card
            uses the same elevated-bg / rounded-md / p-[18px] language as
            the FeatureSections cards below so the visual system stays
            consistent across the page. No images and no inline CTA links
            per design: the cards stand on their copy alone. */}
        <div className="mt-12 sm:mt-16 grid grid-cols-1 md:grid-cols-3 gap-4 sm:gap-6">
          {features.map((feature, i) => (
            <FadeIn key={i} delay={i * 0.08}>
              <article className="h-full rounded-md bg-bg-elevated p-[18px]">
                <h3 className="text-xl sm:text-2xl">{feature.title}</h3>
                <p className="mt-3 text-base text-text-muted leading-relaxed">
                  {feature.description}
                </p>
              </article>
            </FadeIn>
          ))}
        </div>
      </div>
    </section>
  );
}
