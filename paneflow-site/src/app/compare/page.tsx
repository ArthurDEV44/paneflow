import type { Metadata } from "next";
import Link from "next/link";
import { CompareHeader, CompareLayout } from "@/components/compare/compare-layout";

const SITE_URL = "https://paneflow.dev";

export const metadata: Metadata = {
  title: "Compare Paneflow vs other terminal multiplexers",
  description:
    "Honest side-by-side comparisons of Paneflow against cmux and other terminal workspaces. Architecture, features, pricing, when each is the right pick.",
  alternates: { canonical: "/compare" },
  openGraph: {
    title: "Compare Paneflow vs other terminal workspaces",
    description:
      "Honest comparisons of Paneflow against cmux and other agent-first terminal multiplexers.",
    type: "website",
  },
};

const COMPARISONS: Array<{
  slug: string;
  competitor: string;
  blurb: string;
}> = [
  {
    slug: "cmux",
    competitor: "cmux",
    blurb:
      "macOS-native Swift terminal workspace built on libghostty. Mature (v0.64, 17 500+ stars), feature-rich (embedded browser, SSH daemon, cloud VMs), GPL-3.0 + commercial.",
  },
  {
    slug: "wezterm",
    competitor: "WezTerm",
    blurb:
      "Architectural peer: same Rust + GPU + MIT lineage, different purpose. WezTerm is the highly configurable Lua-scripted terminal (26 k+ stars, eight years, FreeBSD + Windows builds, built-in SSH multiplexer). Paneflow is the agent-first workspace.",
  },
  {
    slug: "iterm2",
    competitor: "iTerm2",
    blurb:
      "macOS veteran (16 years, 17 500+ stars, GPL-2.0) that shipped Claude Code integration + multi-vendor AI chat in v3.7.0beta1 (April 2026). Cross-platform vs macOS-only, MIT vs GPL-2.0, CLI-agent-host vs vendored-chat architecture.",
  },
  {
    slug: "warp",
    competitor: "Warp",
    blurb:
      "Open-sourced in April 2026 (AGPL-3.0 client + MIT UI, OpenAI as founding sponsor). Cloud-leaning with $20-$50 per-user tiers, Free-tier telemetry gate for AI, Oz cloud-agent orchestration. Paneflow is local-first MIT with no login, no telemetry, no tiers.",
  },
];

const breadcrumbJsonLd = {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  "@id": `${SITE_URL}/compare#breadcrumb`,
  itemListElement: [
    { "@type": "ListItem", position: 1, name: "Home", item: `${SITE_URL}/` },
    { "@type": "ListItem", position: 2, name: "Compare" },
  ],
};

const collectionJsonLd = {
  "@context": "https://schema.org",
  "@type": "CollectionPage",
  "@id": `${SITE_URL}/compare#page`,
  name: "Compare Paneflow vs other terminal workspaces",
  url: `${SITE_URL}/compare`,
  isPartOf: { "@id": `${SITE_URL}/#website` },
  hasPart: COMPARISONS.map((c) => ({
    "@type": "WebPage",
    "@id": `${SITE_URL}/compare/${c.slug}`,
    name: `Paneflow vs ${c.competitor}`,
    url: `${SITE_URL}/compare/${c.slug}`,
  })),
};

const graph = {
  "@context": "https://schema.org",
  "@graph": [collectionJsonLd, breadcrumbJsonLd],
};

export default function ComparePage() {
  return (
    <CompareLayout jsonLd={graph}>
      {/* Header — Cursor pattern: short h1 + tagline in a narrow column,
          even though the card grid below spans the full container. */}
      <div className="max-w-3xl mb-12 sm:mb-16">
        <CompareHeader
          title="Compare Paneflow"
          tldr={
            <>
              Side-by-side comparisons of Paneflow against other agent-first
              terminal workspaces. Every page below is written from the source
              code of both projects, with a dedicated &ldquo;When NOT to
              choose Paneflow&rdquo; section so you know exactly when to pick
              the alternative.
            </>
          }
        />
      </div>

      {/* Comparison cards — 2 columns on lg+, stacked on mobile. Each
          card uses the same elevated-bg / rounded-md / p-[18px] language
          as the FeatureSections + FeatureTriptych cards so the visual
          system stays consistent across the site. */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        {COMPARISONS.map((c) => (
          <Link
            key={c.slug}
            href={`/compare/${c.slug}`}
            className="group rounded-md bg-bg-elevated p-[18px] transition-opacity hover:opacity-80"
          >
            <div className="flex items-baseline justify-between gap-4">
              <h2 className="text-xl sm:text-2xl">
                Paneflow vs {c.competitor}
              </h2>
              <span className="text-sm text-text-muted group-hover:text-text transition-colors shrink-0">
                Read &rarr;
              </span>
            </div>
            <p className="mt-3 text-base text-text-muted leading-relaxed">
              {c.blurb}
            </p>
          </Link>
        ))}
      </div>

      <p className="mt-12 sm:mt-16 max-w-3xl text-sm text-text-subtle leading-relaxed">
        Future comparisons under consideration: tmux, zellij, Alacritty.
        Open a{" "}
        <a
          href="https://github.com/ArthurDEV44/paneflow/issues"
          className="text-text underline underline-offset-4 decoration-surface-border-hover"
          rel="noopener noreferrer"
          target="_blank"
        >
          GitHub issue
        </a>{" "}
        if you want to vote on which one ships next.
      </p>
    </CompareLayout>
  );
}
