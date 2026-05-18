import type { Metadata } from "next";
import { Suspense } from "react";
import Script from "next/script";
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
  name: "Paneflow",
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
  name: "Paneflow",
  publisher: { "@id": "https://paneflow.dev/#organization" },
  inLanguage: "en-US",
};

export const metadata: Metadata = {
  metadataBase: new URL("https://paneflow.dev"),
  title: "Paneflow - the terminal workspace for Claude Code, Codex & OpenCode",
  description:
    "The terminal workspace built around how you actually work with Claude Code, Codex, and OpenCode. Parallel panes per agent, live branch and dev-server status, session restore. Native Linux and macOS.",
  keywords: [
    "claude code",
    "codex",
    "opencode",
    "coding agent",
    "agentic workflow",
    "ai coding workspace",
    "parallel agents",
    "agent terminal",
    "terminal multiplexer",
    "tmux alternative",
    "rust",
    "linux",
    "macos",
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
    title: "Paneflow - the terminal workspace for Claude Code, Codex & OpenCode",
    description:
      "Run Claude Code, Codex, and OpenCode in parallel panes. Branch and dev-server status per workspace. Native Linux and macOS.",
    type: "website",
    siteName: "Paneflow",
  },
  twitter: {
    card: "summary_large_image",
    title: "Paneflow",
    description:
      "The terminal workspace for Claude Code, Codex, and OpenCode. Parallel panes per agent, scriptable, native.",
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
        {/* Cloudflare Turnstile loader. `lazyOnload` keeps it off the
            critical path - the waitlist form polls window.turnstile and
            renders the widget once the script is ready. Without this
            script, the form falls back to error-state on submit. */}
        <Script
          src="https://challenges.cloudflare.com/turnstile/v0/api.js"
          strategy="lazyOnload"
          async
          defer
        />
      </body>
    </html>
  );
}
