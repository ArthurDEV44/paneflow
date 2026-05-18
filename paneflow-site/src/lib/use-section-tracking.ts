import { useEffect } from "react";
import { onPosthogReady, track } from "./analytics";

// Observes every `[data-track-section]` element on the mounted page and
// fires `section_reached { section_id }` the first time each one crosses
// 50% viewport visibility. Single-shot per element (observer.unobserve
// after the first hit), so the event maps cleanly to a funnel stage
// without noisy repeats when the user scrolls back and forth.
//
// Landing-based tracking (not scroll-%) was chosen in the PRD because it
// survives arbitrary viewport heights and motion-reduced preferences —
// the API reads actual layout intersection, not CSS transforms.
//
// Observer setup is deferred until `posthog:ready` fires. The hero
// section is visible at >= 50% on initial paint, which makes the
// observer callback fire on the next frame — well before PHProvider's
// useEffect has had a chance to call posthog.init() (child effects run
// before parent effects). Without the gate, every section_reached event
// hit track()'s window.posthog guard and got dropped, which explains
// the 0 ingested events across 4 tagged sections over 30 days.
export function useSectionTracking() {
  useEffect(() => {
    // AC #6 unhappy path: legacy browsers without IntersectionObserver.
    // Silent no-op; no JS error, other analytics unaffected.
    if (typeof IntersectionObserver === "undefined") return;

    let observer: IntersectionObserver | null = null;

    const cleanup = onPosthogReady(() => {
      const elements = Array.from(
        document.querySelectorAll<HTMLElement>("[data-track-section]"),
      );
      if (elements.length === 0) return;

      observer = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            if (entry.isIntersecting && entry.intersectionRatio >= 0.5) {
              const slug = entry.target.getAttribute("data-track-section");
              if (slug) {
                track("section_reached", { section_id: slug });
              }
              observer?.unobserve(entry.target);
            }
          }
        },
        { threshold: 0.5 },
      );

      for (const el of elements) observer.observe(el);
    });

    return () => {
      cleanup();
      observer?.disconnect();
    };
  }, []);
}
