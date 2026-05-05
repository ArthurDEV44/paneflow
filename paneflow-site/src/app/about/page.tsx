import type { Metadata } from "next";
import Link from "next/link";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { SectionTracker } from "@/components/section-tracker";

export const metadata: Metadata = {
  title: "About PaneFlow — built by Arthur Jean (Strivex)",
  description:
    "Who builds PaneFlow, the GPU-accelerated terminal multiplexer in Rust. Founder bio, project background, and how to get in touch.",
  alternates: {
    canonical: "/about",
  },
  openGraph: {
    title: "About PaneFlow — built by Arthur Jean (Strivex)",
    description:
      "GPU-accelerated terminal multiplexer in Rust. Built by Arthur Jean at Strivex.",
    // og:type "website" matches the root layout. The "profile" type would
    // require og:profile:first_name / og:profile:last_name namespace props
    // which we don't emit; "website" avoids implicit-missing-property
    // warnings from social graph crawlers.
    type: "website",
  },
};

// Person JSON-LD (US-013). @id matches the Organization.founder ref from
// the root layout (US-009) so search engines collapse both Person nodes
// into a single entity. image field intentionally omitted per AC7 — no
// public/arthur.jpg exists; Person schema remains valid without it.
//
// Person.sameAs is intentionally limited to personal-identity URLs per
// schema.org guidance — the Strivex org URL belongs in worksFor (already
// modeled below), not in sameAs. sameAs grows over time:
//   - TODO(OQ-2): add Arthur's dev.to handle once US-015 article publishes.
//   - TODO(OQ-3): add public LinkedIn URL once Arthur confirms he wants it
//     surfaced on /about.
//   - TODO(US-014): add Wikidata Q-number (Person Q# if minted, or skip).
//   - Twitter/X handle is intentionally absent — none currently confirmed.
const personSchema = {
  "@context": "https://schema.org",
  "@type": "Person",
  "@id": "https://paneflow.dev/#founder",
  name: "Arthur Jean",
  url: "https://paneflow.dev/about",
  jobTitle: "Founder, Strivex — building developer tools",
  worksFor: {
    "@type": "Organization",
    name: "Strivex",
    url: "https://strivex.fr",
  },
  sameAs: ["https://github.com/ArthurDEV44"],
};

// BreadcrumbList JSON-LD (extends the US-011 convention to /about). The
// /about route has no intermediate parent in the site IA, so emit only
// Home → About.
const breadcrumbSchema = {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  itemListElement: [
    {
      "@type": "ListItem",
      position: 1,
      name: "Home",
      item: "https://paneflow.dev",
    },
    {
      "@type": "ListItem",
      position: 2,
      name: "About",
      item: "https://paneflow.dev/about",
    },
  ],
};

export default function AboutPage() {
  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(personSchema) }}
      />
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(breadcrumbSchema) }}
      />
      <Navbar />
      <main>
        <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
          <div className="max-w-2xl mx-auto px-6">
            <header className="mb-10 sm:mb-12">
              <h1 className="text-2xl sm:text-3xl font-semibold tracking-tight">
                About Paneflow &amp; the team behind it
              </h1>
            </header>

            <div className="space-y-10 text-sm sm:text-base text-text-muted leading-relaxed">
              <Section title="What Paneflow is">
                <p>
                  Paneflow is a GPU-accelerated terminal multiplexer written
                  in pure Rust on top of Zed&rsquo;s GPUI rendering engine. It
                  splits, organizes, and tracks multiple terminal sessions in
                  one window, with workspaces, per-pane git and dev-server
                  detection, and a JSON-RPC IPC server that lets Claude Code,
                  Codex, and any other tool drive the editor programmatically.
                </p>
                <p className="mt-3">
                  It targets developers who live in the terminal and want a
                  multiplexer that gets out of the way: native window
                  decorations, no Electron, no JavaScript runtime, no input
                  lag. Linux today; macOS and Windows in active porting.
                </p>
              </Section>

              <Section title="Who builds it">
                <p>
                  Paneflow is built by{" "}
                  <strong className="text-text font-semibold">
                    Arthur Jean
                  </strong>
                  , founder of{" "}
                  <a
                    href="https://strivex.fr"
                    className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                  >
                    Strivex
                  </a>
                  . The project started as a cross-platform Rust port of cmux
                  (a Swift-only terminal multiplexer). The goal was to keep
                  the workflow ergonomics while shipping native binaries to
                  every major desktop OS.
                </p>
                <p className="mt-3">
                  Strivex is a small studio focused on developer tooling.
                  Paneflow is its first open-source release. Everything is MIT
                  licensed and built in the open.
                </p>
              </Section>

              <Section title="Links">
                <ul className="space-y-2.5">
                  <LinkItem
                    label="Source code"
                    href="https://github.com/ArthurDEV44/paneflow"
                    text="github.com/ArthurDEV44/paneflow"
                  />
                  <LinkItem
                    label="GitHub profile"
                    href="https://github.com/ArthurDEV44"
                    text="github.com/ArthurDEV44"
                  />
                  <LinkItem
                    label="Studio"
                    href="https://strivex.fr"
                    text="strivex.fr"
                  />
                  <li className="flex gap-3">
                    <span className="text-text-muted/60 select-none">-</span>
                    <span>
                      Privacy:{" "}
                      <Link
                        href="/legal/privacy"
                        className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                      >
                        /legal/privacy
                      </Link>
                    </span>
                  </li>
                </ul>
              </Section>

              <Section title="Location & contact">
                <p>
                  Working from France. For project questions, bug reports, or
                  collaboration:{" "}
                  <a
                    href="mailto:arthur.jean@strivex.fr"
                    className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                  >
                    arthur.jean@strivex.fr
                  </a>
                  .
                </p>
                <p className="mt-3">
                  For bugs and feature requests, opening a GitHub issue is
                  faster:{" "}
                  <a
                    href="https://github.com/ArthurDEV44/paneflow/issues"
                    className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                  >
                    paneflow/issues
                  </a>
                  .
                </p>
              </Section>
            </div>
          </div>
        </section>
      </main>
      <Footer />
      <SectionTracker />
    </>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h2 className="text-base sm:text-lg font-semibold tracking-tight text-text mb-3">
        {title}
      </h2>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

function LinkItem({
  label,
  href,
  text,
}: {
  label: string;
  href: string;
  text: string;
}) {
  return (
    <li className="flex gap-3">
      <span className="text-text-muted/60 select-none">-</span>
      <span>
        {label}:{" "}
        <a
          href={href}
          className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
        >
          {text}
        </a>
      </span>
    </li>
  );
}
