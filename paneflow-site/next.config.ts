import { createMDX } from "fumadocs-mdx/next";
import type { NextConfig } from "next";
import createNextIntlPlugin from "next-intl/plugin";

// `output: "export"` was removed (commit pending) so that
// `app/api/waitlist/route.ts` can run as a Vercel Function. Static pages
// stay statically rendered on the CDN automatically — Vercel detects the
// build artifact and routes per-page. The `images.unoptimized` flag is
// dropped at the same time to re-enable Next/Image optimization on the
// optimized Vercel pipeline (free, ~1.5 MB hero replaced by webp/avif
// responsive variants).
const nextConfig: NextConfig = {
  images: {
    // Next.js 16 introduced an explicit allowlist for the <Image quality={...}>
    // prop. Default is [75]; values outside the list are silently clamped to
    // the closest allowed entry (no runtime error). Without 95 in the list,
    // the navbar logo gets re-encoded at quality=75 and looks soft.
    // Docs: node_modules/next/dist/docs/01-app/03-api-reference/02-components/image.md
    // (line 712: "required starting with Next.js 16 because unrestricted access
    // could allow malicious actors to optimize more qualities than you intended").
    qualities: [75, 95],
  },
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
const withNextIntl = createNextIntlPlugin();

export default withNextIntl(withMDX(nextConfig));
