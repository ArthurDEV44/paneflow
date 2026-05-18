import { useEffect } from "react";
import posthog from "posthog-js";
import { onPosthogReady } from "./analytics";

const MILESTONES = [25, 50, 75, 100] as const;
type Milestone = (typeof MILESTONES)[number];

// Fires `scroll_depth_reached { milestone }` once per session per
// milestone, and continuously registers `max_scroll_depth_pct` as a
// session-scoped property so it lands on every subsequent event
// (including the autocaptured `$pageleave`). This lets us answer
// "how far did people scroll before bouncing" without a custom
// pageleave handler, and segment the funnel by depth band.
//
// passive: true on the scroll listener — non-negotiable for input
// responsiveness on long pages (chrome.devrel docs).
export function useScrollDepth() {
  useEffect(() => {
    if (typeof window === "undefined") return;

    let maxPct = 0;
    const hit = new Set<Milestone>();
    let rafScheduled = false;

    const computePct = (): number => {
      const doc = document.documentElement;
      const viewport = window.innerHeight;
      const totalScrollable = Math.max(doc.scrollHeight - viewport, 1);
      const scrolled = window.scrollY;
      return Math.min(
        100,
        Math.max(0, Math.round((scrolled / totalScrollable) * 100)),
      );
    };

    const handleScroll = () => {
      if (rafScheduled) return;
      rafScheduled = true;
      window.requestAnimationFrame(() => {
        rafScheduled = false;
        const pct = computePct();
        if (pct > maxPct) {
          maxPct = pct;
          // register_for_session attaches the property to every event
          // captured during the current PostHog session (cookieless
          // sessions are bounded by a 30-min idle window). Re-registering
          // on each update is intentional and idempotent.
          posthog.register_for_session({ max_scroll_depth_pct: maxPct });
        }
        for (const m of MILESTONES) {
          if (pct >= m && !hit.has(m)) {
            hit.add(m);
            posthog.capture("scroll_depth_reached", {
              milestone: m,
              pathname: window.location.pathname,
            });
          }
        }
      });
    };

    const cleanup = onPosthogReady(() => {
      // Emit the starting depth synthetically so single-screen sessions
      // (no scroll at all) still register a max_scroll_depth_pct.
      const initial = computePct();
      maxPct = initial;
      posthog.register_for_session({ max_scroll_depth_pct: initial });
      window.addEventListener("scroll", handleScroll, { passive: true });
      // Run once to catch the case where the document is already at the
      // 100% milestone on load (short pages, anchor-link landings).
      handleScroll();
    });

    return () => {
      cleanup();
      window.removeEventListener("scroll", handleScroll);
    };
  }, []);
}
