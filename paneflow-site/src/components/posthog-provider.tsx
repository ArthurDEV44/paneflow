"use client";

import { useEffect } from "react";
import posthog from "posthog-js";

// EU Cloud is the only endpoint our privacy policy commits to. An absent
// NEXT_PUBLIC_POSTHOG_HOST would otherwise let posthog-js fall back to its
// compiled-in default (US Cloud — app.posthog.com), silently violating
// data-residency. Keeping the constant here — not in `posthog-js` itself —
// makes the intent auditable from the provider file alone.
const POSTHOG_EU_HOST = "https://eu.i.posthog.com";

function hasPosthog(): boolean {
  return (
    typeof window !== "undefined" &&
    Boolean((window as unknown as { posthog?: unknown }).posthog)
  );
}

export function PHProvider({
  children,
  locale,
}: {
  children: React.ReactNode;
  // When supplied, becomes a PostHog super property so every captured
  // event (incl. $pageview, windows_waitlist_*, download_cta_clicked,
  // language_switched, docs_search_query, etc.) automatically carries
  // the active locale. The only PHProvider in the tree lives in
  // [locale]/layout.tsx, which forwards its `locale` param — so docs
  // routes (now under [locale]/docs/) inherit the URL locale verbatim,
  // including the EN root (locale="en") and the 5 prefixed locales.
  // Compatible with cookieless_mode: 'always' - super properties live
  // in posthog-js memory for the session, no cookie write required.
  // First wired by prd-i18n-fr-zh-Hans US-018; docs coverage closed by
  // prd-fumadocs-docs-i18n US-004.
  locale?: string;
}) {
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
        loaded: () => {
          // Unblocks onPosthogReady() consumers (use-section-tracking,
          // use-scroll-milestones, use-max-scroll-depth). Without this,
          // child-component effects fire their first observer callbacks
          // BEFORE init resolves and track() drops them silently.
          window.dispatchEvent(new CustomEvent("posthog:ready"));
        },
      });
    }
  }, []);

  // US-018: keep the `locale` super property in sync with the active
  // route. Two timing cases need to be handled separately:
  //   1. First mount before init resolves -> register on `posthog:ready`.
  //   2. Subsequent locale changes (client-side nav from the language
  //      switcher) -> register immediately because posthog-js is already
  //      loaded.
  // posthog-js queues captures until init completes, so the first
  // $pageview (fired by PostHogPageView with the same `pathname` dep)
  // is also covered as long as register() lands in the same tick window.
  useEffect(() => {
    if (typeof window === "undefined" || !locale) return;
    const register = () => {
      if (hasPosthog()) {
        posthog.register({ locale });
      }
    };
    if (hasPosthog()) {
      register();
      return;
    }
    const handler = () => register();
    window.addEventListener("posthog:ready", handler, { once: true });
    return () => window.removeEventListener("posthog:ready", handler);
  }, [locale]);

  return <>{children}</>;
}
