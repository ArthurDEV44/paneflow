import type { Metadata } from "next";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { DownloadView } from "@/components/download/download-view";
import { SectionTracker } from "@/components/section-tracker";

export const metadata: Metadata = {
  title: "Télécharger PaneFlow: Linux, macOS, Windows",
  description:
    "Télécharge PaneFlow, le multiplexeur de terminal GPU‑accéléré écrit en Rust. Disponible pour Linux (.deb, .rpm, AppImage, .tar.gz). macOS et Windows bientôt.",
  openGraph: {
    title: "Télécharger PaneFlow",
    description:
      "Multiplexeur de terminal GPU‑accéléré en Rust. Linux disponible, macOS et Windows bientôt.",
    type: "website",
  },
};

export default function DownloadPage() {
  return (
    <>
      <Navbar />
      <main>
        <DownloadView />
        <Footer />
      </main>
      <SectionTracker />
    </>
  );
}
