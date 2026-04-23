import { Navbar } from "@/components/navbar";
import { Hero } from "@/components/hero";
import { StatsStrip } from "@/components/stats-strip";
import { FeatureTriptych } from "@/components/feature-triptych";
import { FeatureSections } from "@/components/feature-sections";
import { Footer } from "@/components/footer";
import { SectionTracker } from "@/components/section-tracker";

export default function Home() {
  return (
    <>
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
