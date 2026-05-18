"use client";

import { FadeIn } from "./fade-in";

const stats = [
  "Detects Claude Code, Codex & OpenCode sessions",
  "Linux and macOS native, not Electron",
  "JSON-RPC socket - script it from any language",
];

export function StatsStrip() {
  return (
    <section data-track-section="stats" className="py-10 sm:py-12">
      <FadeIn>
        <div className="max-w-3xl mx-auto px-6">
          <div className="flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-[13px] font-mono text-text-muted">
            {stats.map((stat, i) => (
              <span key={i} className="flex items-center gap-x-5">
                {i > 0 && (
                  <span aria-hidden className="text-text-subtle">
                    &middot;
                  </span>
                )}
                {stat}
              </span>
            ))}
          </div>
        </div>
      </FadeIn>
    </section>
  );
}
