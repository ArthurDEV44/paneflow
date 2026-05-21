import type { Metadata } from "next";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { DownloadView } from "@/components/download/download-view";
import { SectionTracker } from "@/components/section-tracker";
import { LATEST_VERSION, linuxAppImageUrl } from "@/lib/release";

// SoftwareApplication JSON-LD (US-010). Mirrors src/app/page.tsx but
// adds installUrl for the recommended Linux x86_64 AppImage. The page
// also lists macOS, but schema.org installUrl is singular here, so keep
// the universal Linux artifact as the canonical install URL.
// softwareVersion + installUrl both source from src/lib/release.ts — a
// LATEST_VERSION bump there propagates here automatically.
const softwareApplicationSchema = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: "Paneflow",
  description:
    "A native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Parallel panes, branch-aware workspaces, live dev-server status, session restore, and a JSON-RPC IPC server. Written in pure Rust on top of Zed's GPUI rendering engine.",
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
  title: "Download Paneflow - run Claude Code, Codex, and OpenCode in parallel",
  description:
    "Download Paneflow, the native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents in parallel panes with branch-aware workspaces and session restore. Linux and macOS available now. Windows targeted for Q3 2026.",
  alternates: {
    canonical: "/download",
  },
  openGraph: {
    title: "Download Paneflow - orchestrate Claude Code, Codex, and OpenCode",
    description:
      "The native terminal workspace for running CLI coding agents side by side. Linux and macOS available now. Windows targeted for Q3 2026.",
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
