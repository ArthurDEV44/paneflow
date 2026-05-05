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
      "Binary tree split system. Up to 32 panes per workspace with drag-to-resize dividers, preset layouts, and directional focus navigation.",
  },
  {
    title: "20 workspaces",
    description:
      "Named, tabbed workspaces with session persistence. Your layouts and working directories survive restarts. Switch with Ctrl+1–9.",
  },
  {
    title: "Zed’s terminal core",
    description:
      "Same Alacritty fork and GPUI rendering engine as the Zed editor. GPU-accelerated, cell-by-cell paint with APCA contrast enforcement.",
  },
];

export function FeatureTriptych() {
  return (
    <section className="py-16 sm:py-20">
      <FadeIn>
        <div className="max-w-2xl mx-auto px-6">
          <h2 className="text-2xl sm:text-3xl font-semibold tracking-tight">
            Terminal, evolved
          </h2>
          <p className="mt-3 text-sm sm:text-base text-text-muted leading-relaxed">
            Everything you need to manage complex workflows in a single
            window.
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
