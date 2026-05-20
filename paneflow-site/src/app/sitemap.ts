import type { MetadataRoute } from "next";
import { source } from "@/lib/source";

export const dynamic = "force-static";

// Hardcoded absolute origin - do NOT switch to a metadataBase-relative path.
// US-002 must work even before US-005 ships metadataBase.
const ORIGIN = "https://paneflow.dev";

// MAINTENANCE POLICY: docs routes under /docs/** are auto-enumerated from
// the Fumadocs source loader (see buildDocsEntries below); only top-level
// pages remain manual. Every new top-level route (NOT under /docs/)
// added to the site MUST also be added here in the SAME PR. Forgetting
// this is the most common SEO regression on static builds - Googlebot
// will not auto-discover orphan routes.
export default function sitemap(): MetadataRoute.Sitemap {
  const buildTime = new Date();

  const manual: MetadataRoute.Sitemap = [
    {
      url: `${ORIGIN}/`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 1.0,
    },
    {
      url: `${ORIGIN}/download`,
      lastModified: buildTime,
      changeFrequency: "weekly",
      priority: 0.9,
    },
    {
      url: `${ORIGIN}/about`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/compare`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/compare/cmux`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/compare/wezterm`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/compare/iterm2`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/compare/warp`,
      lastModified: buildTime,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/legal/privacy`,
      lastModified: buildTime,
      changeFrequency: "yearly",
      priority: 0.3,
    },
  ];

  return [...manual, ...buildDocsEntries(buildTime)];
}

interface DocsPageMeta {
  url: string;
  data: { dateModified?: string };
}

/**
 * Enumerate every `/docs/**` route from the Fumadocs source loader.
 * `lastModified` is read from the page's `dateModified` frontmatter
 * (parsed by the extended schema in `source.config.ts`); pages without
 * a `dateModified` fall back to the current build time so the entry is
 * still emitted with a sensible value.
 *
 * Each docs page emits TWO entries: the HTML route (canonical) and the
 * `.md` raw-markdown twin served via the rewrite in `next.config.ts`
 * (handler at `src/app/api/docs-raw/[[...slug]]/route.ts`). The `.md`
 * URL shares the HTML twin's `lastModified` so traditional crawlers
 * that only consume sitemaps can discover the markdown form without
 * relying on `/llms.txt` (US-002).
 *
 * Unhappy path: any throw or empty page set returns `[]` so the manual
 * top-level routes still ship in the sitemap.
 */
function buildDocsEntries(buildTime: Date): MetadataRoute.Sitemap {
  try {
    const pages = source.getPages() as unknown as DocsPageMeta[];
    return pages.flatMap((page) => {
      const lastModified = parseDateOrFallback(
        page.data.dateModified,
        buildTime,
      );
      const html = {
        url: `${ORIGIN}${page.url}`,
        lastModified,
        changeFrequency: "weekly" as const,
        priority: 0.7,
      };
      const markdown = {
        url: `${ORIGIN}${page.url}.md`,
        lastModified,
        changeFrequency: "weekly" as const,
        priority: 0.5,
      };
      return [html, markdown];
    });
  } catch (err) {
    console.error(
      "sitemap: failed to enumerate /docs/** pages; emitting manual routes only",
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
