import type { MetadataRoute } from "next";
import type { Locale } from "next-intl";
import { source } from "@/lib/source";
import { routing } from "@/i18n/routing";
import { localePath } from "@/lib/i18n-metadata";

export const dynamic = "force-static";

// Hardcoded absolute origin - do NOT switch to a metadataBase-relative path.
// Sitemap URLs must be fully qualified per the Sitemap Protocol.
const ORIGIN = "https://paneflow.dev";

// Marketing route catalogue. Each entry produces one sitemap entry per
// locale in `routing.locales` (US-013). Docs routes under `/docs/**` are
// auto-enumerated from the Fumadocs source loader; they remain
// locale-unaware in v1 (docs i18n is deferred to a P3 PRD).
//
// MAINTENANCE POLICY: every new top-level marketing route MUST be added
// here in the SAME PR. Forgetting this is the most common SEO regression
// on static builds - Googlebot will not auto-discover orphan routes.
interface MarketingRoute {
  path: string; // EN-canonical pathname, e.g. "/", "/compare/warp"
  changeFrequency: NonNullable<MetadataRoute.Sitemap[number]["changeFrequency"]>;
  priority: number;
}

const MARKETING_ROUTES: MarketingRoute[] = [
  { path: "/", changeFrequency: "monthly", priority: 1.0 },
  { path: "/download", changeFrequency: "weekly", priority: 0.9 },
  { path: "/about", changeFrequency: "monthly", priority: 0.7 },
  { path: "/compare", changeFrequency: "monthly", priority: 0.7 },
  { path: "/compare/cmux", changeFrequency: "monthly", priority: 0.7 },
  { path: "/compare/wezterm", changeFrequency: "monthly", priority: 0.7 },
  { path: "/compare/iterm2", changeFrequency: "monthly", priority: 0.7 },
  { path: "/compare/warp", changeFrequency: "monthly", priority: 0.7 },
  { path: "/legal/privacy", changeFrequency: "yearly", priority: 0.3 },
];

export default function sitemap(): MetadataRoute.Sitemap {
  const buildTime = new Date();
  return [...buildMarketingEntries(buildTime), ...buildDocsEntries(buildTime)];
}

// One entry per (route × locale). Each entry carries the full hreflang
// cluster under `alternates.languages` (Next emits this as
// `<xhtml:link rel="alternate" hreflang="..." href="..."/>` siblings of
// the `<loc>` tag). x-default points at the default-locale URL.
function buildMarketingEntries(buildTime: Date): MetadataRoute.Sitemap {
  return MARKETING_ROUTES.flatMap(({ path, changeFrequency, priority }) => {
    const languages: Record<string, string> = {};
    for (const loc of routing.locales) {
      languages[loc] = `${ORIGIN}${localePath(loc, path)}`;
    }
    languages["x-default"] = `${ORIGIN}${localePath(routing.defaultLocale, path)}`;

    return routing.locales.map((loc) => ({
      url: `${ORIGIN}${localePath(loc, path)}`,
      lastModified: buildTime,
      changeFrequency,
      priority,
      alternates: { languages },
    }));
  });
}

interface DocsPageMeta {
  slugs: string[];
  url: string;
  locale?: string;
  data: { dateModified?: string };
}

/**
 * Enumerate every `/docs/**` route from the Fumadocs source loader.
 * `lastModified` is read from the page's `dateModified` frontmatter
 * (parsed by the extended schema in `source.config.ts`); pages without
 * a `dateModified` fall back to the current build time so the entry is
 * still emitted with a sensible value.
 *
 * Each (page × locale × format) triple emits ONE sitemap entry:
 *   - HTML route   (priority 0.7)
 *   - `.md` raw-markdown twin (priority 0.5), served via the rewrite
 *     handler at `src/app/api/docs-raw/[[...slug]]/route.ts` so
 *     traditional crawlers that only consume sitemaps can discover
 *     the markdown form without relying on `/llms.txt`.
 *
 * Every entry carries the full hreflang cluster under
 * `alternates.languages` (Next emits this as
 * `<xhtml:link rel="alternate" hreflang="...">` siblings of `<loc>`):
 *   - one URL per locale in `routing.locales` (6)
 *   - one `x-default` pointing at the EN URL
 * Without this, Google may treat per-locale variants as duplicate
 * content rather than as a language cluster, fragmenting authority.
 *
 * `source.getPages()` (no arg) returns one Page per (slug × locale)
 * thanks to `i18n: fumadocsI18n` in `src/lib/source.ts`. The cluster
 * is identical across all locale variants of a given slug, so we
 * compute it once per slug and reuse it for every variant + the `.md`
 * twin.
 *
 * Unhappy paths:
 *   - page with no `dateModified` frontmatter -> fall back to build time
 *   - any throw or empty page set -> return `[]` so the marketing routes
 *     still ship in the sitemap
 */
function buildDocsEntries(buildTime: Date): MetadataRoute.Sitemap {
  try {
    const pages = source.getPages() as unknown as DocsPageMeta[];
    return pages.flatMap((page) => {
      const lastModified = parseDateOrFallback(
        page.data.dateModified,
        buildTime,
      );
      // Locale-independent canonical path. Fumadocs `baseUrl: "/docs"`
      // + slugs reconstructs the EN-canonical pathname; localePath()
      // re-applies the locale prefix per the routing config.
      const canonicalPath =
        page.slugs.length > 0 ? `/docs/${page.slugs.join("/")}` : "/docs";

      const htmlLanguages: Record<string, string> = {};
      const markdownLanguages: Record<string, string> = {};
      for (const loc of routing.locales) {
        const absHtml = `${ORIGIN}${localePath(loc, canonicalPath)}`;
        htmlLanguages[loc] = absHtml;
        markdownLanguages[loc] = `${absHtml}.md`;
      }
      const defaultHtml = `${ORIGIN}${localePath(routing.defaultLocale, canonicalPath)}`;
      htmlLanguages["x-default"] = defaultHtml;
      markdownLanguages["x-default"] = `${defaultHtml}.md`;

      // page.locale is typed as `string | undefined` by Fumadocs, but the
      // i18n loader guarantees it is one of routing.locales (or undefined
      // for non-i18n setups, which we don't have here).
      const currentLocale = (page.locale ?? routing.defaultLocale) as Locale;
      const ownHtml = `${ORIGIN}${localePath(currentLocale, canonicalPath)}`;
      const ownMarkdown = `${ownHtml}.md`;

      return [
        {
          url: ownHtml,
          lastModified,
          changeFrequency: "weekly" as const,
          priority: 0.7,
          alternates: { languages: htmlLanguages },
        },
        {
          url: ownMarkdown,
          lastModified,
          changeFrequency: "weekly" as const,
          priority: 0.5,
          alternates: { languages: markdownLanguages },
        },
      ];
    });
  } catch (err) {
    console.error(
      "sitemap: failed to enumerate /docs/** pages; emitting marketing routes only",
      err,
    );
    return [];
  }
}

function parseDateOrFallback(value: string | undefined, fallback: Date): Date {
  if (!value) return fallback;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? fallback : parsed;
}
