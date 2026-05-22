import { getTranslations } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";

// Localized 404 for the marketing surface. Triggered by the catch-all
// page at `[locale]/[...rest]/page.tsx` (next-intl's documented
// pattern) so any unknown path inside a valid locale segment renders
// this component under the `[locale]/layout.tsx` chain - which means
// the `setRequestLocale(locale)` call made there is already in effect
// and `getTranslations` resolves to the active locale automatically.
//
// An invalid locale (e.g. `/zz/foo`) is short-circuited by the layout's
// `hasLocale` guard. Without a shared root layout above `[locale]/`,
// Next.js falls back to its built-in 404 wrapper with no translations -
// documented in tasks/prd-i18n-fr-zh-Hans.md US-017 as the accepted
// unhappy path.
export default async function LocaleNotFound() {
  const t = await getTranslations("NotFound");

  return (
    <>
      <Navbar />
      <main className="min-h-[60vh]">
        <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
          <div className="max-w-2xl mx-auto px-6 text-center">
            <p className="text-sm text-text-subtle uppercase tracking-widest">
              404
            </p>
            <h1 className="mt-4 text-3xl sm:text-4xl md:text-5xl">
              {t("title")}
            </h1>
            <p className="mt-6 text-sm sm:text-base text-text-muted leading-relaxed">
              {t("description")}
            </p>
            <div className="mt-10 flex flex-col sm:flex-row gap-3 sm:gap-4 justify-center">
              <Link
                href="/"
                className="inline-flex items-center justify-center px-5 py-2.5 rounded-md bg-text text-bg text-sm font-medium hover:bg-text-muted transition-colors"
              >
                {t("ctaHome")}
              </Link>
              <Link
                href="/docs"
                className="inline-flex items-center justify-center px-5 py-2.5 rounded-md border border-surface-border text-sm font-medium text-text hover:border-surface-border-hover transition-colors"
              >
                {t("ctaDocs")}
              </Link>
            </div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
