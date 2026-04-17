"use client";

import { FadeIn } from "./fade-in";
import { Columns3, Layers, Cpu } from "lucide-react";
import type { LucideIcon } from "lucide-react";

interface Feature {
  icon: LucideIcon;
  title: string;
  description: string;
}

const features: Feature[] = [
  {
    icon: Columns3,
    title: "Split anything",
    description:
      "Binary tree split system. Up to 32 panes per workspace with drag-to-resize dividers, preset layouts, and directional focus navigation.",
  },
  {
    icon: Layers,
    title: "20 workspaces",
    description:
      "Named, tabbed workspaces with session persistence. Your layouts and working directories survive restarts. Switch with Ctrl+1\u20139.",
  },
  {
    icon: Cpu,
    title: "Zed\u2019s terminal core",
    description:
      "Same Alacritty fork and GPUI rendering engine as the Zed editor. GPU-accelerated, cell-by-cell paint with APCA contrast enforcement.",
  },
];

export function FeatureTriptych() {
  return (
    <section className="py-24 sm:py-32">
      <div className="max-w-5xl mx-auto px-6">
        <FadeIn>
          <h2 className="text-3xl sm:text-4xl font-bold tracking-tight text-center mb-4">
            Terminal, evolved
          </h2>
          <p className="text-text-muted text-center max-w-2xl mx-auto mb-16">
            Everything you need to manage complex workflows in a single window.
          </p>
        </FadeIn>

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
          {features.map((feature, i) => (
            <FadeIn key={i} delay={i * 0.1}>
              <div className="group relative p-6 rounded-xl border border-surface-border bg-surface/30 h-full">
                <feature.icon className="w-8 h-8 text-accent mb-4 stroke-[1.5]" />
                <h3 className="text-lg font-semibold mb-2">{feature.title}</h3>
                <p className="text-sm text-text-muted leading-relaxed">
                  {feature.description}
                </p>
              </div>
            </FadeIn>
          ))}
        </div>
      </div>
    </section>
  );
}
