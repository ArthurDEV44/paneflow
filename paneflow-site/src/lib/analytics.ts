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
