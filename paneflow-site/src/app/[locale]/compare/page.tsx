import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import { CompareHeader, CompareLayout } from "@/components/compare/compare-layout";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

const SITE_URL = "https://paneflow.dev";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({
    locale,
    namespace: "ComparePage.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/compare", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "website",
      ...buildOpenGraphLocale(locale),
    },
  };
}

const COMPARISON_SLUGS = ["cmux", "wezterm", "iterm2", "warp"] as const;

export default async function ComparePage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("ComparePage");

  const COMPARISONS = COMPARISON_SLUGS.map((slug) => ({
    slug,
    competitor: t(`comparisons.${slug}.competitor`),
    blurb: t(`comparisons.${slug}.blurb`),
  }));

  const breadcrumbJsonLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    "@id": `${SITE_URL}/compare#breadcrumb`,
    itemListElement: [
      {
        "@type": "ListItem",
        position: 1,
        name: t("schema.breadcrumbHome"),
        item: `${SITE_URL}/`,
      },
      {
        "@type": "ListItem",
        position: 2,
        name: t("schema.breadcrumbCompare"),
      },
    ],
  };

  const collectionJsonLd = {
    "@context": "https://schema.org",
    "@type": "CollectionPage",
    "@id": `${SITE_URL}/compare#page`,
    name: t("schema.collectionName"),
    url: `${SITE_URL}/compare`,
    isPartOf: { "@id": `${SITE_URL}/#website` },
    hasPart: COMPARISONS.map((c) => ({
      "@type": "WebPage",
      "@id": `${SITE_URL}/compare/${c.slug}`,
      name: t("cardTitle", { competitor: c.competitor }),
      url: `${SITE_URL}/compare/${c.slug}`,
    })),
  };

  const graph = {
    "@context": "https://schema.org",
    "@graph": [collectionJsonLd, breadcrumbJsonLd],
  };

  return (
    <CompareLayout jsonLd={graph}>
      {/* Header — Cursor pattern: short h1 + tagline in a narrow column,
          even though the card grid below spans the full container. */}
      <div className="max-w-3xl mb-12 sm:mb-16">
        <CompareHeader title={t("header.title")} tldr={t("header.tldr")} />
      </div>

      {/* Comparison cards — 2 columns on lg+, stacked on mobile. Each
          card uses the same elevated-bg / rounded-md / p-[18px] language
          as the FeatureSections + FeatureTriptych cards so the visual
          system stays consistent across the site. */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        {COMPARISONS.map((c) => (
          <Link
            key={c.slug}
            href={`/compare/${c.slug}`}
            className="group rounded-md bg-bg-elevated p-[18px] transition-opacity hover:opacity-80"
          >
            <div className="flex items-baseline justify-between gap-4">
              <h2 className="text-xl sm:text-2xl">
                {t("cardTitle", { competitor: c.competitor })}
              </h2>
              <span className="text-sm text-text-muted group-hover:text-text transition-colors shrink-0">
                {t("cardCta")}
              </span>
            </div>
            <p className="mt-3 text-base text-text-muted leading-relaxed">
              {c.blurb}
            </p>
          </Link>
        ))}
      </div>

      <p className="mt-12 sm:mt-16 max-w-3xl text-sm text-text-subtle leading-relaxed">
        {t.rich("futureNote", {
          link: (chunks) => (
            <a
              href="https://github.com/ArthurDEV44/paneflow/issues"
              className="text-text underline underline-offset-4 decoration-surface-border-hover"
              rel="noopener noreferrer"
              target="_blank"
            >
              {chunks}
            </a>
          ),
        })}
      </p>
    </CompareLayout>
  );
}
