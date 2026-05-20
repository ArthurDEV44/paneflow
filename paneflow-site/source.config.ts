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
