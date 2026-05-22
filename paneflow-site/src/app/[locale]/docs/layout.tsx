// eslint-disable-next-line no-restricted-imports -- fumadocs-ui migration tracked in tasks/prd-fumadocs-docs.md
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { Suspense } from "react";
import type * as React from "react";
import { SearchUrlSync } from "@/components/docs/search-url-sync";
import { LanguageSwitcher } from "@/components/language-switcher";
import { source } from "@/lib/source";

// Segment layout — no <html>/<body>/fonts/providers here. Those are owned
// by the parent `[locale]/layout.tsx` (which already wraps everything
// with NextIntlClientProvider + Providers + PHProvider + Fumadocs
// RootProvider). This layout only adds the Fumadocs sidebar/nav chrome
// keyed to the active locale.
//
// `source.pageTree[locale]` resolves the locale-specific page tree
// (Fumadocs builds one tree per language declared in `defineI18n`).
// When a locale has no translated MDX, Fumadocs falls back to the
// defaultLanguage tree per `fallbackLanguage` (default = "en") — so
// /fr/docs renders the EN sidebar + EN content until FR translations land.
export default async function Layout({
  children,
  params,
}: {
  children: React.ReactNode;
  params: Promise<{ locale: string }>;
}): Promise<React.ReactElement> {
  const { locale } = await params;
  return (
    <DocsLayout
      tree={source.pageTree[locale]}
      nav={{
        title: "Paneflow",
        // Pill `LanguageSwitcher` (same component as the marketing
        // footer) injected into <DocsLayout>'s top nav via the
        // `nav.children` slot. We deliberately do NOT use Fumadocs's
        // `slots.languageToggle` / `<LanguageToggle>` so the docs
        // surface stays visually coherent with the rest of the site
        // (same globe icon, native autoglossonyms, dropdown-up
        // behavior, design tokens). Active locale comes from
        // `useLocale()` inside the switcher, so switching from
        // /fr/docs/installation to /docs/installation preserves the
        // slug via next-intl's `router.replace(pathname, { locale })`.
        children: <LanguageSwitcher variant="pill" />,
      }}
      githubUrl="https://github.com/ArthurDEV44/paneflow"
    >
      <Suspense fallback={null}>
        <SearchUrlSync />
      </Suspense>
      {children}
    </DocsLayout>
  );
}
