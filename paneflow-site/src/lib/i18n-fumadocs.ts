import { defineI18n } from "fumadocs-core/i18n";
import { routing } from "../../i18n/routing";

// Fumadocs i18n config, derived from the next-intl `routing` source of
// truth so the two stay in lockstep (adding a locale to routing.ts
// automatically extends docs to that locale).
//
// `hideLocale: "default-locale"` mirrors next-intl's
// `localePrefix: "as-needed"`: EN docs URLs stay at `/docs/<slug>` with
// no prefix, non-EN at `/<locale>/docs/<slug>`.
//
// `fallbackLanguage` defaults to `defaultLanguage` (EN) when omitted —
// so any non-EN docs route without a matching `<page>.<locale>.mdx`
// file automatically renders the EN content. This lets us ship the
// route topology now and add translated MDX (file naming convention:
// `<page>.<locale>.mdx`, e.g. `installation.fr.mdx`) incrementally.
//
// `parser: "dot"` is the default — kept implicit.
export const fumadocsI18n = defineI18n({
  defaultLanguage: routing.defaultLocale,
  languages: [...routing.locales],
  hideLocale: "default-locale",
});
