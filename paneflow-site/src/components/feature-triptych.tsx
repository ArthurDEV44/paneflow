"use client";

import { FadeIn } from "./fade-in";

interface Feature {
  title: string;
  description: string;
}

const features: Feature[] = [
  {
    title: "Split anything",
    description:
      "Binary tree splits, up to 32 panes per workspace. Run one agent per pane, drag dividers to give the busy one more room, navigate with the keyboard.",
  },
  {
    title: "20 workspaces, one per branch",
    description:
      "Named workspaces with session restore. Your agent panes, git branch, and working directory survive restarts. Switch with Ctrl+1-9.",
  },
  {
    title: "Native, not Electron",
    description:
      "Built on Zed's GPU rendering engine and an Alacritty fork - the same stack Zed ships. No JIT, no embedded Chromium, no battery drain when your agents run all night.",
  },
];

export function FeatureTriptych() {
  return (
    <section className="py-16 sm:py-20">
      <FadeIn>
        <div className="max-w-2xl mx-auto px-6">
          <h2 className="text-2xl sm:text-3xl font-semibold tracking-tight">
            One pane per agent.
          </h2>
          <p className="mt-3 text-sm sm:text-base text-text-muted leading-relaxed">
            Stop juggling tmux windows to keep your coding agents in view.
            Paneflow gives each session its own pane, tagged with the agent
            and the branch it&apos;s working on.
          </p>
        </div>
      </FadeIn>

      <div className="max-w-5xl mx-auto px-6 mt-10 grid grid-cols-1 sm:grid-cols-3 gap-x-8 gap-y-8">
        {features.map((feature, i) => (
          <FadeIn key={i} delay={i * 0.08}>
            <div className="border-t border-surface-border pt-4">
              <h3 className="text-sm font-semibold text-text">
                {feature.title}
              </h3>
              <p className="mt-2 text-sm text-text-muted leading-relaxed">
                {feature.description}
              </p>
            </div>
          </FadeIn>
        ))}
      </div>
    </section>
  );
}
