import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Navbar } from "@/components/navbar";
import { Hero } from "@/components/hero";
import { FeatureTriptych } from "@/components/feature-triptych";
import { FeatureSections } from "@/components/feature-sections";
import { Footer } from "@/components/footer";
import { SectionTracker } from "@/components/section-tracker";
import { LATEST_VERSION } from "@/lib/release";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

// Per-locale title/description/openGraph + hreflang alternates for the homepage.
// Without this override the layout's static EN title leaks into /fr and /zh-Hans
// (US-010 flagged this; US-019 fixed it).
export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "HomePage.Metadata" });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      ...buildOpenGraphLocale(locale),
    },
    twitter: {
      title: t("ogTitle"),
      description: t("ogDescription"),
    },
  };
}

// SoftwareApplication JSON-LD (US-010). softwareVersion sources from
// LATEST_VERSION (single source of truth in src/lib/release.ts) - keep
// this in sync on every release cut; the per-release checklist
// (tasks/seo-launch-checklist.md US-010) tracks the verification.
// Intentionally omits aggregateRating (no visible reviews on this page
// would fail Google's visible-reviews precondition) and FAQPage/HowTo
// (not the primary content of this page).

export default async function Home({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("HomePage.schema");

  const softwareApplicationSchema = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Paneflow",
    description: t("description"),
    applicationCategory: "DeveloperApplication",
    applicationSubCategory: "Terminal Multiplexer",
    operatingSystem: "Linux, macOS, Windows",
    url: "https://paneflow.dev",
    downloadUrl: "https://paneflow.dev/download",
    softwareVersion: LATEST_VERSION,
    releaseNotes: `https://github.com/ArthurDEV44/paneflow/releases/tag/v${LATEST_VERSION}`,
    softwareRequirements: t("softwareRequirements"),
    screenshot: "https://paneflow.dev/images/paneflow-hero.png",
    license: "https://opensource.org/licenses/MIT",
    author: { "@id": "https://paneflow.dev/#organization" },
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
  };

  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify(softwareApplicationSchema),
        }}
      />
      <Navbar />
      <main>
        <Hero />
        <div id="features" data-track-section="features">
          <FeatureTriptych />
        </div>
        <FeatureSections />
        <Footer />
      </main>
      <SectionTracker />
    </>
  );
}
