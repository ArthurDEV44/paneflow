import { loader } from "fumadocs-core/source";
import { docs } from "collections/server";
import { fumadocsI18n } from "@/lib/i18n-fumadocs";

/*
 * Fumadocs source loader. Server-only.
 *
 * i18n-aware (defineI18n config in @/lib/i18n-fumadocs). One page tree
 * per declared locale; URLs respect `hideLocale: "default-locale"`
 * (EN at `/docs/<slug>`, non-EN at `/<locale>/docs/<slug>`). Content
 * stays flat under `content/docs/`; per-locale MDX uses the `<page>.<locale>.mdx`
 * convention (parser: "dot", the default). Pages without a locale-specific
 * file fall back to the defaultLanguage (EN) automatically.
 */
export const source = loader({
  baseUrl: "/docs",
  i18n: fumadocsI18n,
  source: docs.toFumadocsSource(),
});
