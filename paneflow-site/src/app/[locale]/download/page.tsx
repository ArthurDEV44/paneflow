import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { DownloadView } from "@/components/download/download-view";
import { SectionTracker } from "@/components/section-tracker";
import { LATEST_VERSION, linuxAppImageUrl } from "@/lib/release";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

// SoftwareApplication JSON-LD (US-010). Mirrors src/app/page.tsx but
// adds installUrl for the recommended Linux x86_64 AppImage. The page
// also lists macOS, but schema.org installUrl is singular here, so keep
// the universal Linux artifact as the canonical install URL.
// softwareVersion + installUrl both source from src/lib/release.ts — a
// LATEST_VERSION bump there propagates here automatically.

export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({
    locale,
    namespace: "DownloadPage.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/download", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "website",
      ...buildOpenGraphLocale(locale),
    },
  };
}

export default async function DownloadPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("DownloadPage.schema");

  const softwareApplicationSchema = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Paneflow",
    description: t("description"),
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Linux, macOS, Windows",
    url: "https://paneflow.dev/download",
    downloadUrl: "https://paneflow.dev/download",
    installUrl: linuxAppImageUrl("x86_64"),
    softwareVersion: LATEST_VERSION,
    license: "https://opensource.org/licenses/MIT",
    author: { "@id": "https://paneflow.dev/#organization" },
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
  };

  // BreadcrumbList JSON-LD (US-011). Two-item chain Home → Download.
  const breadcrumbSchema = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      {
        "@type": "ListItem",
        position: 1,
        name: t("breadcrumbHome"),
        item: "https://paneflow.dev",
      },
      {
        "@type": "ListItem",
        position: 2,
        name: t("breadcrumbDownload"),
        item: "https://paneflow.dev/download",
      },
    ],
  };

  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify(softwareApplicationSchema),
        }}
      />
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify(breadcrumbSchema),
        }}
      />
      <Navbar />
      <main>
        <DownloadView />
        <Footer />
      </main>
      <SectionTracker />
    </>
  );
}
