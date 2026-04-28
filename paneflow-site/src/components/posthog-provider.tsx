"use client";

import { useEffect } from "react";
import posthog from "posthog-js";

// EU Cloud is the only endpoint our privacy policy commits to. An absent
// NEXT_PUBLIC_POSTHOG_HOST would otherwise let posthog-js fall back to its
// compiled-in default (US Cloud — app.posthog.com), silently violating
// data-residency. Keeping the constant here — not in `posthog-js` itself —
// makes the intent auditable from the provider file alone.
const POSTHOG_EU_HOST = "https://eu.i.posthog.com";

export function PHProvider({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    if (typeof window !== "undefined" && process.env.NEXT_PUBLIC_POSTHOG_KEY) {
      posthog.init(process.env.NEXT_PUBLIC_POSTHOG_KEY!, {
        api_host: process.env.NEXT_PUBLIC_POSTHOG_HOST ?? POSTHOG_EU_HOST,
        // Requires "Cookieless server hash mode" enabled at the PostHog
        // project level (Project Settings → Web analytics). Without that
        // toggle, posthog-js sends events with distinct_id="$posthog_cookieless"
        // and PostHog accepts them with 200 OK then silently drops them at
        // ingestion (no HMAC secret to resolve the sentinel). Symptom: 0
        // ingested events, no error in console or network tab.
        cookieless_mode: "always",
        capture_pageview: false,
        capture_pageleave: true,
        // Defense-in-depth alongside PostHog project settings: refuse to
        // ever create server-side person profiles. The privacy page's
        // "no IP stored" claim depends on a project toggle today — this
        // client-level opt-out makes the invariant local to the code.
        person_profiles: "never",
        defaults: "2026-01-30",
      });
    }
  }, []);

  return <>{children}</>;
}
