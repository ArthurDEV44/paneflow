import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
} from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import type { Metadata } from "next";
import type * as React from "react";
import { LlmActions } from "@/components/docs/llm-actions";
import { readPageMarkdown } from "@/lib/docs-llms";
import {
  type BreadcrumbCrumb,
  buildBreadcrumbListJsonLd,
  buildFaqPageJsonLd,
  buildHowToJsonLd,
  buildSoftwareApplicationJsonLd,
  buildTechArticleJsonLd,
  shouldEmitHowTo,
  wrapInGraph,
} from "@/lib/json-ld-docs";
import { source } from "@/lib/source";
import { getMDXComponents } from "@/mdx-components";

const SITE_URL = "https://paneflow.dev";

export const dynamicParams = false;

export function generateStaticParams(): Array<{ slug: string[] }> {
  return source.generateParams().map((p) => ({ slug: p.slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug?: string[] }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const page = source.getPage(slug);
  if (!page) return {};
  const title = page.data.title ?? "Paneflow Documentation";
  const description = page.data.description;
  return { title, description };
}

export default async function Page({
  params,
}: {
  params: Promise<{ slug?: string[] }>;
}): Promise<React.ReactElement> {
  const { slug } = await params;
  const page = source.getPage(slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const components = getMDXComponents();
  const pageMarkdown = (await readPageMarkdown(page)) ?? "";
  const jsonLdGraph = buildJsonLdForPage(page, pageMarkdown);

  return (
    <DocsPage
      toc={page.data.toc}
      full={page.data.full}
      tableOfContent={{ style: "clerk" }}
    >
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLdGraph) }}
      />
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <LlmActions
        markdown={pageMarkdown}
        pageUrl={`${SITE_URL}${page.url}`}
        pagePath={page.url}
      />
      <DocsBody>
        <MDX components={components} />
      </DocsBody>
    </DocsPage>
  );
}

/**
 * Decide which JSON-LD schemas apply to the current page and return a
 * single `@graph`-wrapped payload. One `<script type="application/ld+json">`
 * tag per page - distinct schemas cross-reference each other via `@id`
 * inside the graph.
 *
 * Routing rules:
 *   - `/docs` (index) -> SoftwareApplication + BreadcrumbList(Home -> Docs)
 *   - `/docs/<leaf>` -> TechArticle + BreadcrumbList + (optional) FAQPage + (optional) HowTo
 *   - `faqpage: true` frontmatter -> FAQPage adjunct
 *   - `howto: true` OR 3+ `## How do I ...?` H2s -> HowTo adjunct
 */
function buildJsonLdForPage(
  page: {
    url: string;
    slugs: string[];
    data: {
      title?: string;
      description?: string;
      dateModified?: string;
      howto?: boolean;
      faqpage?: boolean;
    };
  },
  body: string,
): Record<string, unknown> {
  const payloads: Array<Record<string, unknown>> = [];

  if (page.url === "/docs") {
    payloads.push(buildSoftwareApplicationJsonLd());
    payloads.push(
      buildBreadcrumbListJsonLd({
        pageUrl: page.url,
        crumbs: [
          { name: "Home", url: "/" },
          { name: "Docs" },
        ],
      }),
    );
    return wrapInGraph(payloads);
  }

  const frontmatter = page.data;
  const pageMeta = {
    title: frontmatter.title ?? "",
    description: frontmatter.description,
    url: page.url,
  };

  // TechArticle - always emitted on leaf docs pages. AI engines key on
  // `@type` for content classification; `TechArticle` outperforms bare
  // `Article` for technical documentation queries.
  payloads.push(
    buildTechArticleJsonLd({
      title: pageMeta.title,
      description: pageMeta.description,
      url: page.url,
      dateModified: frontmatter.dateModified,
    }),
  );

  // BreadcrumbList - reconstructed from page.slugs. Intermediate crumbs
  // use a prettified segment name ("Installation") rather than the index
  // page title ("Install Paneflow") for a tighter visual ladder.
  payloads.push(
    buildBreadcrumbListJsonLd({
      pageUrl: page.url,
      crumbs: buildBreadcrumbCrumbs(page.slugs, pageMeta.title),
    }),
  );

  if (body && frontmatter.faqpage === true) {
    const faq = buildFaqPageJsonLd({ page: pageMeta, body });
    if (faq) payloads.push(faq);
  }

  if (body && shouldEmitHowTo({ frontmatter: { howto: frontmatter.howto }, body })) {
    const howto = buildHowToJsonLd({ page: pageMeta, body });
    if (howto) payloads.push(howto);
  }

  return wrapInGraph(payloads);
}

/**
 * Build the BreadcrumbList ladder for a docs leaf page from its slug
 * array. Intermediate folder crumbs use a prettified segment label
 * ("Installation", "Configuration") rather than the index page title
 * - keeps the breadcrumb tight and avoids duplicating words with the
 * leaf title ("Install Paneflow" / "Install Paneflow on Linux").
 *
 * Final crumb is the current page title with no `item` URL - Google
 * infers it from the page itself.
 */
function buildBreadcrumbCrumbs(
  slugs: string[],
  leafTitle: string,
): BreadcrumbCrumb[] {
  const crumbs: BreadcrumbCrumb[] = [
    { name: "Home", url: "/" },
    { name: "Docs", url: "/docs" },
  ];
  for (let i = 0; i < slugs.length - 1; i++) {
    const slug = slugs[i];
    crumbs.push({
      name: prettifySegment(slug),
      url: `/docs/${slugs.slice(0, i + 1).join("/")}`,
    });
  }
  crumbs.push({ name: leafTitle });
  return crumbs;
}

function prettifySegment(segment: string): string {
  return segment
    .split(/[-_]/g)
    .map((word) =>
      word.length > 0 ? word.charAt(0).toUpperCase() + word.slice(1) : "",
    )
    .join(" ");
}
