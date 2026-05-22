import type { Metadata } from "next";
import type { Locale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import {
  CompareFaq,
  CompareHeader,
  CompareLayout,
  CompareSection,
  CompareTable,
  DecisionGuide,
} from "@/components/compare/compare-layout";
import { buildCompareJsonLd } from "@/lib/json-ld-compare";
import { buildAlternates, buildOpenGraphLocale } from "@/lib/i18n-metadata";

const DATE_MODIFIED = "2026-05-20";

const FAQ_KEYS = ["0", "1", "2", "3", "4", "5", "6", "7"] as const;
const LEFT_BULLET_KEYS = ["0", "1", "2", "3", "4", "5"] as const;
const RIGHT_BULLET_KEYS = ["0", "1", "2", "3", "4", "5", "6"] as const;
const MIGRATE_BULLET_KEYS = ["0", "1", "2", "3"] as const;
const WHEN_NOT_KEYS = ["0", "1", "2", "3", "4"] as const;

export async function generateMetadata({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({
    locale,
    namespace: "CompareWarp.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/compare/warp", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "article",
      ...buildOpenGraphLocale(locale),
    },
  };
}

export default async function CompareWarpPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("CompareWarp");

  const FAQ = FAQ_KEYS.map((key) => ({
    question: t(`faq.items.${key}.question`),
    answer: t(`faq.items.${key}.answer`),
  }));

  const jsonLd = buildCompareJsonLd({
    competitorName: "Warp",
    competitorSlug: "warp",
    headline: t("schema.headline"),
    description: t("schema.description"),
    dateModified: DATE_MODIFIED,
    faq: FAQ,
    locale,
  });

  const strong = (chunks: React.ReactNode) => (
    <strong className="text-text">{chunks}</strong>
  );

  const tldr = t.rich("header.tldr", { strong });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader title={t("header.title")} tldr={tldr} />

      <CompareSection id="context" title={t("context.title")}>
        <p>{t.rich("context.p1", { em: (c) => <em>{c}</em> })}</p>
        <p>
          {t.rich("context.p2", { code: (c) => <code>{c}</code> })}
        </p>
        <p>{t("context.p3")}</p>
      </CompareSection>

      <CompareSection id="quick-comparison" title={t("quick.title")}>
        <p>{t("quick.intro")}</p>

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          {t("quick.licenseHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            [
              "License (client)",
              "MIT throughout",
              "AGPL-3.0 client + MIT for warpui crates",
            ],
            [
              "Backend",
              "n/a (no server)",
              "Closed-source server stays proprietary",
            ],
            ["Login required to use terminal", "No", "No (lifted Nov 2024)"],
            [
              "Telemetry required for AI on Free tier",
              "n/a (no Free tier; no AI service)",
              "Yes (Warp docs verbatim)",
            ],
            ["Per-seat pricing", "n/a", "Build $20/mo, Business $50/user/mo"],
            ["Paid tiers / credit caps", "n/a", "Free 75 cr/mo (150 first 2 mo), Build 1500/mo, Business 1500/user/mo"],
            ["Founding sponsor / commercial backing", "Indie (Arthur Jean)", "Denver Technologies, Inc.; OpenAI founding sponsor of OSS repo"],
            [
              "Embed in closed-source product",
              "Yes (MIT)",
              "warpui crates only (MIT); rest is AGPL-3.0",
            ],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.coreHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            ["Language", "Rust", "Rust"],
            [
              "GPU stack",
              "GPUI/Blade over Vulkan + Metal",
              "Custom Rust + Metal-direct (forked Alacritty grid model, custom renderer)",
            ],
            ["OS support", "Linux + macOS (Windows planned)", "Linux + macOS + Windows GA"],
            ["VT emulator", "alacritty_terminal 0.26 (upstream)", "Forked from Alacritty grid model"],
            ["Pane layout + tabs", "Yes", "Yes"],
            ["Session restore", "Yes", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + YAML override (cloud-synced)"],
            ["Cross-platform breadth", "Linux + macOS now", "Linux + macOS + Windows now"],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.agentHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            [
              "AI agent model",
              "Launches external CLI agents (Claude Code, Codex, OpenCode)",
              "Built-in AI agent + supports launching CLI agents",
            ],
            ["Cloud orchestration layer (parallel server-side agents)", "n/a", "Oz (available from Build $20/mo)"],
            ["Shared team command library", "n/a", "Warp Drive"],
            ["SAML / SSO", "n/a", "Business tier ($50/user/mo)"],
            ["SCIM / team admin console", "n/a", "Business tier"],
            ["Zero Data Retention enforcement", "n/a (no data collected)", "Business tier (team-wide enforced)"],
            ["Shared credits pool", "n/a", "Business tier"],
            ["Commercial support contract", "n/a", "Available via Enterprise tier"],
          ]}
        />

        <p className="text-xs text-text-subtle mt-8 leading-relaxed">
          {t.rich("quick.footnote", { strong })}
        </p>
      </CompareSection>

      <CompareSection id="decision-guide" title={t("decision.title")}>
        <p>{t("decision.intro")}</p>
        <DecisionGuide
          left={{
            heading: t("decision.leftHeading"),
            bullets: LEFT_BULLET_KEYS.map((k) =>
              t(`decision.leftBullets.${k}`),
            ),
          }}
          right={{
            heading: t("decision.rightHeading"),
            bullets: RIGHT_BULLET_KEYS.map((k) =>
              t(`decision.rightBullets.${k}`),
            ),
          }}
        />
      </CompareSection>

      <CompareSection id="architecture" title={t("architecture.title")}>
        <p>
          {t.rich("architecture.p1", {
            strong,
            code: (c) => <code>{c}</code>,
          })}
        </p>
        <p>
          {t.rich("architecture.p2", {
            strong,
            code: (c) => <code>{c}</code>,
          })}
        </p>
        <p>
          {t.rich("architecture.p3", {
            code: (c) => <code>{c}</code>,
          })}
        </p>
      </CompareSection>

      <CompareSection id="pricing" title={t("pricing.title")}>
        <p>{t("pricing.intro")}</p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>{t.rich("pricing.paneflow", { strong })}</span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>{t.rich("pricing.warp", { strong })}</span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="migrating" title={t("migrating.title")}>
        <p>
          {t.rich("migrating.intro", {
            code: (c) => <code>{c}</code>,
          })}
        </p>
        <ul className="space-y-2.5 text-sm">
          {MIGRATE_BULLET_KEYS.map((k) => (
            <li key={k} className="flex gap-2.5">
              <span className="text-text-muted/60 select-none mt-0.5">-</span>
              <span>
                {t.rich(`migrating.bullets.${k}`, { strong })}
              </span>
            </li>
          ))}
        </ul>
      </CompareSection>

      <CompareSection id="when-not" title={t("whenNot.title")}>
        <p>{t("whenNot.intro")}</p>
        <ol className="space-y-3 text-sm">
          {WHEN_NOT_KEYS.map((k, i) => (
            <li key={k} className="flex gap-3">
              <span className="text-text-muted/60 select-none mt-0.5">
                {i + 1}.
              </span>
              <span>{t.rich(`whenNot.items.${k}`, { strong })}</span>
            </li>
          ))}
        </ol>
      </CompareSection>

      <CompareSection id="faq" title={t("faq.title")}>
        <CompareFaq
          entries={FAQ.map(({ question, answer }) => ({
            question,
            answer,
          }))}
        />
      </CompareSection>

      <CompareSection id="next" title={t("next.title")}>
        <p>
          {t.rich("next.body", {
            download: (chunks) => (
              <Link
                href="/download"
                className="text-text underline underline-offset-4 decoration-surface-border-hover"
              >
                {chunks}
              </Link>
            ),
            docs: (chunks) => (
              <Link
                href="/docs"
                className="text-text underline underline-offset-4 decoration-surface-border-hover"
              >
                {chunks}
              </Link>
            ),
            repo: (chunks) => (
              <a
                href="https://github.com/warpdotdev/Warp/tree/master"
                className="text-text underline underline-offset-4 decoration-surface-border-hover"
                rel="noopener noreferrer"
                target="_blank"
              >
                {chunks}
              </a>
            ),
          })}
        </p>
      </CompareSection>
    </CompareLayout>
  );
}
