"use client";

import { useScrollDepth } from "../lib/use-scroll-depth";
import { useSectionTracking } from "../lib/use-section-tracking";

// Client shim so Server-Component pages (`app/page.tsx`,
// `app/download/page.tsx`, `app/about/page.tsx`) can mount the analytics
// hooks without being converted to client components themselves.
//
// Mounts:
//   - section_reached (one-shot per [data-track-section] crossed)
//   - scroll_depth_reached at 25/50/75/100%
//   - max_scroll_depth_pct session property (lands on $pageleave)
export function SectionTracker() {
  useSectionTracking();
  useScrollDepth();
  return null;
}
