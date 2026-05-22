import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { SectionTracker } from "@/components/section-tracker";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "AboutPage.Metadata" });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/about", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      // og:type "website" matches the root layout. The "profile" type would
      // require og:profile:first_name / og:profile:last_name namespace props
      // which we don't emit; "website" avoids implicit-missing-property
      // warnings from social graph crawlers.
      type: "website",
      ...buildOpenGraphLocale(locale),
    },
  };
}

// Person JSON-LD (US-013). @id matches the Organization.founder ref from
// the root layout (US-009) so search engines collapse both Person nodes
// into a single entity. image field intentionally omitted per AC7: no
// public/arthur.jpg exists; Person schema remains valid without it.
//
// Person.sameAs lists personal-identity URLs per schema.org guidance
// (the Strivex org URL also appears here because it is treated as a
// public-presence reference for Arthur, not only the employer record
// already modeled below via worksFor). A Wikidata Person Q-number is
// not yet minted; when it is, append the Q-URL to this array.

export default async function AboutPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("AboutPage");

  const personSchema = {
    "@context": "https://schema.org",
    "@type": "Person",
    "@id": "https://paneflow.dev/#founder",
    name: "Arthur Jean",
    url: "https://paneflow.dev/about",
    jobTitle: t("schema.jobTitle"),
    worksFor: {
      "@type": "Organization",
      name: "Strivex",
      url: "https://strivex.fr",
    },
    sameAs: [
      "https://github.com/ArthurDEV44",
      "https://www.linkedin.com/in/arthur-jean-strivex/",
      "https://x.com/arthurjdev",
      "https://dev.to/arthurj-dev",
      "https://arthurjean.com",
    ],
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
        name: t("schema.breadcrumbHome"),
        item: "https://paneflow.dev",
      },
      {
        "@type": "ListItem",
        position: 2,
        name: t("schema.breadcrumbAbout"),
        item: "https://paneflow.dev/about",
      },
    ],
  };

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
              <h1 className="text-3xl sm:text-4xl md:text-5xl">
                {t("heading")}
              </h1>
            </header>

            <div className="space-y-10 text-sm sm:text-base text-text-muted leading-relaxed">
              <Section title={t("sections.what.title")}>
                <p>{t("sections.what.p1")}</p>
                <p className="mt-3">{t("sections.what.p2")}</p>
              </Section>

              <Section title={t("sections.who.title")}>
                <p>
                  {t.rich("sections.who.p1", {
                    strong: (chunks) => (
                      <strong className="text-text font-semibold">
                        {chunks}
                      </strong>
                    ),
                    link: (chunks) => (
                      <a
                        href="https://strivex.fr"
                        className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                      >
                        {chunks}
                      </a>
                    ),
                  })}
                </p>
                <p className="mt-3">{t("sections.who.p2")}</p>
              </Section>

              <Section title={t("sections.links.title")}>
                <ul className="space-y-2.5">
                  <LinkItem
                    label={t("sections.links.sourceCode")}
                    href="https://github.com/ArthurDEV44/paneflow"
                    text="github.com/ArthurDEV44/paneflow"
                  />
                  <LinkItem
                    label={t("sections.links.githubProfile")}
                    href="https://github.com/ArthurDEV44"
                    text="github.com/ArthurDEV44"
                  />
                  <LinkItem
                    label={t("sections.links.studio")}
                    href="https://strivex.fr"
                    text="strivex.fr"
                  />
                  <li className="flex gap-3">
                    <span className="text-text-muted/60 select-none">-</span>
                    <span>
                      {t("sections.links.privacyLabel")}{" "}
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

              <Section title={t("sections.contact.title")}>
                <p>
                  {t.rich("sections.contact.p1", {
                    email: (chunks) => (
                      <a
                        href="mailto:arthur.jean@strivex.fr"
                        className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                      >
                        {chunks}
                      </a>
                    ),
                  })}
                </p>
                <p className="mt-3">
                  {t.rich("sections.contact.p2", {
                    link: (chunks) => (
                      <a
                        href="https://github.com/ArthurDEV44/paneflow/issues"
                        className="text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover"
                      >
                        {chunks}
                      </a>
                    ),
                  })}
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
