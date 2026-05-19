import type { MetadataRoute } from "next";

// Required by `output: "export"` - emits a static `out/robots.txt` at build time.
export const dynamic = "force-static";

// Hardcoded absolute URL - do NOT switch to a metadataBase-relative path.
// US-001 must work even before US-005 ships metadataBase.
const SITEMAP_URL = "https://paneflow.dev/sitemap.xml";

// Fully open policy: allow every crawler, including AI training AND retrieval
// agents. This is an intentional reversal of the earlier default-deny stance.
//
// Reasoning:
// 1. AEO/GEO: appearing in ChatGPT, Claude, Gemini, and Perplexity answers
//    requires the retrieval/search bots (OAI-SearchBot, ChatGPT-User,
//    Claude-SearchBot, Claude-User, PerplexityBot, Perplexity-User) to be
//    able to fetch the site. Blocking these kills citation visibility - the
//    earlier blocklist mistakenly mixed training-only bots with retrieval
//    bots (PerplexityBot is retrieval, not training).
// 2. Brand entity in foundational models: allowing training crawlers
//    (GPTBot, ClaudeBot, Google-Extended, CCBot, etc.) lets Paneflow's
//    canonical positioning land in future model weights. For a small
//    open-source product in discovery phase, this is a compounding ROI
//    on 2-5 years - when a dev asks ChatGPT-2027 "what is Paneflow",
//    the model answers from baked-in knowledge.
// 3. The marketing copy on paneflow.dev is already public; absorbing it
//    into training corpora is not a strategic loss.
//
// If this policy is ever reverted, do NOT block PerplexityBot - it is the
// retrieval index for Perplexity citations, not a training bot. See the
// per-vendor crawler taxonomy: OpenAI (platform.openai.com/docs/bots),
// Anthropic (support.claude.com), Google (developers.google.com/search),
// Perplexity (docs.perplexity.ai).
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: "/",
      },
    ],
    sitemap: SITEMAP_URL,
  };
}
