import type { Metadata } from "next";
import { Suspense } from "react";
import { Geist, Geist_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import { PHProvider } from "@/components/posthog-provider";
import { PostHogPageView } from "@/components/posthog-pageview";
import { Providers } from "@/components/providers";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

// Organization + WebSite JSON-LD (US-009).
// Hardcoded absolute URLs — these schemas must remain valid even if the site
// is mirrored on a non-canonical host. Maintenance note: any change to founder
// name, GitHub handle, or sameAs links must update this block in the same
// commit. Organization.sameAs will grow over time:
//   - TODO(US-014): add the project Wikidata Q-number once minted
//     (e.g. "https://www.wikidata.org/wiki/Q<NNNNNN>"). Runbook in
//     tasks/seo-launch-checklist.md → "US-014 — Wikidata entity stub".
//   - TODO(US-015): add the dev.to article URL once it has accumulated
//     reactions/comments (per US-015 AC: entity disambiguation signal).
// LinkedIn / dev.to handle for the founder live on Person.sameAs in
// src/app/about/page.tsx (US-013), NOT here — Organization.sameAs is for
// the project entity, not for Arthur personally.
const organizationSchema = {
  "@context": "https://schema.org",
  "@type": "Organization",
  "@id": "https://paneflow.dev/#organization",
  name: "PaneFlow",
  url: "https://paneflow.dev",
  logo: "https://paneflow.dev/logos/paneflow-web-300.png",
  founder: {
    "@type": "Person",
    "@id": "https://paneflow.dev/#founder",
    name: "Arthur Jean",
  },
  sameAs: [
    "https://github.com/ArthurDEV44/paneflow",
    "https://www.wikidata.org/wiki/Q139574816",
  ],
};

// Intentionally omits potentialAction.SearchAction — paneflow.dev has no
// on-site search; declaring it would be a fake feature (AC US-009 #3).
const websiteSchema = {
  "@context": "https://schema.org",
  "@type": "WebSite",
  "@id": "https://paneflow.dev/#website",
  url: "https://paneflow.dev",
  name: "PaneFlow",
  publisher: { "@id": "https://paneflow.dev/#organization" },
  inLanguage: "en-US",
};

export const metadata: Metadata = {
  metadataBase: new URL("https://paneflow.dev"),
  title: "PaneFlow: GPU-accelerated terminal multiplexer",
  description:
    "A terminal multiplexer built in pure Rust with Zed's GPUI framework. Split, organize, and control your terminal. GPU-accelerated.",
  keywords: [
    "terminal",
    "multiplexer",
    "rust",
    "gpui",
    "gpu",
    "linux",
    "tmux",
    "pane",
  ],
  alternates: {
    canonical: "/",
  },
  // GSC ownership verification. Token is provided via the
  // NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION env var (see .env.example);
  // when unset, Next.js omits the meta tag entirely — no broken empty tag.
  verification: {
    google: process.env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION,
  },
  openGraph: {
    title: "PaneFlow: GPU-accelerated terminal multiplexer",
    description:
      "Split, organize, and control your terminal. Built in pure Rust with Zed's rendering engine.",
    type: "website",
    siteName: "PaneFlow",
  },
  twitter: {
    card: "summary_large_image",
    title: "PaneFlow",
    description:
      "GPU-accelerated terminal multiplexer in Rust + Zed's GPUI framework.",
    // twitter:image is auto-injected by src/app/twitter-image.tsx.
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      data-scroll-behavior="smooth"
      className={`${geistSans.variable} ${geistMono.variable} antialiased`}
      suppressHydrationWarning
    >
      <body className="grain">
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(organizationSchema) }}
        />
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(websiteSchema) }}
        />
        <Providers>
          <PHProvider>
            {children}
            <Suspense fallback={null}>
              <PostHogPageView />
            </Suspense>
            <Analytics />
          </PHProvider>
        </Providers>
      </body>
    </html>
  );
}
