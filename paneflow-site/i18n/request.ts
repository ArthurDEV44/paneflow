import { hasLocale } from "next-intl";
import { getRequestConfig } from "next-intl/server";
import { routing } from "./routing";

// Deep-merge two message trees. The override tree wins for any leaf key
// that exists in it; missing keys fall through to the base. Used so
// newly-added locales can ship with partial translations: English keys
// cover anything not yet localized, and next-intl never throws a
// MISSING_MESSAGE error in the meantime. Essential for rolling out
// ja/de/es without a complete pass on the deep compare/* pages — those
// keys fall back to EN until a human translator fills them.
function deepMerge(
  base: Record<string, unknown>,
  override: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(override)) {
    const baseValue = out[key];
    if (
      value !== null &&
      typeof value === "object" &&
      !Array.isArray(value) &&
      baseValue !== null &&
      typeof baseValue === "object" &&
      !Array.isArray(baseValue)
    ) {
      out[key] = deepMerge(
        baseValue as Record<string, unknown>,
        value as Record<string, unknown>,
      );
    } else {
      out[key] = value;
    }
  }
  return out;
}

export default getRequestConfig(async ({ requestLocale }) => {
  const requested = await requestLocale;
  const locale = hasLocale(routing.locales, requested)
    ? requested
    : routing.defaultLocale;

  const defaultMessages = (
    await import(`../messages/${routing.defaultLocale}.json`)
  ).default as Record<string, unknown>;

  if (locale === routing.defaultLocale) {
    return { locale, messages: defaultMessages };
  }

  let localeMessages: Record<string, unknown>;
  try {
    localeMessages = (await import(`../messages/${locale}.json`))
      .default as Record<string, unknown>;
  } catch {
    console.warn(
      `[i18n] messages/${locale}.json missing; falling back to ${routing.defaultLocale}`,
    );
    return { locale, messages: defaultMessages };
  }

  return { locale, messages: deepMerge(defaultMessages, localeMessages) };
});
