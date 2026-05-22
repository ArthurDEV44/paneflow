import { notFound } from "next/navigation";

// Catch-all under `[locale]/` so any unknown path inside a valid
// locale segment is funnelled through `notFound()` and renders the
// localized `[locale]/not-found.tsx` under the layout (which has
// already set up the i18n context). Without this, Next.js silently
// falls back to its default global 404 because no other page module
// matches the URL.
//
// Required by next-intl's documented pattern - see
// https://next-intl.dev/docs/environments/error-files and US-017 of
// tasks/prd-i18n-fr-zh-Hans.md.
export default function CatchAllPage() {
  notFound();
}
