"use client";

import { useEffect } from "react";
import { usePathname, useSearchParams } from "next/navigation";
import posthog from "posthog-js";
import { trackDocsPageView } from "@/lib/docs-analytics";

// GDPR data-minimisation (Art. 5(1)(c)): `$pageview` intentionally drops
// the query string — UTM parameters are surfaced as first-class properties
// by posthog-js (via `$initial_referring_domain` + `utm_*`), and any other
// query param (a password-reset token, an invite hash, etc.) would be PII
// by construction. We strip it at the source rather than relying on a
// downstream denylist.
const UTM_KEYS = new Set([
  "utm_source",
  "utm_medium",
  "utm_campaign",
  "utm_term",
  "utm_content",
]);

function sanitizedPageviewProps() {
  const params = new URLSearchParams(window.location.search);
  const utms: Record<string, string> = {};
  for (const [key, value] of params.entries()) {
    if (UTM_KEYS.has(key)) utms[key] = value;
  }
  return {
    $current_url: `${window.location.origin}${window.location.pathname}`,
    ...utms,
  };
}

export function PostHogPageView() {
  const pathname = usePathname();
  const searchParams = useSearchParams();

  useEffect(() => {
    posthog.capture("$pageview", sanitizedPageviewProps());
    // US-017: additional `docs_page_view` enrichment for /docs/** routes.
    // No-op for any non-docs path; helper handles the section derivation
    // and the silent-on-failure guard.
    trackDocsPageView(pathname);
  }, [pathname, searchParams]);

  return null;
}
