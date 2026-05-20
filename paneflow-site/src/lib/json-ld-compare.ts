import { LATEST_VERSION } from "@/lib/release";

/**
 * JSON-LD builders for comparison pages (`/compare/<x>`). One function
 * returns a single `@graph` payload combining the schemas Google and
 * AI engines reward for comparison content:
 *
 *   - `TechArticle`  : the comparison article itself (headline, dates).
 *   - `SoftwareApplication` (Paneflow) : product entity, linked back to
 *     the canonical `#organization` and `#website` declared in the root
 *     layout. Gives rich-result eligibility.
 *   - `FAQPage`      : every Q/A pair, cited ~3x more in AI Overviews
 *     vs prose-only content (Frase 2026 study).
 *   - `BreadcrumbList` : Home -> Compare -> {Competitor}.
 *
 * Schema.org has no native `Comparison` type; this combination is the
 * canonical pattern for 2026 (see /docs/audit JSON-LD synthesis).
 */

const SITE_URL = "https://paneflow.dev";

export interface CompareJsonLdInput {
  competitorName: string;
  /** e.g. "cmux" - used in URL + breadcrumb. */
  competitorSlug: string;
  /** Visible page title, used as `Article.headline`. */
  headline: string;
  description: string;
  /** ISO date `YYYY-MM-DD` from page metadata. */
  dateModified: string;
  /** Question/Answer pairs for the FAQPage adjunct. */
  faq: Array<{ question: string; answer: string }>;
  /**
   * Optional inspiration link expressed as `Article.isBasedOn`. Use ONLY
   * when the comparison page explicitly credits the competitor as a
   * design inspiration (currently: cmux). Becomes a semantic edge that
   * AI engines can follow to model the relationship between the two
   * projects. Omit on neutral / purely comparative pages.
   */
  isBasedOn?: {
    name: string;
    /** Canonical URL for the source (typically a GitHub repo). */
    url: string;
  };
}

export function buildCompareJsonLd(
  input: CompareJsonLdInput,
): Record<string, unknown> {
  const pageUrl = `${SITE_URL}/compare/${input.competitorSlug}`;
  const iso = toIso8601(input.dateModified);

  const article: Record<string, unknown> = {
    "@type": "TechArticle",
    "@id": `${pageUrl}#article`,
    headline: input.headline,
    description: input.description,
    url: pageUrl,
    inLanguage: "en-US",
    datePublished: iso,
    dateModified: iso,
    image: `${SITE_URL}/opengraph-image`,
    author: {
      "@type": "Person",
      name: "Arthur Jean",
      url: `${SITE_URL}/about`,
    },
    publisher: { "@id": `${SITE_URL}/#organization` },
    isPartOf: { "@id": `${SITE_URL}/#website` },
    mainEntityOfPage: {
      "@type": "WebPage",
      "@id": pageUrl,
    },
  };

  if (input.isBasedOn) {
    article.isBasedOn = {
      "@type": "SoftwareSourceCode",
      name: input.isBasedOn.name,
      url: input.isBasedOn.url,
      codeRepository: input.isBasedOn.url,
    };
  }

  const software: Record<string, unknown> = {
    "@type": "SoftwareApplication",
    "@id": `${pageUrl}#paneflow`,
    name: "Paneflow",
    description:
      "Native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Pure Rust on top of Zed's GPUI engine.",
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Linux, macOS",
    url: `${SITE_URL}/`,
    downloadUrl: `${SITE_URL}/download`,
    softwareVersion: LATEST_VERSION,
    license: "https://opensource.org/licenses/MIT",
    author: { "@id": `${SITE_URL}/#organization` },
    publisher: { "@id": `${SITE_URL}/#organization` },
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
  };

  const faqPage: Record<string, unknown> = {
    "@type": "FAQPage",
    "@id": `${pageUrl}#faq`,
    mainEntity: input.faq.map((entry) => ({
      "@type": "Question",
      name: entry.question,
      acceptedAnswer: {
        "@type": "Answer",
        text: entry.answer,
      },
    })),
  };

  const breadcrumb: Record<string, unknown> = {
    "@type": "BreadcrumbList",
    "@id": `${pageUrl}#breadcrumb`,
    itemListElement: [
      {
        "@type": "ListItem",
        position: 1,
        name: "Home",
        item: `${SITE_URL}/`,
      },
      {
        "@type": "ListItem",
        position: 2,
        name: "Compare",
        item: `${SITE_URL}/compare`,
      },
      {
        "@type": "ListItem",
        position: 3,
        name: input.competitorName,
      },
    ],
  };

  return {
    "@context": "https://schema.org",
    "@graph": [article, software, faqPage, breadcrumb],
  };
}

function toIso8601(date: string): string {
  if (/T\d{2}:\d{2}/.test(date)) return date;
  if (/^\d{4}-\d{2}-\d{2}$/.test(date)) return `${date}T00:00:00Z`;
  return date;
}
