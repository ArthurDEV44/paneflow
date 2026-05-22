import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { routing } from "@/i18n/routing";
import { ogLocaleFor } from "@/lib/i18n-bcp47";

type Alternates = NonNullable<Metadata["alternates"]>;
type OpenGraphLocale = {
  locale: string;
  alternateLocale: string[];
};

// Build a path with the appropriate locale prefix. The default locale
// emits no prefix (matches `localePrefix: 'as-needed'` in routing.ts).
// Any other locale produces `/<locale>` + path. Adding a new locale to
// `routing.locales` automatically extends the surface area - no edits
// needed here or in any sitemap/metadata consumer.
//
// `path` is the EN-canonical pathname: leading slash, no trailing
// slash, e.g. "/", "/about", "/compare/warp".
export function localePath(locale: Locale, path: string): string {
  if (locale === routing.defaultLocale) return path;
  if (path === "/") return `/${locale}`;
  return `/${locale}${path}`;
}

// Builds the Next.js `alternates` metadata block for a route.
// `path` is the EN-canonical pathname (no locale prefix, leading slash).
// `locale` is the current page's locale — the returned `canonical` points
// at *that* locale's URL so each locale variant is its own canonical
// (the correct i18n pattern; if we set canonical=EN for all variants,
// Google can treat FR/zh-Hans as duplicates and skip indexing them).
// `languages` always emits the full hreflang cluster (one entry per
// `routing.locales` plus x-default) regardless of which locale we render
// — required for the reciprocal-hreflang rule.
//
// The helper is purely string-based: passing an unmapped path (e.g.
// `/nonexistent`) still returns the full alternates and a canonical
// built from the same path. No route validation by design.
export function buildAlternates(path: string, locale: Locale): Alternates {
  const languages: Record<string, string> = {};
  for (const loc of routing.locales) {
    languages[loc] = localePath(loc, path);
  }
  languages["x-default"] = localePath(routing.defaultLocale, path);
  return {
    canonical: localePath(locale, path),
    languages,
  };
}

// Open Graph locale metadata. Facebook + LinkedIn use `og:locale` to pick
// the right preview localisation when a URL is shared. `og:locale:alternate`
// signals the other available variants so a FR user sharing the EN URL on
// LinkedIn FR gets the EN preview but the platform knows a FR variant
// exists. Spread the return value into the page's `openGraph` block.
export function buildOpenGraphLocale(locale: Locale): OpenGraphLocale {
  return {
    locale: ogLocaleFor(locale),
    alternateLocale: routing.locales
      .filter((loc) => loc !== locale)
      .map(ogLocaleFor),
  };
}
