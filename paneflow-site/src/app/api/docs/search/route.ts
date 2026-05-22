import { createFromSource } from "fumadocs-core/search/server";
import { source } from "@/lib/source";

export const revalidate = false;
export const dynamic = "force-static";

// Per-locale Orama tokenizers. The source `loader()` is i18n-aware, so
// `createFromSource` routes to `createI18nSearchAPI` under the hood and
// requires a tokenizer per declared locale. Without `localeMap` the call
// silently fails to build an index for any locale whose token name does
// not match an Orama stemmer key, surfacing as a 500 on /api/docs/search.
//
// Orama 3.x supports: english, french, german, spanish (and others), but
// NOT japanese and NOT any CJK script. `ja` and `zh-Hans` fall back to
// `english` — acceptable for code-heavy docs where exact-match dominates
// over stemming. Revisit when @orama/stemmers ships CJK support.
const localeMap = {
  en: "english",
  fr: "french",
  de: "german",
  es: "spanish",
  ja: "english",
  "zh-Hans": "english",
} as const;

export const { staticGET: GET } = createFromSource(source, { localeMap });
