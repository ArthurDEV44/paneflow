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
    namespace: "CompareCmux.Metadata",
  });
  return {
    title: t("title"),
    description: t("description"),
    alternates: buildAlternates("/compare/cmux", locale),
    openGraph: {
      title: t("ogTitle"),
      description: t("ogDescription"),
      type: "article",
      ...buildOpenGraphLocale(locale),
    },
  };
}

export default async function CompareCmuxPage({
  params,
}: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await params;
  setRequestLocale(locale);
  const t = await getTranslations("CompareCmux");

  const FAQ = FAQ_KEYS.map((key) => ({
    question: t(`faq.items.${key}.question`),
    answer: t(`faq.items.${key}.answer`),
  }));

  const jsonLd = buildCompareJsonLd({
    competitorName: "cmux",
    competitorSlug: "cmux",
    headline: t("schema.headline"),
    description: t("schema.description"),
    dateModified: DATE_MODIFIED,
    faq: FAQ,
    locale,
    // Paneflow's design is openly inspired by cmux (see the "Inspiration"
    // section below). Declaring `Article.isBasedOn` makes that credit
    // machine-readable so AI engines can follow the semantic edge
    // between the two projects (matches the prose acknowledgment in
    // README.md + this page's #inspiration section).
    isBasedOn: {
      name: "cmux",
      url: "https://github.com/manaflow-ai/cmux",
    },
  });

  const strong = (chunks: React.ReactNode) => (
    <strong className="text-text">{chunks}</strong>
  );

  const tldr = t.rich("header.tldr", { strong });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader title={t("header.title")} tldr={tldr} />

      <CompareSection id="inspiration" title={t("inspiration.title")}>
        <p>{t("inspiration.p1")}</p>
        <p>
          {t.rich("inspiration.p2", {
            code: (c) => <code>{c}</code>,
          })}
        </p>
      </CompareSection>

      <CompareSection id="quick-comparison" title={t("quick.title")}>
        <p>{t("quick.intro")}</p>

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          {t("quick.perfHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "cmux"]}
          rows={[
            ["Cold start", "<200 ms", "n/a"],
            ["Keystroke-to-pixel latency", "<4 ms", "n/a"],
            ["Language", "Rust", "Swift (with Go SSH daemon)"],
            ["UI framework", "GPUI (same as Zed)", "AppKit + SwiftUI"],
            [
              "Terminal engine",
              "alacritty_terminal 0.26 (upstream crates.io)",
              "libghostty via GhosttyKit.xcframework",
            ],
            ["GPU stack", "Vulkan / Metal via Blade", "Metal via CAMetalLayer"],
            ["Binary distribution", "Single static binary", "macOS .app bundle"],
            ["License", "MIT", "GPL-3.0-or-later (+ commercial license)"],
            ["OS support", "Linux, macOS (Windows planned)", "macOS only"],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.coreHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "cmux"]}
          rows={[
            ["Workspaces", "Yes", "Yes"],
            ["Pane layout", "N-ary tree, 4 preset layouts", "N-ary tree via Bonsplit"],
            ["Session restore", "Yes (workspaces + CWD)", "Yes"],
            ["Dev-server port detection", "Yes", "Yes"],
            ["Branch-aware workspace badges", "Yes", "Yes"],
            [
              "AI agents (any CLI)",
              "Unlimited (launch any binary in a pane)",
              "Unlimited (launch any binary in a pane)",
            ],
            [
              "AI agents (dedicated UI buttons)",
              "3 (Claude Code, Codex, OpenCode)",
              "15+ (Claude Code, Codex, Grok, OpenCode, Cursor, Copilot, Gemini, Antigravity, Rovo Dev, Hermes, CodeBuddy, Factory, Qoder, Amp, Pi + custom)",
            ],
            ["Markdown panes", "Yes (beta)", "Yes"],
          ]}
        />

        {/* US-005: table cells intentionally not catalogued; remain EN across all locales pending follow-up. */}
        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          {t("quick.toolkitHeading")}
        </h3>
        <CompareTable
          headers={["", "Paneflow", "cmux"]}
          rows={[
            [
              "Embedded browser",
              "n/a",
              "WKWebView with Chrome/Firefox/Safari/Brave/Edge/Arc profile import",
            ],
            ["Cloud VM provisioning", "n/a", "Yes (`cmux vm new`)"],
            ["SSH remote workspaces", "n/a", "Auto-deployed Go daemon over scp/SSH"],
            [
              "IPC surface",
              "JSON-RPC 2.0 over Unix socket (~13 methods)",
              "Dual socket: V1 space-delimited text + V2 newline-delimited JSON, several hundred commands",
            ],
            ["Command palette", "n/a", "Yes (fuzzy-search)"],
            ["AppleScript scripting", "n/a", "Yes (.sdef bundle)"],
            [
              "Tmux compatibility shim",
              "n/a",
              "capture-pane, pipe-pane, bind-key, paste-buffer, set-hook",
            ],
            [
              "Right sidebar panels",
              "Workspaces sidebar only",
              "5 panels: Files, Find, Vault, Feed, Dock",
            ],
            [
              "Per-directory config",
              "n/a",
              "Ancestor-walk with trust modes",
            ],
            ["Notifications", "Basic (per-pane)", "Persistent with unread tracking"],
            [
              "Socket access control modes",
              "Single trust mode",
              "5 modes (off, cmuxOnly, automation, password, allowAll)",
            ],
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
            em: (c) => <em>{c}</em>,
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
            <span>{t.rich("pricing.cmux", { strong })}</span>
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
              <span>
                {t.rich(`whenNot.items.${k}`, {
                  strong,
                  code: (c) => <code>{c}</code>,
                })}
              </span>
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
                href="https://github.com/manaflow-ai/cmux"
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
