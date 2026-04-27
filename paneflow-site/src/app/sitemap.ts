import type { MetadataRoute } from "next";

// Required by `output: "export"` — emits a static `out/sitemap.xml` at build time.
export const dynamic = "force-static";

// Hardcoded absolute origin — do NOT switch to a metadataBase-relative path.
// US-002 must work even before US-005 ships metadataBase.
const ORIGIN = "https://paneflow.dev";

// MAINTENANCE POLICY: every new route added to the site MUST also be added
// here in the SAME PR. Forgetting this is the most common SEO regression on
// static-export builds — Googlebot will not auto-discover orphan routes.
export default function sitemap(): MetadataRoute.Sitemap {
  const lastModified = new Date();

  return [
    {
      url: `${ORIGIN}/`,
      lastModified,
      changeFrequency: "monthly",
      priority: 1.0,
    },
    {
      url: `${ORIGIN}/download`,
      lastModified,
      changeFrequency: "weekly",
      priority: 0.9,
    },
    {
      url: `${ORIGIN}/about`,
      lastModified,
      changeFrequency: "monthly",
      priority: 0.7,
    },
    {
      url: `${ORIGIN}/legal/privacy`,
      lastModified,
      changeFrequency: "yearly",
      priority: 0.3,
    },
  ];
}
