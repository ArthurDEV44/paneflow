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
    namespace: "CompareIterm2.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/compare/iterm2", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "article",
      ...buildOpenGraphLocale(locale),
    },
  };
}

export default async function CompareIterm2Page({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("CompareIterm2");

  const FAQ = FAQ_KEYS.map((key) => ({
    question: t(`faq.items.${key}.question`),
    answer: t(`faq.items.${key}.answer`),
  }));

  const jsonLd = buildCompareJsonLd({
    competitorName: "iTerm2",
    competitorSlug: "iterm2",
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
        <p>
          {t.rich("context.p1", {
            em: (c) => <em>{c}</em>,
            code: (c) => <code>{c}</code>,
          })}
        </p>
        <p>{t("context.p2")}</p>
      </CompareSection>

      <CompareSection id="quick-comparison" title={t("quick.title")}>
        <p>{t("quick.intro")}</p>

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          {t("quick.portabilityHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["Cold start", "<200 ms", "not published"],
            ["Keystroke-to-pixel latency", "<4 ms", "not published"],
            ["OS support", "Linux + macOS (Windows planned)", "macOS only"],
            ["License", "MIT", "GPL-2.0"],
            [
              "Agent model",
              "Launches CLI agents (Claude Code, Codex, OpenCode) as panes",
              "Vendored multi-vendor chat (OpenAI, Anthropic, Gemini, DeepSeek) + Claude Code session hooks",
            ],
            [
              "AI agents (dedicated UI buttons)",
              "3 (Claude Code, Codex, OpenCode)",
              "Multi-vendor chat panel + Claude Code workgroup mode",
            ],
            ["Branch-aware workspace badges", "Yes", "n/a"],
            ["Dev-server port detection", "Yes", "n/a"],
            [
              "Latest release",
              "v0.2.16 (May 2026, active weekly)",
              "v3.7.0beta1 (April 2026, marked work-in-progress)",
            ],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.coreHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["GPU rendering", "Yes (GPUI/Blade over Vulkan + Metal)", "Yes (Metal-direct via PTYTextView)"],
            ["Language", "Rust", "Hybrid Objective-C + Swift"],
            [
              "VT emulator",
              "alacritty_terminal 0.26 (upstream crate)",
              "Built-in (VT100Parser, VT100Terminal, VT100Screen, VT100Grid)",
            ],
            ["Tabs + splits", "Yes", "Yes"],
            ["Session restore on relaunch", "Yes", "Yes"],
            ["True color, mouse, hyperlinks", "Yes", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + custom color schemes"],
            ["Pricing", "Free (MIT)", "Free (GPL-2.0, donation-supported)"],
            ["Maintainer model", "Indie (Arthur Jean, solo)", "Indie (George Nachman, ~98% commits over 16 years)"],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.macosHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["AppleScript .sdef scripting", "n/a", "Yes"],
            ["Python scripting API", "n/a", "Yes (long-running scripts supported)"],
            [
              "Shell integration (triggers, smart selection, prompt awareness)",
              "n/a",
              "Yes (extensive)",
            ],
            ["Workgroups (session grouping + Code Review mode)", "n/a", "Yes (new in v3.7.0beta1)"],
            [
              "Concurrent instances (independent settings)",
              "n/a",
              "Yes (`--suite=com.iterm2.<id>`)",
            ],
            ["Tab Status system + per-pane status sorting", "n/a", "Yes (new in v3.7)"],
            ["Hotkey window", "n/a", "Yes"],
            ["GitHub stars", "Small, growing", "17 500+ (16 years of accumulation)"],
            ["Codebase age", "~1 month", "~16 years (first commit 2010-07-20)"],
          ]}
        />

        <p className="text-xs text-text-subtle mt-8 leading-relaxed">
          {t.rich("quick.footnote", {
            strong,
            code: (c) => <code>{c}</code>,
          })}
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
            <span>{t.rich("pricing.iterm2", { strong })}</span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="migrating" title={t("migrating.title")}>
        <p>
          {t.rich("migrating.p1", {
            code: (c) => <code>{c}</code>,
          })}
        </p>
        <p>{t("migrating.p2")}</p>
        <ul className="space-y-2.5 text-sm">
          {MIGRATE_BULLET_KEYS.map((k) => (
            <li key={k} className="flex gap-2.5">
              <span className="text-text-muted/60 select-none mt-0.5">-</span>
              <span>{t.rich(`migrating.bullets.${k}`, { strong })}</span>
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
                href="https://github.com/gnachman/iTerm2"
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
