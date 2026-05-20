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

      <div className="grid gap-3 sm:gap-4">
        {COMPARISONS.map((c) => (
          <Link
            key={c.slug}
            href={`/compare/${c.slug}`}
            className="group rounded-lg border border-surface-border bg-bg-elevated p-5 sm:p-6 transition-colors hover:bg-bg-subtle"
          >
            <div className="flex items-baseline justify-between gap-4">
              <h2 className="text-base sm:text-lg font-semibold text-text">
                Paneflow vs {c.competitor}
              </h2>
              <span className="text-xs sm:text-sm text-text-subtle group-hover:text-text-muted transition-colors">
                Read &rarr;
              </span>
            </div>
            <p className="mt-2 text-sm text-text-muted leading-relaxed">
              {c.blurb}
            </p>
          </Link>
        ))}
      </div>

      <p className="mt-10 text-xs sm:text-sm text-text-subtle leading-relaxed">
        Comparisons against WezTerm, Warp, and iTerm2 are planned. Open a{" "}
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
