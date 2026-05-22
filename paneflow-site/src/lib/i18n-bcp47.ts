// Maps an internal locale token (matches `routing.locales`) to the BCP 47
// language tag used in JSON-LD `inLanguage` fields. Google + AI engines
// expect specific BCP 47 forms; this central mapping keeps the codebase
// from sprinkling string literals around.
//
// Param is intentionally `string`, not the `Locale` union: the function
// is defensive (falls back to "en-US" + dev warning on unknown values),
// which lets callers pass `params.locale` from Next.js dynamic segments
// without having to narrow through `hasLocale` first.
//
//   en       -> en-US     (default English variant for the site)
//   fr       -> fr-FR     (regional French; matches the audience)
//   zh-Hans  -> zh-Hans   (script-only tag, per Google guidance)
//   ja       -> ja-JP     (regional Japanese; only variant in active use)
//   de       -> de-DE     (regional German; broader DACH reach acceptable)
//   es       -> es        (no region — covers ES + LATAM without bias)
//
// Unhappy path: an unknown locale falls back to `"en-US"` and logs a
// warning in development. This keeps schema valid even if a new locale
// is added to `routing.locales` before this map is updated.
const BCP47_MAP: Record<string, string> = {
  en: "en-US",
  fr: "fr-FR",
  "zh-Hans": "zh-Hans",
  ja: "ja-JP",
  de: "de-DE",
  es: "es",
};

// Open Graph locale tags use the IETF underscore form `xx_XX`, not the
// BCP 47 hyphen form. Facebook's documented locale list also expects
// regional variants (so zh-Hans -> zh_CN, es -> es_ES). Twitter accepts
// either form but follows Open Graph for consistency.
//
// Reference: https://developers.facebook.com/docs/internationalization
const OG_LOCALE_MAP: Record<string, string> = {
  en: "en_US",
  fr: "fr_FR",
  "zh-Hans": "zh_CN",
  ja: "ja_JP",
  de: "de_DE",
  es: "es_ES",
};

export function bcp47ForJsonLd(locale: string): string {
  const tag = BCP47_MAP[locale];
  if (tag) return tag;
  if (process.env.NODE_ENV !== "production") {
    console.warn(
      `bcp47ForJsonLd: unknown locale "${locale}", falling back to "en-US". ` +
        `Add a mapping in src/lib/i18n-bcp47.ts.`,
    );
  }
  return "en-US";
}

export function ogLocaleFor(locale: string): string {
  const tag = OG_LOCALE_MAP[locale];
  if (tag) return tag;
  if (process.env.NODE_ENV !== "production") {
    console.warn(
      `ogLocaleFor: unknown locale "${locale}", falling back to "en_US". ` +
        `Add a mapping in src/lib/i18n-bcp47.ts.`,
    );
  }
  return "en_US";
}
