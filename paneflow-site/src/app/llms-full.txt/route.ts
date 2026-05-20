import { buildLlmsFullTxt } from "@/lib/docs-llms";

export const revalidate = false;
export const dynamic = "force-static";

export async function GET(): Promise<Response> {
  const body = await buildLlmsFullTxt();
  return new Response(body, {
    status: 200,
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=0, must-revalidate",
    },
  });
}
