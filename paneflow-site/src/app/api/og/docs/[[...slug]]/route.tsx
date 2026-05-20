import { ImageResponse } from "next/og";
import { notFound } from "next/navigation";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { source } from "@/lib/source";

/*
 * Per-page Open Graph image for /docs/<slug> (US-004).
 *
 * Implemented as a Route Handler under /api/og/docs/<slug> rather than
 * a colocated `opengraph-image.tsx` because Next.js 16 rejects the
 * metadata file convention inside an optional catch-all segment
 * (`[[...slug]]/opengraph-image` puts the catch-all not-last, which
 * fails the routing precondition). The Route Handler path keeps the
 * `[[...slug]]` catch-all as the final URL segment, sidestepping the
 * limitation (Risk #6 in tasks/prd-seo-aeo-polish.md).
 *
 * Static-prerendered at build time via `generateStaticParams`; one PNG
 * per docs slug. Consumed by `generateMetadata` in
 * `/docs/[[...slug]]/page.tsx` via `openGraph.images`.
 *
 * Falls back to `notFound()` on an unknown slug; the social platform
 * receives a 404 and reverts to the sitewide `/opengraph-image`
 * declared on the root layout.
 */

export const dynamic = "force-static";
export const dynamicParams = false;

export function generateStaticParams(): Array<{ slug: string[] }> {
  return source.generateParams().map((p) => ({ slug: p.slug }));
}

const BG = "#0a0a0a";
const SURFACE = "#141414";
const BORDER = "rgba(255, 255, 255, 0.08)";
const TEXT = "#e8e8e8";
const TEXT_MUTED = "#888888";
const ACCENT = "#a3e635";

const FALLBACK_TITLE = "Paneflow Documentation";
const FALLBACK_DESCRIPTION =
  "Terminal workspace for orchestrating Claude Code, Codex, and OpenCode.";

const SIZE = { width: 1200, height: 630 };

export async function GET(
  _request: Request,
  context: { params: Promise<{ slug?: string[] }> },
): Promise<Response> {
  const { slug } = await context.params;
  const page = source.getPage(slug);
  if (!page) notFound();

  const title =
    typeof page.data.title === "string" && page.data.title.length > 0
      ? page.data.title
      : FALLBACK_TITLE;
  const description =
    typeof page.data.description === "string" && page.data.description.length > 0
      ? page.data.description
      : FALLBACK_DESCRIPTION;

  const logoData = await readFile(
    join(process.cwd(), "public/logos/paneflow-web-300.png"),
  );
  const logoSrc = `data:image/png;base64,${logoData.toString("base64")}`;

  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          background: BG,
          display: "flex",
          flexDirection: "column",
          padding: "72px 88px",
          color: TEXT,
          fontFamily:
            '"Helvetica Neue", "Helvetica", "Arial", system-ui, sans-serif',
        }}
      >
        {/* Header: logo + wordmark */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 24,
          }}
        >
          <img
            src={logoSrc}
            width={64}
            height={64}
            alt=""
            style={{ borderRadius: 14 }}
          />
          <div
            style={{
              fontSize: 44,
              fontWeight: 600,
              letterSpacing: -1.2,
              color: TEXT,
              display: "flex",
            }}
          >
            Paneflow
          </div>
        </div>

        {/* Body: documentation eyebrow + title + description */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: 64,
            gap: 24,
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 14,
              fontSize: 22,
              fontWeight: 500,
              letterSpacing: 2,
              textTransform: "uppercase",
              color: ACCENT,
            }}
          >
            <div
              style={{
                width: 10,
                height: 10,
                borderRadius: 999,
                background: ACCENT,
                display: "flex",
              }}
            />
            <div style={{ display: "flex" }}>Documentation</div>
          </div>
          <div
            style={{
              fontSize: title.length > 48 ? 72 : 88,
              fontWeight: 700,
              lineHeight: 1.05,
              letterSpacing: -2,
              color: TEXT,
              maxWidth: 1024,
              display: "flex",
            }}
          >
            {title}
          </div>
          <div
            style={{
              fontSize: 28,
              color: TEXT_MUTED,
              lineHeight: 1.3,
              maxWidth: 960,
              display: "-webkit-box",
              WebkitLineClamp: 2,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
            }}
          >
            {description}
          </div>
        </div>

        {/* Footer: domain badge + page path */}
        <div
          style={{
            marginTop: "auto",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            width: "100%",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 14,
              fontSize: 24,
              color: TEXT_MUTED,
            }}
          >
            <div
              style={{
                width: 10,
                height: 10,
                borderRadius: 999,
                background: TEXT,
                display: "flex",
              }}
            />
            <div style={{ display: "flex" }}>paneflow.dev</div>
          </div>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              fontSize: 22,
              color: TEXT_MUTED,
              fontFamily:
                '"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace',
              padding: "10px 18px",
              background: SURFACE,
              border: `1px solid ${BORDER}`,
              borderRadius: 999,
            }}
          >
            {page.url}
          </div>
        </div>
      </div>
    ),
    {
      ...SIZE,
    },
  );
}
