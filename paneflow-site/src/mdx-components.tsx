import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
import type { AnchorHTMLAttributes } from "react";
import { AgentInstall } from "@/components/docs/agent-install";
import { GetStartedDownloads } from "@/components/docs/get-started-downloads";
import { Since, VersionBadge } from "@/components/docs/version-badge";
import { routing } from "@/i18n/routing";
import { localePath } from "@/lib/i18n-metadata";
import type { Locale } from "next-intl";

/**
 * Build a locale-aware anchor override. Internal `/docs/...` hrefs get
 * rewritten to `/<locale>/docs/...` for non-default locales so a FR
 * reader clicking a cross-reference stays inside the FR docs cluster
 * instead of leaking back to `/docs/...` (EN). Default-locale links
 * remain unchanged. External hrefs (http/https/mailto/#) pass through
 * verbatim.
 *
 * This is a server-rendered override because MDX is rendered in the
 * server component tree (`page.url` flows from Fumadocs source-loader).
 * `locale` is baked in at component-construction time by the docs page
 * caller, which lifts it from `params.locale`.
 */
function buildLocaleAwareAnchor(locale: Locale) {
  return function LocaleAwareAnchor(
    props: AnchorHTMLAttributes<HTMLAnchorElement>,
  ) {
    const { href, ...rest } = props;
    if (typeof href === "string" && href.startsWith("/docs")) {
      const rewritten = localePath(locale, href);
      return <a href={rewritten} {...rest} />;
    }
    return <a href={href} {...rest} />;
  };
}

/**
 * MDX component registry. Spreads fumadocs-ui defaults (Callout, Tabs,
 * Tab, heading anchors, pre/code with copy button, Steps, Accordion,
 * etc.) and adds project-specific components on top.
 *
 * If a key collides, the project-specific entry wins. To override a
 * fumadocs-ui default, add it AFTER the spread.
 *
 * `locale` opt-in: when provided, internal `/docs/...` links in the MDX
 * body get rewritten with the locale prefix so cross-references stay
 * inside the active locale cluster (cohesion signal for Google +
 * Perplexity/Claude/ChatGPT citation engines).
 */
function buildMDXComponents(
  components: MDXComponents,
  locale?: Locale,
): MDXComponents {
  const anchorOverride: MDXComponents = locale
    ? { a: buildLocaleAwareAnchor(locale) }
    : {};
  return {
    ...defaultMdxComponents,
    ...anchorOverride,
    ...components,
    VersionBadge,
    Since,
    GetStartedDownloads,
    AgentInstall,
  };
}

/**
 * Next.js convention export. Kept for compatibility - if `@next/mdx` is
 * ever activated alongside fumadocs-mdx, Next will auto-pick this up.
 * No locale rewriting on this path because the legacy `@next/mdx` flow
 * does not provide a locale signal; defaults to identity behaviour.
 */
export function useMDXComponents(components: MDXComponents): MDXComponents {
  return buildMDXComponents(components);
}

/**
 * Non-hook helper. Call from Server Components to obtain the merged
 * component map without triggering React's hook conventions.
 *
 * Pass `locale` (from `params.locale` in a Server Component) to enable
 * locale-aware `/docs/...` link rewriting. Omit to fall back to plain
 * anchors — useful for non-docs surfaces or tests.
 */
export function getMDXComponents(
  components: MDXComponents = {},
  locale?: string,
): MDXComponents {
  // `locale` is typed `string` for caller ergonomics (avoids forcing
  // each Server Component to narrow via hasLocale first); guard here
  // and only enable the rewrite when the value is a known locale.
  const validLocale = locale && (routing.locales as readonly string[]).includes(locale)
    ? (locale as Locale)
    : undefined;
  return buildMDXComponents(components, validLocale);
}
