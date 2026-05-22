import { defineRouting } from "next-intl/routing";

export const routing = defineRouting({
  locales: ["en", "fr", "zh-Hans", "ja", "de", "es"],
  defaultLocale: "en",
  localePrefix: "as-needed",
});
