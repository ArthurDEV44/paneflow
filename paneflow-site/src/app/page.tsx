import { Navbar } from "@/components/navbar";
import { Hero } from "@/components/hero";
import { StatsStrip } from "@/components/stats-strip";
import { FeatureTriptych } from "@/components/feature-triptych";
import { FeatureSections } from "@/components/feature-sections";
import { Footer } from "@/components/footer";
import { SectionTracker } from "@/components/section-tracker";
import { LATEST_VERSION } from "@/lib/release";

// SoftwareApplication JSON-LD (US-010). softwareVersion sources from
// LATEST_VERSION (single source of truth in src/lib/release.ts) — keep
// this in sync on every release cut; the per-release checklist
// (tasks/seo-launch-checklist.md → US-010) tracks the verification.
// Intentionally omits aggregateRating (no visible reviews on this page —
// would fail Google's visible-reviews precondition) and FAQPage/HowTo
// (not the primary content of this page).
const softwareApplicationSchema = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: "PaneFlow",
  applicationCategory: "DeveloperApplication",
  operatingSystem: "Linux, macOS, Windows",
  url: "https://paneflow.dev",
  downloadUrl: "https://paneflow.dev/download",
  softwareVersion: LATEST_VERSION,
  license: "https://opensource.org/licenses/MIT",
  author: { "@id": "https://paneflow.dev/#organization" },
  offers: {
    "@type": "Offer",
    price: "0",
    priceCurrency: "USD",
  },
};

export default function Home() {
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
        <StatsStrip />
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
