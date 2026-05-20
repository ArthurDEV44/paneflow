import { createMDX } from "fumadocs-mdx/next";
import type { NextConfig } from "next";

// `output: "export"` was removed (commit pending) so that
// `app/api/waitlist/route.ts` can run as a Vercel Function. Static pages
// stay statically rendered on the CDN automatically — Vercel detects the
// build artifact and routes per-page. The `images.unoptimized` flag is
// dropped at the same time to re-enable Next/Image optimization on the
// optimized Vercel pipeline (free, ~1.5 MB hero replaced by webp/avif
// responsive variants).
const nextConfig: NextConfig = {
  // `/docs/<slug>.md` -> raw Markdown handler. Lets AI crawlers and the
  // "Open in <LLM>" buttons reference a clean Markdown URL alongside the
  // rendered HTML page. The handler lives at `app/api/docs-raw/[[...slug]]`.
  async rewrites() {
    return [
      { source: "/docs.md", destination: "/api/docs-raw" },
      { source: "/docs/:slug*.md", destination: "/api/docs-raw/:slug*" },
    ];
  },
};

const withMDX = createMDX();

export default withMDX(nextConfig);
