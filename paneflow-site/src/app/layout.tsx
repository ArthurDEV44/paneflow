import type { Metadata } from "next";
import { Suspense } from "react";
import Script from "next/script";
import { Geist, Geist_Mono, Hanken_Grotesk } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import { RootProvider } from "fumadocs-ui/provider/next";
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

// Hanken Grotesk - neo-grotesque sans, gratuit via Google Fonts. Joue le
// rôle de CursorGothic / Matter (Cursor / Warp) : weight 400 sur les
// headings + letter-spacing tight donne un look éditorial chaud, sans
// la signature "starter Next.js" qu'a Geist seul.
const hankenGrotesk = Hanken_Grotesk({
  variable: "--font-hanken-sans",
  subsets: ["latin"],
  display: "swap",
});

// Organization + WebSite JSON-LD (US-009).
// Hardcoded absolute URLs - these schemas must remain valid even if the site
// is mirrored on a non-canonical host. Maintenance note: any change to founder
// name, GitHub handle, or sameAs links must update this block in the same
// commit. Wikidata entity Q139574816 is already in sameAs (US-014 done).
// LinkedIn / dev.to handle for the founder live on Person.sameAs in
// src/app/about/page.tsx (US-013), NOT here - Organization.sameAs is for
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

// `potentialAction.SearchAction` points at the docs search URL
// (Orama-backed, shipped via `prd-fumadocs-docs.md`). The /docs route
// honors the `?q=<term>` query convention via `SearchUrlSync` in
// `src/app/docs/layout.tsx`, which opens the Fumadocs search dialog
// when the param is present. The previous "no on-site search" rationale
// (US-009) no longer applies. If a sitewide search ever ships, update
// this urlTemplate to point at that route instead.
const websiteSchema = {
  "@context": "https://schema.org",
  "@type": "WebSite",
  "@id": "https://paneflow.dev/#website",
  url: "https://paneflow.dev",
  name: "Paneflow",
  description:
    "A native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Parallel panes, branch-aware workspaces, live dev-server status, session restore, and a JSON-RPC IPC server. Written in pure Rust on top of Zed's GPUI rendering engine.",
  publisher: { "@id": "https://paneflow.dev/#organization" },
  inLanguage: "en-US",
  potentialAction: {
    "@type": "SearchAction",
    target: {
      "@type": "EntryPoint",
      urlTemplate: "https://paneflow.dev/docs?q={search_term_string}",
    },
    "query-input": "required name=search_term_string",
  },
};

export const metadata: Metadata = {
  metadataBase: new URL("https://paneflow.dev"),
  title: "Paneflow - terminal workspace for orchestrating coding agents",
  description:
    "A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Parallel panes, branch-aware workspaces, dev-server status, session restore, and scripting hooks.",
  keywords: [
    "claude code",
    "codex",
    "opencode",
    "coding agent",
    "cli coding agent",
    "agentic workflow",
    "ai coding workspace",
    "agent orchestration",
    "parallel agents",
    "agent terminal",
    "agent sessions",
    "branch-aware workspace",
    "terminal multiplexer",
    "tmux alternative",
    "cmux alternative",
    "rust",
    "linux",
    "macos",
    "windows",
  ],
  alternates: {
    canonical: "/",
  },
  // GSC ownership verification. Token is provided via the
  // NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION env var (see .env.example);
  // when unset, Next.js omits the meta tag entirely - no broken empty tag.
  verification: {
    google: process.env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION,
  },
  openGraph: {
    title: "Paneflow - terminal workspace for orchestrating coding agents",
    description:
      "Supervise Claude Code, Codex, OpenCode, and other CLI agents in parallel panes with branch-aware workspaces and live dev-server status.",
    type: "website",
    siteName: "Paneflow",
  },
  twitter: {
    card: "summary_large_image",
    title: "Paneflow",
    description:
      "A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents.",
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
      className={`${geistSans.variable} ${geistMono.variable} ${hankenGrotesk.variable} antialiased`}
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
            {/* Fumadocs UI RootProvider drives sidebar/search context for
                /docs/* routes. `theme={{ enabled: false }}` defers to the
                project's next-themes ThemeProvider in <Providers> to avoid
                double-wrapping (RootProvider ships next-themes by default). */}
            <RootProvider theme={{ enabled: false }}>
              {children}
            </RootProvider>
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
