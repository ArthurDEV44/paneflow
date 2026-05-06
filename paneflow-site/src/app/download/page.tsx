import type { Metadata } from "next";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { DownloadView } from "@/components/download/download-view";
import { SectionTracker } from "@/components/section-tracker";
import { LATEST_VERSION, linuxAppImageUrl } from "@/lib/release";

// SoftwareApplication JSON-LD (US-010). Mirrors src/app/page.tsx but
// adds installUrl for the recommended Linux x86_64 AppImage — the only
// OS currently shipping. macOS/Windows binaries are not yet listed on
// this page (heading: "PaneFlow est disponible pour Linux. macOS et
// Windows bientôt"), so emitting installUrl for them would be a fake
// feature. Append to installUrl as additional OS binaries ship.
// softwareVersion + installUrl both source from src/lib/release.ts — a
// LATEST_VERSION bump there propagates here automatically.
const softwareApplicationSchema = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: "PaneFlow",
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
      name: "Home",
      item: "https://paneflow.dev",
    },
    {
      "@type": "ListItem",
      position: 2,
      name: "Download",
      item: "https://paneflow.dev/download",
    },
  ],
};

export const metadata: Metadata = {
  title: "Télécharger PaneFlow: Linux, macOS, Windows",
  description:
    "Télécharge PaneFlow, le multiplexeur de terminal GPU‑accéléré écrit en Rust. Disponible pour Linux (.deb, .rpm, AppImage, .tar.gz) et macOS (.dmg signé + notarisé, Apple Silicon). Windows bientôt.",
  alternates: {
    canonical: "/download",
  },
  openGraph: {
    title: "Télécharger PaneFlow",
    description:
      "Multiplexeur de terminal GPU‑accéléré en Rust. Linux et macOS disponibles, Windows bientôt.",
    type: "website",
  },
};

export default function DownloadPage() {
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
