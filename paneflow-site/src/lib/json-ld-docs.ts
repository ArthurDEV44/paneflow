import { LATEST_VERSION } from "@/lib/release";

const SITE_URL = "https://paneflow.dev";

export interface PageMeta {
  title: string;
  description?: string;
  url: string;
}

export interface SchemaInput {
  page: PageMeta;
  /**
   * Plain Markdown body of the page with frontmatter and JSX components
   * already stripped (produced by `readPageMarkdown` in `@/lib/docs-llms`,
   * which delegates to Fumadocs-MDX `page.data.getText("processed")`).
   */
  body: string;
}

/**
 * SoftwareApplication schema for the docs index. Mirrors the schema on
 * `/download` (`src/app/download/page.tsx`) but with the docs URL, so
 * search engines can resolve `paneflow.dev/docs` to the product entity
 * when answering "what is Paneflow / how to use Paneflow" queries.
 */
export function buildSoftwareApplicationJsonLd(): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Paneflow",
    description:
      "A native terminal workspace for orchestrating Claude Code, Codex, OpenCode, and other CLI coding agents. Parallel panes, branch-aware workspaces, live dev-server status, session restore, and a JSON-RPC IPC server. Written in pure Rust on top of Zed's GPUI rendering engine.",
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Linux, macOS",
    url: `${SITE_URL}/docs`,
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
}

/**
 * HowTo schema for procedural docs pages. Each `## How do I ...?` H2 in
 * the page body becomes a HowToStep. Returns null when fewer than one
 * step is found, so the caller can decide whether to fall back to no
 * schema rather than emit an empty `step` array.
 */
export function buildHowToJsonLd(input: SchemaInput): Record<string, unknown> | null {
  const steps = extractHowToSteps(input.body);
  if (steps.length === 0) return null;
  return {
    "@context": "https://schema.org",
    "@type": "HowTo",
    name: input.page.title,
    description: input.page.description,
    url: absoluteUrl(input.page.url),
    step: steps.map((step, index) => ({
      "@type": "HowToStep",
      position: index + 1,
      name: step.name,
      text: step.text,
      url: `${absoluteUrl(input.page.url)}#${slugify(step.name)}`,
    })),
  };
}

/**
 * FAQPage schema for the troubleshooting page. Each `### Why ...?` H3
 * becomes a Question + acceptedAnswer pair. Returns null when no
 * questions are found.
 */
export function buildFaqPageJsonLd(
  input: SchemaInput,
): Record<string, unknown> | null {
  const questions = extractFaqQuestions(input.body);
  if (questions.length === 0) return null;
  return {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    url: absoluteUrl(input.page.url),
    mainEntity: questions.map((q) => ({
      "@type": "Question",
      name: q.name,
      acceptedAnswer: {
        "@type": "Answer",
        text: q.text,
      },
    })),
  };
}

/**
 * True when the page should emit HowTo JSON-LD. Either explicit
 * `howto: true` frontmatter OR auto-detection: the body contains 3+
 * `## How do I ...?` H2 headings.
 */
export function shouldEmitHowTo(input: {
  frontmatter: { howto?: boolean };
  body: string;
}): boolean {
  if (input.frontmatter.howto === true) return true;
  const matches = input.body.match(/^##\s+How do I\b/gm);
  return (matches?.length ?? 0) >= 3;
}

interface HowToStep {
  name: string;
  text: string;
}

function extractHowToSteps(body: string): HowToStep[] {
  // Walk lines and partition into sections. A section starts at any H2.
  // Only sections whose H2 starts with "How do I" are kept.
  const lines = body.split(/\r?\n/);
  const sections: { heading: string | null; level: number; content: string[] }[] = [];
  let current: { heading: string | null; level: number; content: string[] } = {
    heading: null,
    level: 0,
    content: [],
  };
  for (const line of lines) {
    const match = line.match(/^(#{1,6})\s+(.*)$/);
    if (match) {
      sections.push(current);
      // Strip trailing ` [#heading-anchor]` that Fumadocs `getText("processed")`
      // appends to every H2/H3 - it pollutes both the visible step name
      // and the slugified anchor URL (which gets double-slugged).
      const heading = match[2]
        .trim()
        .replace(/\s*\[#[\w-]+\]\s*$/, "")
        .trim();
      current = {
        heading,
        level: match[1].length,
        content: [],
      };
    } else {
      current.content.push(line);
    }
  }
  sections.push(current);

  const steps: HowToStep[] = [];
  for (const section of sections) {
    if (section.level !== 2 || !section.heading) continue;
    if (!/^How do I\b/i.test(section.heading)) continue;
    const text = plainifyMarkdown(section.content.join("\n"));
    if (!text) continue;
    steps.push({ name: section.heading, text });
  }
  return steps;
}

interface FaqEntry {
  name: string;
  text: string;
}

function extractFaqQuestions(body: string): FaqEntry[] {
  const lines = body.split(/\r?\n/);
  const sections: { heading: string | null; level: number; content: string[] }[] = [];
  let current: { heading: string | null; level: number; content: string[] } = {
    heading: null,
    level: 0,
    content: [],
  };
  for (const line of lines) {
    const match = line.match(/^(#{1,6})\s+(.*)$/);
    if (match) {
      sections.push(current);
      // Strip trailing ` [#heading-anchor]` that Fumadocs `getText("processed")`
      // appends to every H2/H3 - it pollutes both the visible step name
      // and the slugified anchor URL (which gets double-slugged).
      const heading = match[2]
        .trim()
        .replace(/\s*\[#[\w-]+\]\s*$/, "")
        .trim();
      current = {
        heading,
        level: match[1].length,
        content: [],
      };
    } else {
      current.content.push(line);
    }
  }
  sections.push(current);

  const out: FaqEntry[] = [];
  for (const section of sections) {
    if (section.level !== 3 || !section.heading) continue;
    if (!/^Why\b/i.test(section.heading)) continue;
    const text = plainifyMarkdown(section.content.join("\n"));
    if (!text) continue;
    out.push({ name: section.heading, text });
  }
  return out;
}

function plainifyMarkdown(md: string): string {
  return md
    .replace(/```[\s\S]*?```/g, "") // fenced code blocks
    .replace(/`([^`]+)`/g, "$1") // inline code
    .replace(/\*\*([^*]+)\*\*/g, "$1") // bold
    // Italic only when underscores/asterisks sit at word boundaries -
    // otherwise identifiers like `window_decorations`, `split_horizontally`,
    // and `fish_add_path` would get their underscores eaten by greedy
    // global matching across separate occurrences.
    .replace(/(?<![\w*])\*([^\s*][^*\n]*?)\*(?![\w*])/g, "$1")
    .replace(/(?<![\w_])_([^\s_][^_\n]*?)_(?![\w_])/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1") // markdown links
    .replace(/^\s*[-*+]\s+/gm, "") // list markers
    .replace(/^\s*\d+\.\s+/gm, "") // ordered list markers
    .replace(/^>+\s*/gm, "") // blockquote markers
    .replace(/^\|.*\|$/gm, "") // markdown table rows
    .replace(/^#+\s+.*$/gm, "") // any leftover headings (defensive)
    .replace(/\r?\n{2,}/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function absoluteUrl(url: string): string {
  return url.startsWith("http") ? url : `${SITE_URL}${url}`;
}

/**
 * Normalise a YYYY-MM-DD date string (the form `dateModified` arrives
 * in from the MDX frontmatter Zod transform) into a full ISO 8601
 * datetime. Google's Article structured data validator silently drops
 * `datePublished` / `dateModified` values that aren't full datetimes.
 */
function toIso8601(date?: string): string | undefined {
  if (!date) return undefined;
  if (/T\d{2}:\d{2}/.test(date)) return date; // already a full datetime
  if (/^\d{4}-\d{2}-\d{2}$/.test(date)) return `${date}T00:00:00Z`;
  return date;
}

export interface ArticleInput {
  /** Page title from frontmatter; mapped to `headline`. */
  title: string;
  description?: string;
  /** Relative URL like `/docs/installation/linux`. Absolutised here. */
  url: string;
  /** YYYY-MM-DD or full ISO 8601 - normalised internally. */
  dateModified?: string;
}

/**
 * `TechArticle` schema for leaf documentation pages. Selected over
 * `Article` and `BlogPosting` because the schema.org taxonomy
 * (`Thing > CreativeWork > Article > TechArticle`) signals "technical
 * documentation" more precisely to AI engines (Perplexity, ChatGPT
 * search) while inheriting all Article fields Google indexes.
 *
 * `datePublished` reuses `dateModified` as a fallback - Google validates
 * `datePublished <= dateModified`, so equality is safe. When we later
 * add explicit `datePublished` frontmatter, the caller can pass it
 * separately.
 *
 * Do NOT emit this on the `/docs` root: that page is an index, not an
 * article. Use `SoftwareApplication` there instead.
 */
export function buildTechArticleJsonLd(input: ArticleInput): Record<string, unknown> {
  const url = absoluteUrl(input.url);
  const iso = toIso8601(input.dateModified);
  // Per-page OG image (US-004) served at /api/og/docs/<slug>. Keeping
  // TechArticle.image aligned with og:image avoids the JSON-LD vs
  // OpenGraph divergence where AI engines pick one or the other. For
  // the unlikely input.url that does not start with /docs, fall back to
  // the sitewide OG.
  const ogPath = input.url.startsWith("/docs")
    ? input.url.replace(/^\/docs/, "/api/og/docs")
    : "/opengraph-image";
  const image = `${SITE_URL}${ogPath}`;
  return {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    "@id": `${url}#article`,
    headline: input.title,
    description: input.description,
    url,
    inLanguage: "en-US",
    datePublished: iso,
    dateModified: iso,
    image,
    author: {
      "@type": "Person",
      name: "Arthur Jean",
      "@id": `${SITE_URL}/#founder`,
      url: `${SITE_URL}/about`,
    },
    publisher: { "@id": `${SITE_URL}/#organization` },
    isPartOf: { "@id": `${SITE_URL}/#website` },
    mainEntityOfPage: {
      "@type": "WebPage",
      "@id": url,
    },
  };
}

export interface BreadcrumbCrumb {
  /** Visible label, e.g. "Installation" or "Install Paneflow on Linux". */
  name: string;
  /**
   * Absolute or relative URL. Omit on the LAST crumb only - Google
   * infers `item` from the current page URL.
   */
  url?: string;
}

export interface BreadcrumbInput {
  /** Page URL the breadcrumb sits on; used to mint the `@id`. */
  pageUrl: string;
  crumbs: BreadcrumbCrumb[];
}

/**
 * `BreadcrumbList` schema following Google's Search Central guidance
 * (last updated 2025-12-10). The final `ListItem` omits `item`, which
 * Google explicitly allows - it infers the URL from the current page.
 */
export function buildBreadcrumbListJsonLd(
  input: BreadcrumbInput,
): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    "@id": `${absoluteUrl(input.pageUrl)}#breadcrumb`,
    itemListElement: input.crumbs.map((crumb, index) => {
      const item: Record<string, unknown> = {
        "@type": "ListItem",
        position: index + 1,
        name: crumb.name,
      };
      if (crumb.url) item.item = absoluteUrl(crumb.url);
      return item;
    }),
  };
}

/**
 * Combine multiple JSON-LD payloads into a single `@graph` object. The
 * wrapper carries the `@context`, and each inner payload has its own
 * `@context` stripped so the document validates cleanly. Multiple
 * schemas on one page is the canonical Schema.org pattern - emitting
 * them inside a single `@graph` block beats multiple `<script>` tags
 * for cross-referencing via `@id` and avoids duplicate `@context`.
 */
export function wrapInGraph(
  payloads: Array<Record<string, unknown>>,
): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@graph": payloads.map((payload) => {
      const { "@context": _ctx, ...rest } = payload;
      return rest;
    }),
  };
}

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}
