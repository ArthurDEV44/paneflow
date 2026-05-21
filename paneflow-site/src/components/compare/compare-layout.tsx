import type * as React from "react";
import { Footer } from "@/components/footer";
import { Navbar } from "@/components/navbar";
import { SectionTracker } from "@/components/section-tracker";

/**
 * Page scaffold shared by every `/compare/<x>` page. Wraps the
 * Navbar + Footer + SectionTracker so each page only emits its
 * comparison content + JSON-LD.
 *
 * The outer container (max-w-[1440px] + px-6 sm:px-10 lg:px-16) is the
 * same one used by the hero / navbar / feature cards / footer so the
 * comparison pages sit on the same vertical line as the rest of the
 * site. There is no inner column constraint — articles span the full
 * width of the outer container so paragraphs, decision cards and
 * comparison tables all have room to breathe. Pages that want a
 * narrower editorial column for a specific block (e.g. the index page's
 * header) wrap that block themselves with `<div className="max-w-3xl">`.
 */
export function CompareLayout({
  children,
  jsonLd,
}: {
  children: React.ReactNode;
  jsonLd: Record<string, unknown>;
}): React.ReactElement {
  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />
      <Navbar />
      <main>
        <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
          <div className="max-w-[1440px] mx-auto px-6 sm:px-10 lg:px-16">
            {children}
          </div>
        </section>
      </main>
      <Footer />
      <SectionTracker />
    </>
  );
}

/**
 * H1 + TL;DR header. The TL;DR is intentionally one paragraph - AI
 * engines anchor citations on the first ~30% of page content.
 */
export function CompareHeader({
  title,
  tldr,
}: {
  title: string;
  tldr: React.ReactNode;
}): React.ReactElement {
  return (
    <header className="mb-12 sm:mb-14">
      <h1 className="text-3xl sm:text-4xl md:text-5xl">
        {title}
      </h1>
      <p className="mt-5 text-base sm:text-lg text-text-muted leading-relaxed">
        {tldr}
      </p>
    </header>
  );
}

/**
 * H2 section wrapper. `id` doubles as the anchor for in-page links.
 */
export function CompareSection({
  id,
  title,
  children,
}: {
  id: string;
  title: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <section
      id={id}
      className="mb-12 sm:mb-14 scroll-mt-24 text-sm sm:text-base text-text-muted leading-relaxed"
    >
      <h2 className="text-2xl sm:text-3xl text-text mb-5">
        {title}
      </h2>
      <div className="space-y-4">{children}</div>
    </section>
  );
}

/**
 * Side-by-side decision guide. Two cards. Each side lists a tight
 * "choose X if" bullet set. Renders as two columns on sm+, stacked
 * on mobile. AI engines extract self-contained bullets reliably.
 */
export function DecisionGuide({
  left,
  right,
}: {
  left: { heading: string; bullets: string[] };
  right: { heading: string; bullets: string[] };
}): React.ReactElement {
  return (
    <div className="grid sm:grid-cols-2 gap-4">
      <DecisionCard heading={left.heading} bullets={left.bullets} />
      <DecisionCard heading={right.heading} bullets={right.bullets} />
    </div>
  );
}

function DecisionCard({
  heading,
  bullets,
}: {
  heading: string;
  bullets: string[];
}): React.ReactElement {
  return (
    <div className="rounded-lg border border-surface-border p-5 bg-bg-elevated">
      <h3 className="text-sm sm:text-base font-semibold text-text mb-3">
        {heading}
      </h3>
      <ul className="space-y-2.5 text-sm leading-relaxed">
        {bullets.map((bullet) => (
          <li key={bullet} className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>{bullet}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Feature comparison table. HTML <table> (not div grid) so AI engines
 * extract structured data reliably. `headers` are the 3 columns
 * (typically: feature | Paneflow | competitor).
 */
export function CompareTable({
  headers,
  rows,
}: {
  headers: [string, string, string];
  rows: Array<[string, React.ReactNode, React.ReactNode]>;
}): React.ReactElement {
  return (
    <div className="overflow-x-auto -mx-2 sm:mx-0">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="border-b border-surface-border">
            {headers.map((h, i) => (
              <th
                // biome-ignore lint/suspicious/noArrayIndexKey: stable header set
                key={i}
                className="text-left font-semibold text-text px-3 py-2.5"
              >
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr
              // biome-ignore lint/suspicious/noArrayIndexKey: stable row order
              key={i}
              className="border-b border-surface-border/50"
            >
              <td className="px-3 py-2.5 font-medium text-text align-top">
                {row[0]}
              </td>
              <td className="px-3 py-2.5 align-top">{row[1]}</td>
              <td className="px-3 py-2.5 align-top">{row[2]}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * FAQ accordion-style list. Each entry is a Q/A pair. The list mirrors
 * the FAQPage JSON-LD entries one-to-one so the structured data and
 * visible content stay in sync (Google flags markup that diverges from
 * visible content).
 */
export function CompareFaq({
  entries,
}: {
  entries: Array<{ question: string; answer: React.ReactNode }>;
}): React.ReactElement {
  return (
    <div className="space-y-5">
      {entries.map((entry) => (
        <div key={entry.question}>
          <h3 className="text-sm sm:text-base font-semibold text-text mb-2">
            {entry.question}
          </h3>
          <p className="leading-relaxed">{entry.answer}</p>
        </div>
      ))}
    </div>
  );
}
