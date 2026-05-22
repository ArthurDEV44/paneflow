import { pageSchema } from "fumadocs-core/source/schema";
import { defineConfig, defineDocs } from "fumadocs-mdx/config";
import { z } from "zod";

/**
 * Custom frontmatter fields:
 *   - `dateModified` (string): ISO date of last meaningful edit. Sitemap
 *     and llms.txt consume this; rendering ignores it.
 *   - `howto` (boolean): force HowTo JSON-LD emission. The renderer also
 *     auto-detects pages with 3+ `## How do I ...?` H2s, so this flag
 *     only overrides edge cases.
 *   - `faqpage` (boolean): force FAQPage JSON-LD emission. Drives the
 *     troubleshooting page.
 *   - `lastSyncedFrom` (string, 7-40 lowercase hex chars): present ONLY
 *     on translated MDX (any `*.<locale>.mdx`); absent on the EN source
 *     of truth. Records the EN-source commit SHA that the translation
 *     was synced from, so `scripts/check-docs-stale.ts` (US-008) can
 *     compare against the current EN HEAD and surface stale translations.
 *     Mirrors the Docusaurus / Astro convention. EN source files must
 *     NOT set this field (Zod accepts undefined to enforce that by
 *     contract — translators consult AGENTS.md for the workflow).
 */
export const docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: pageSchema.extend({
      // YAML auto-parses bare ISO dates (`2026-05-19`) into Date objects.
      // Accept both forms and normalise to an ISO date string so callers
      // (sitemap, llms.txt) get a stable representation.
      dateModified: z
        .union([
          z.string(),
          z.date().transform((d) => d.toISOString().slice(0, 10)),
        ])
        .optional(),
      howto: z.boolean().optional(),
      faqpage: z.boolean().optional(),
      // Lowercase hex SHA, 7 to 40 chars (short SHA up to full SHA-1).
      // Validation only — the freshness check itself lives in
      // scripts/check-docs-stale.ts (US-008). A malformed value here
      // fails the MDX build with a clear Zod error pointing at the file.
      lastSyncedFrom: z
        .string()
        .regex(/^[0-9a-f]{7,40}$/, {
          message:
            "lastSyncedFrom must be a 7-40 character lowercase hex SHA (the EN-source commit this translation was synced from).",
        })
        .optional(),
    }),
    // Build-time post-processing: expose the AST-extracted Markdown body
    // via `page.data.getText("processed")` at runtime. The processed pass
    // operates on the parsed MDX AST, so frontmatter, top-level imports,
    // and code blocks are handled correctly without regex risk.
    //
    // Note: Fumadocs hardcodes `filterElement` internally to only drop
    // `mdxjsEsm` nodes - it does NOT strip MDX JSX components like
    // `<Callout>` or `<VersionBadge/>` from the output (see
    // `node_modules/fumadocs-core/dist/mdx-plugins/remark-llms.js`, lines
    // 8-19). The user-facing API silently overrides any custom
    // `filterElement` we pass. A regex post-strip in
    // `src/lib/docs-llms.ts::readPageMarkdown` finishes the job for now.
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
});

export default defineConfig();
