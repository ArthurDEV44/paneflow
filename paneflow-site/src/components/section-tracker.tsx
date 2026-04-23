"use client";

import { useSectionTracking } from "../lib/use-section-tracking";

// Client shim so Server-Component pages (`app/page.tsx`,
// `app/download/page.tsx`) can mount the section-tracking hook without
// being converted to client components themselves.
export function SectionTracker() {
  useSectionTracking();
  return null;
}
