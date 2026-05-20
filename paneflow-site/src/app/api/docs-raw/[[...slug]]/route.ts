import { NextResponse, type NextRequest } from "next/server";
import { readPageMarkdown } from "@/lib/docs-llms";
import { source } from "@/lib/source";

export const dynamic = "force-static";
export const revalidate = false;

/**
 * Per-page raw Markdown endpoint. Powers two things:
 *   1. AI crawlers and LLM agents that fetch the cleanest possible
 *      representation of a docs page (no JSX, no frontmatter, plain
 *      Markdown).
 *   2. The `.md` rewrite in `next.config.ts`: `/docs/<slug>.md` is
 *      rewritten to `/api/docs-raw/<slug>` and served from here. Anyone
 *      can hand a docs URL with `.md` appended to an LLM that supports
 *      browsing.
 *
 * The body is sourced from `readPageMarkdown(page)` (lib/docs-llms.ts),
 * which strips frontmatter + PascalCase JSX tags. Reuses the exact same
 * transform used by `/llms-full.txt` so the raw representation stays
 * consistent across all surfaces.
 */
export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ slug?: string[] }> },
): Promise<NextResponse> {
  const { slug } = await params;
  const page = source.getPage(slug);
  if (!page) {
    return new NextResponse("Not found", { status: 404 });
  }

  const markdown = await readPageMarkdown(page);
  if (markdown == null) {
    return new NextResponse("Not found", { status: 404 });
  }

  return new NextResponse(markdown, {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8",
      "Cache-Control": "public, max-age=0, s-maxage=3600",
    },
  });
}

export function generateStaticParams(): Array<{ slug: string[] }> {
  return source.generateParams().map((p) => ({ slug: p.slug }));
}
