import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({
    locale,
    namespace: "LegalPrivacy.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/legal/privacy", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "website",
      ...buildOpenGraphLocale(locale),
    },
    robots: {
      // Legal pages are not ranking pages — let them be indexed for
      // transparency but avoid thin-content SEO noise. `index: true`
      // is the default; no overrides necessary here.
    },
  };
}

// Last substantive update to the sub-processor list, retention, or
// data-subject rights procedure. Update this date whenever the content
// changes — it is the one piece of mutable copy on this page.
const LAST_UPDATED = "2026-04-23";

const linkClass =
  "text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover";

const RIGHTS_KEYS = ["0", "1", "2", "3", "4", "5", "6"] as const;
const RETENTION_KEYS = ["0", "1", "2"] as const;
const EVENT_KEYS = ["0", "1", "2"] as const;
const SUBPROCESSOR_KEYS = ["0", "1", "2"] as const;

export default async function PrivacyPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("LegalPrivacy");

  // BreadcrumbList JSON-LD (US-011). Intermediate "/legal" omitted per
  // the AC4 logged decision: there is no /legal page yet, and Google
  // warns on non-resolving breadcrumb items. The position-2 label is in
  // French to match the page's content language (matches the <main lang="fr">
  // + hreflang fr-FR signals shipped in US-008).
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
        name: t("schema.breadcrumbPrivacy"),
        item: "https://paneflow.dev/legal/privacy",
      },
    ],
  };

  const strong = (chunks: React.ReactNode) => (
    <strong className="text-text font-semibold">{chunks}</strong>
  );
  const em = (chunks: React.ReactNode) => <em>{chunks}</em>;
  const mono = (chunks: React.ReactNode) => (
    <span className="font-mono text-text">{chunks}</span>
  );
  const email = (chunks: React.ReactNode) => (
    <a href="mailto:arthur.jean@strivex.fr" className={linkClass}>
      {chunks}
    </a>
  );
  const cnil = (chunks: React.ReactNode) => (
    <a href="https://www.cnil.fr/" className={linkClass}>
      {chunks}
    </a>
  );

  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify(breadcrumbSchema),
        }}
      />
      <Navbar />
      <main lang="fr">
        <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
          <div className="max-w-2xl mx-auto px-6">
            <header className="mb-10 sm:mb-12">
              <h1 className="text-3xl sm:text-4xl md:text-5xl">
                {t("heading")}
              </h1>
              <p className="mt-3 text-sm sm:text-base text-text-muted leading-relaxed">
                {t("lastUpdated", { date: LAST_UPDATED })}
              </p>
            </header>

            <div className="space-y-10 text-sm sm:text-base text-text-muted leading-relaxed">
              <Section title={t("sections.controller.title")}>
                <p>{t("sections.controller.intro")}</p>
                <ul className="mt-3 space-y-2">
                  <BulletItem>{t("sections.controller.name")}</BulletItem>
                  <BulletItem>
                    {t("sections.controller.contactLabel")}{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className={linkClass}
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </BulletItem>
                </ul>
              </Section>

              <Section title={t("sections.siteData.title")}>
                <p>{t.rich("sections.siteData.intro", { mono })}</p>
                <ul className="mt-3 space-y-2.5">
                  <BulletItem>
                    {t.rich("sections.siteData.vercel", { strong })}
                  </BulletItem>
                  <BulletItem>
                    {t.rich("sections.siteData.posthog", { strong, em, mono })}
                  </BulletItem>
                </ul>
                <p className="mt-3">{t("sections.siteData.outro")}</p>
              </Section>

              <Section title={t("sections.telemetry.title")}>
                <p>{t.rich("sections.telemetry.p1", { strong })}</p>
                <p className="mt-3">{t("sections.telemetry.p2")}</p>
                <ul className="mt-3 space-y-2">
                  {EVENT_KEYS.map((k) => (
                    <BulletItem key={k}>
                      {t.rich(`sections.telemetry.events.${k}`, { mono })}
                    </BulletItem>
                  ))}
                </ul>
                <p className="mt-3">
                  {t.rich("sections.telemetry.p3", { strong })}
                </p>
                <p className="mt-3">
                  {t.rich("sections.telemetry.p4", { mono })}
                </p>
              </Section>

              <Section title={t("sections.subprocessors.title")}>
                <p>{t("sections.subprocessors.intro")}</p>
                <div className="mt-4 space-y-3">
                  {SUBPROCESSOR_KEYS.map((k) => (
                    <SubProcessor
                      key={k}
                      name={t(`sections.subprocessors.items.${k}.name`)}
                      region={t(`sections.subprocessors.items.${k}.region`)}
                      role={t(`sections.subprocessors.items.${k}.role`)}
                      notes={t(`sections.subprocessors.items.${k}.notes`)}
                      locationLabel={t("sections.subprocessors.locationLabel")}
                      roleLabel={t("sections.subprocessors.roleLabel")}
                    />
                  ))}
                </div>
              </Section>

              <Section title={t("sections.retention.title")}>
                <ul className="space-y-2">
                  {RETENTION_KEYS.map((k) => (
                    <BulletItem key={k}>
                      {t(`sections.retention.items.${k}`)}
                    </BulletItem>
                  ))}
                </ul>
                <p className="mt-3">{t("sections.retention.outro")}</p>
              </Section>

              <Section title={t("sections.rights.title")}>
                <p>{t("sections.rights.intro")}</p>
                <ul className="mt-3 space-y-2.5">
                  {RIGHTS_KEYS.map((k) => (
                    <BulletItem key={k}>
                      {t.rich(`sections.rights.items.${k}`, { strong })}
                    </BulletItem>
                  ))}
                </ul>
                <p className="mt-3">
                  {t.rich("sections.rights.outro", { email, cnil })}
                </p>
              </Section>

              <Section title={t("sections.contact.title")}>
                <p>{t("sections.contact.intro")}</p>
                <ul className="mt-3 space-y-2">
                  <BulletItem>
                    {t("sections.contact.emailLabel")}{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className={linkClass}
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </BulletItem>
                  <BulletItem>
                    {t("sections.contact.publisherLabel")}{" "}
                    {t("sections.contact.publisherName")}
                  </BulletItem>
                </ul>
              </Section>
            </div>
          </div>
        </section>
      </main>
      <Footer />
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

function BulletItem({ children }: { children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="text-text-muted/60 select-none">-</span>
      <span>{children}</span>
    </li>
  );
}

function SubProcessor({
  name,
  region,
  role,
  notes,
  locationLabel,
  roleLabel,
}: {
  name: string;
  region: string;
  role: string;
  notes: string;
  locationLabel: string;
  roleLabel: string;
}) {
  return (
    <div className="rounded-lg border border-surface-border bg-bg-elevated p-4">
      <div className="text-text text-sm font-semibold">{name}</div>
      <div className="mt-1.5 text-sm">
        <span className="text-text-subtle">{locationLabel}</span> {region}
      </div>
      <div className="mt-1 text-sm">
        <span className="text-text-subtle">{roleLabel}</span> {role}
      </div>
      <div className="mt-2 text-sm">{notes}</div>
    </div>
  );
}
