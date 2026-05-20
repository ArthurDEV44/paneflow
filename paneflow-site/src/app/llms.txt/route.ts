import { buildLlmsTxt } from "@/lib/docs-llms";

export const revalidate = false;
export const dynamic = "force-static";

export function GET(): Response {
  const body = buildLlmsTxt();
  return new Response(body, {
    status: 200,
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=0, must-revalidate",
    },
  });
}
