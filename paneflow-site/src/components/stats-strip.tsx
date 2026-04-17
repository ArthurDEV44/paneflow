"use client";

import { FadeIn } from "./fade-in";

const stats = [
  { label: "Pure Rust", sublabel: "Zero runtime overhead" },
  { label: "GPU-accelerated", sublabel: "Zed's GPUI engine" },
  { label: "< 5ms", sublabel: "Keystroke to pixel" },
];

export function StatsStrip() {
  return (
    <section className="py-20">
      <FadeIn>
        <div className="max-w-4xl mx-auto px-6">
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-8 sm:gap-4 text-center">
            {stats.map((stat, i) => (
              <div key={i} className="space-y-1">
                <div className="text-2xl font-bold tracking-tight">
                  {stat.label}
                </div>
                <div className="text-sm text-text-muted">{stat.sublabel}</div>
              </div>
            ))}
          </div>
        </div>
      </FadeIn>
    </section>
  );
}
