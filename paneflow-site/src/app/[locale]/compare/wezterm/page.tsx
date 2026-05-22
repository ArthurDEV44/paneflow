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

const FAQ_KEYS = ["0", "1", "2", "3", "4", "5", "6"] as const;
const LEFT_BULLET_KEYS = ["0", "1", "2", "3", "4", "5"] as const;
const RIGHT_BULLET_KEYS = ["0", "1", "2", "3", "4", "5"] as const;
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
    namespace: "CompareWezterm.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/compare/wezterm", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "article",
      ...buildOpenGraphLocale(locale),
    },
  };
}

export default async function CompareWeztermPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("CompareWezterm");

  const FAQ = FAQ_KEYS.map((key) => ({
    question: t(`faq.items.${key}.question`),
    answer: t(`faq.items.${key}.answer`),
  }));

  const jsonLd = buildCompareJsonLd({
    competitorName: "WezTerm",
    competitorSlug: "wezterm",
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
        <p>{t("context.p1")}</p>
        <p>{t("context.p2")}</p>
      </CompareSection>

      <CompareSection id="quick-comparison" title={t("quick.title")}>
        <p>{t("quick.intro")}</p>

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          {t("quick.perfHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Cold start", "<200 ms", "not published"],
            ["Keystroke-to-pixel latency", "<4 ms", "not published"],
            [
              "AI agents (dedicated UI buttons)",
              "3 (Claude Code, Codex, OpenCode)",
              "n/a",
            ],
            [
              "AI agents (any CLI)",
              "Unlimited (launch any binary in a pane)",
              "Unlimited via Lua spawn, no first-class UI",
            ],
            ["Branch-aware workspace badges", "Yes", "n/a"],
            ["Dev-server port detection", "Yes", "n/a"],
            [
              "Latest stable release",
              "v0.2.16 (May 2026, active weekly)",
              "20240203-110809 (Feb 2024, 15+ month gap; main is active)",
            ],
            [
              "Workspace + branch persistence",
              "Yes (workspaces, layouts, CWD on disk)",
              "Workspaces concept exists, no branch awareness",
            ],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.coreHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Language", "Rust", "Rust"],
            [
              "GPU stack",
              "GPUI/Blade over Vulkan + Metal",
              "wgpu 25.0.2 over Vulkan / Metal / DX12",
            ],
            ["License", "MIT", "MIT"],
            ["VT emulator", "alacritty_terminal 0.26 (upstream)", "Built-in"],
            ["Pane layout", "N-ary tree, 4 preset layouts", "Tabs + freeform splits"],
            ["Session restore", "Yes (workspaces + CWD)", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + Lua override"],
            ["Mouse selection + hyperlinks", "Yes", "Yes"],
            ["True color / ligatures", "Yes", "Yes"],
            ["Maintainer model", "Indie (Arthur Jean)", "Indie (Wez Furlong, ~98% commits)"],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.configHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Config language", "JSON (static)", "Lua (full scripting surface)"],
            ["Config path", "~/.config/paneflow/paneflow.json", "~/.wezterm.lua"],
            [
              "Event handlers in config",
              "n/a",
              "Yes (Lua callbacks, format-tab-title, mux-startup, etc.)",
            ],
            [
              "Built-in SSH multiplexer",
              "n/a",
              "Yes (SSH: and SSHMUX: domains)",
            ],
            ["GitHub stars", "Small, growing", "26 210 (fetched 2026-05-20)"],
            ["Codebase age", "~1 month (first commit April 2026)", "~8 years (first commit Dec 2017)"],
            ["Platform: Linux", "Yes", "Yes"],
            ["Platform: macOS", "Yes", "Yes"],
            ["Platform: Windows", "Planned", "Yes"],
            ["Platform: FreeBSD", "n/a", "Yes"],
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
        <p>{t("architecture.p3")}</p>
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
            <span>{t.rich("pricing.wezterm", { strong })}</span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="migrating" title={t("migrating.title")}>
        <p>{t("migrating.intro")}</p>
        <ul className="space-y-2.5 text-sm">
          {MIGRATE_BULLET_KEYS.map((k) => (
            <li key={k} className="flex gap-2.5">
              <span className="text-text-muted/60 select-none mt-0.5">-</span>
              <span>
                {t.rich(`migrating.bullets.${k}`, {
                  strong,
                  code: (c) => <code>{c}</code>,
                  em: (c) => <em>{c}</em>,
                  link: (chunks) => (
                    <Link
                      href="/docs/configuration/schema"
                      className="text-text underline underline-offset-4 decoration-surface-border-hover"
                    >
                      {chunks}
                    </Link>
                  ),
                })}
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
                href="https://github.com/wezterm/wezterm"
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
