"use client";

import posthog from "posthog-js";

/**
 * PostHog event helpers for the docs surface.
 *
 * IMPORTANT: requires the "Cookieless server hash mode" toggle to be ON
 * at the PostHog project level (Project Settings -> Web analytics) - the
 * client is initialised with `cookieless_mode: "always"` and
 * `person_profiles: "never"` in `posthog-provider.tsx`. Without that
 * toggle, PostHog accepts the events with HTTP 200 then silently drops
 * them at ingestion (no HMAC secret to resolve the cookieless sentinel).
 * See [[project_posthog_cookieless]] for the full trap and resolution.
 *
 * All helpers are no-ops when PostHog has not loaded (extension block,
 * network failure, blocked by the user). They never throw, never log to
 * console - failures are swallowed so a broken analytics path can never
 * crash the docs UI.
 *
 * PII policy: only the slug and the user-typed query string are sent.
 * Nothing identifies the visitor; no IP, no profile, no cookie.
 */

function capture(event: string, properties: Record<string, unknown>): void {
  if (typeof window === "undefined") return;
  try {
    if (typeof posthog?.capture !== "function") return;
    posthog.capture(event, properties);
  } catch {
    // Silent: a broken analytics path must never crash docs.
  }
}

/**
 * Derive the docs section from a pathname. `/docs` -> "overview",
 * `/docs/installation/linux` -> "installation", `/docs/keybindings`
 * -> "keybindings". A non-`/docs` path returns null so callers can skip
 * the event entirely.
 */
export function deriveDocsSection(pathname: string): string | null {
  if (!pathname.startsWith("/docs")) return null;
  const segments = pathname.split("/").filter(Boolean);
  if (segments.length <= 1) return "overview";
  return segments[1];
}

export function trackDocsPageView(pathname: string): void {
  const section = deriveDocsSection(pathname);
  if (section === null) return;
  capture("docs_page_view", { slug: pathname, section });
}

export function trackDocsSearchQuery(query: string, resultCount: number): void {
  capture("docs_search_query", {
    query,
    result_count: resultCount,
    section: "docs",
  });
  if (resultCount === 0) trackDocsSearchZeroResult(query);
}

export function trackDocsSearchZeroResult(query: string): void {
  capture("docs_search_zero_result", { query, section: "docs" });
}

export function trackDocsCodeCopy(payload: {
  snippetId: string;
  slug: string;
  language?: string;
}): void {
  capture("docs_code_copy", {
    snippet_id: payload.snippetId,
    slug: payload.slug,
    language: payload.language ?? "plaintext",
  });
}
