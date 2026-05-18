import posthog from "posthog-js";

// Guarded capture helper for US-005 instrumentation sites (navbar, footer,
// hero "View on GitHub", install copy button).
//
// AC #6 of US-005 requires that every tracked handler stays a no-op when
// posthog is not initialized — ad-blocker blocked the script, env vars
// missing in a preview branch, init() rejected, etc. posthog-js assigns
// itself to `window.posthog` on init, so the global's presence is the
// authoritative "ready" signal (more accurate than the module import,
// which exists even when init never ran).
export function track(event: string, properties?: Record<string, unknown>) {
  if (
    typeof window !== "undefined" &&
    (window as unknown as { posthog?: unknown }).posthog
  ) {
    posthog.capture(event, properties);
  }
}

// Listener registry for `posthog:ready` — see PHProvider for the dispatch.
// Hooks that set up IntersectionObserver / scroll listeners must wait for
// posthog to be initialized; otherwise events fired between mount and init
// hit `track()`'s window.posthog guard and are dropped silently. This was
// the root cause behind zero `section_reached` events in 30 days of data
// despite four sections being tagged with data-track-section.
//
// React effect order is bottom-up (child first, then parent), so
// SectionTracker.useEffect runs BEFORE PHProvider.useEffect when both are
// mounted in the same render pass. The CustomEvent bridges that gap
// without coupling hooks to the provider's internal state.
export function onPosthogReady(callback: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  if ((window as unknown as { posthog?: unknown }).posthog) {
    callback();
    return () => {};
  }
  const handler = () => callback();
  window.addEventListener("posthog:ready", handler, { once: true });
  return () => window.removeEventListener("posthog:ready", handler);
}
