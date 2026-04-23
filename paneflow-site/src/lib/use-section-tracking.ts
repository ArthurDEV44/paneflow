import { useEffect } from "react";
import { track } from "./analytics";

// Observes every `[data-track-section]` element on the mounted page and
// fires `section_reached { section_id }` the first time each one crosses
// 50% viewport visibility. Single-shot per element (observer.unobserve
// after the first hit), so the event maps cleanly to a funnel stage
// without noisy repeats when the user scrolls back and forth.
//
// Landing-based tracking (not scroll-%) was chosen in the PRD because it
// survives arbitrary viewport heights and motion-reduced preferences —
// the API reads actual layout intersection, not CSS transforms.
export function useSectionTracking() {
  useEffect(() => {
    // AC #6 unhappy path: legacy browsers without IntersectionObserver.
    // Silent no-op; no JS error, other analytics unaffected.
    if (typeof IntersectionObserver === "undefined") return;

    const elements = Array.from(
      document.querySelectorAll<HTMLElement>("[data-track-section]"),
    );
    if (elements.length === 0) return;

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting && entry.intersectionRatio >= 0.5) {
            const slug = entry.target.getAttribute("data-track-section");
            if (slug) {
              track("section_reached", { section_id: slug });
            }
            observer.unobserve(entry.target);
          }
        }
      },
      { threshold: 0.5 },
    );

    for (const el of elements) observer.observe(el);

    return () => observer.disconnect();
  }, []);
}
