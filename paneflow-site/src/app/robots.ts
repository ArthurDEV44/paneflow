import type { MetadataRoute } from "next";

// Required by `output: "export"` — emits a static `out/robots.txt` at build time.
export const dynamic = "force-static";

// Hardcoded absolute URL — do NOT switch to a metadataBase-relative path.
// US-001 must work even before US-005 ships metadataBase.
const SITEMAP_URL = "https://paneflow.dev/sitemap.xml";

// Default-deny AI training crawlers. PaneFlow is open source, but the
// marketing copy on paneflow.dev should not feed LLM training corpora
// without explicit consent. Search-engine indexers (Googlebot, Bingbot,
// DuckDuckBot, etc.) remain allowed via the wildcard rule below.
const AI_TRAINING_CRAWLERS = [
  "GPTBot",
  "ClaudeBot",
  "PerplexityBot",
  "Google-Extended",
  "Bytespider",
  "CCBot",
];

export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: "/",
      },
      {
        userAgent: AI_TRAINING_CRAWLERS,
        disallow: "/",
      },
    ],
    sitemap: SITEMAP_URL,
  };
}
