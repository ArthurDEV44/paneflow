import { Analytics } from "@vercel/analytics/next";
// eslint-disable-next-line no-restricted-imports -- fumadocs-ui migration tracked in tasks/prd-fumadocs-docs.md
import { RootProvider } from "fumadocs-ui/provider/next";
import type { Metadata } from "next";
import Script from "next/script";
import { notFound } from "next/navigation";
import { NextIntlClientProvider, hasLocale, type Locale } from "next-intl";
import { setRequestLocale } from "next-intl/server";
import { Suspense } from "react";
import { PostHogPageView } from "@/components/posthog-pageview";
import { PHProvider } from "@/components/posthog-provider";
import { Providers } from "@/components/providers";
import { routing } from "@/i18n/routing";
import { bcp47ForJsonLd } from "@/lib/i18n-bcp47";
import {
  geistSans,
  geistMono,
  hankenGrotesk,
  notoSansJP,
  notoSansSC,
} from "@/lib/fonts";
import { THEME_INIT_SCRIPT } from "@/lib/theme";
import "../globals.css";

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

// WebSite schema is locale-aware. `inLanguage` is emitted as an array of
// BCP 47 tags covering every locale declared in `routing.locales` — this
// signals to Google + AI engines that the site is fully multilingual,
// rather than implying that only the rendered locale is supported. The
// rendered locale stays the first entry so the strongest signal still
// matches the URL prefix.
function buildWebsiteSchema(locale: Locale): Record<string, unknown> {
  const inLanguage = [
    bcp47ForJsonLd(locale),
    ...routing.locales
      .filter((loc) => loc !== locale)
      .map((loc) => bcp47ForJsonLd(loc)),
  ];
  return {
    "@context": "https://schema.org",
    "@type": "WebSite",
    "@id": "https://paneflow.dev/#website",
    url: "https://paneflow.dev",
    name: "Paneflow",
    description:
      "A native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Parallel panes, branch-aware workspaces, live dev-server status, session restore, and a JSON-RPC IPC server. Written in pure Rust on top of Zed's GPUI rendering engine.",
    publisher: { "@id": "https://paneflow.dev/#organization" },
    inLanguage,
    potentialAction: {
      "@type": "SearchAction",
      target: {
        "@type": "EntryPoint",
        urlTemplate: "https://paneflow.dev/docs?q={search_term_string}",
      },
      "query-input": "required name=search_term_string",
    },
  };
}

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
  verification: {
    google: process.env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION,
  },
  openGraph: {
    title: "Paneflow - terminal workspace for orchestrating coding agents",
    description:
      "Supervise Claude Code, Codex, OpenCode, and other CLI agents in parallel panes with branch-aware workspaces and live dev-server status.",
    type: "website",
    siteName: "Paneflow",
    locale: "en_US",
    alternateLocale: ["fr_FR", "zh_CN", "ja_JP", "de_DE", "es_ES"],
  },
  twitter: {
    card: "summary_large_image",
    title: "Paneflow",
    description:
      "A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents.",
  },
};

export function generateStaticParams() {
  return routing.locales.map((locale) => ({ locale }));
}

export default async function LocaleLayout({
  children,
  params,
}: Readonly<{
  children: React.ReactNode;
  params: Promise<{ locale: string }>;
}>) {
  const { locale } = await params;
  if (!hasLocale(routing.locales, locale)) {
    notFound();
  }
  setRequestLocale(locale);

  const websiteSchema = buildWebsiteSchema(locale);

  // CJK font variables are only attached on the relevant routes so the
  // CJK woff2 chunks and their `<link rel="preload">` tag are skipped
  // entirely on non-CJK locales (LCP guard). The CSS fallback stacks
  // live in src/app/globals.css under `html[lang="zh-Hans"]` and
  // `html[lang="ja"]`. Latin-script locales (en/fr/de/es) use the
  // Geist+Hanken stack only.
  const baseFontVars = `${geistSans.variable} ${geistMono.variable} ${hankenGrotesk.variable}`;
  let fontVariables: string;
  if (locale === "zh-Hans") {
    fontVariables = `${baseFontVars} ${notoSansSC.variable}`;
  } else if (locale === "ja") {
    fontVariables = `${baseFontVars} ${notoSansJP.variable}`;
  } else {
    fontVariables = baseFontVars;
  }

  return (
    <html
      lang={locale}
      data-scroll-behavior="smooth"
      className={`${fontVariables} antialiased`}
      suppressHydrationWarning
    >
      <body className="grain">
        <div
          hidden
          suppressHydrationWarning
          dangerouslySetInnerHTML={{
            __html: `<script>${THEME_INIT_SCRIPT}</script>`,
          }}
        />
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(organizationSchema) }}
        />
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(websiteSchema) }}
        />
        <NextIntlClientProvider>
          <Providers>
            <PHProvider locale={locale}>
              {/*
                `i18n.locale` flows to the default SearchDialog via
                Fumadocs's `useI18n()` -> `useDocsSearch({ locale })`, so
                /fr/docs queries hit the FR Orama index (built per locale
                by `createFromSource(source, { localeMap })` — see
                src/app/api/docs/search/route.ts). Without this the
                dialog passes `locale: undefined` and falls through to
                the default index.

                `search.options` overrides the dialog default
                (`type: "fetch"`, `api: "/api/search"`) to point at our
                static prebuilt index at `/api/docs/search`, which
                `staticGET` from fumadocs-core emits at build time.
              */}
              <RootProvider
                theme={{ enabled: false }}
                i18n={{ locale }}
                search={{
                  options: { type: "static", api: "/api/docs/search" },
                }}
              >
                {children}
              </RootProvider>
              <Suspense fallback={null}>
                <PostHogPageView />
              </Suspense>
              <Analytics />
            </PHProvider>
          </Providers>
        </NextIntlClientProvider>
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
