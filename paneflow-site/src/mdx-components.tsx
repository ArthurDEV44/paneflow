import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
import { AgentInstall } from "@/components/docs/agent-install";
import { GetStartedDownloads } from "@/components/docs/get-started-downloads";
import { Since, VersionBadge } from "@/components/docs/version-badge";

/**
 * MDX component registry. Spreads fumadocs-ui defaults (Callout, Tabs,
 * Tab, heading anchors, pre/code with copy button, Steps, Accordion,
 * etc.) and adds project-specific components on top.
 *
 * If a key collides, the project-specific entry wins. To override a
 * fumadocs-ui default, add it AFTER the spread.
 */
function buildMDXComponents(components: MDXComponents): MDXComponents {
  return {
    ...defaultMdxComponents,
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
 */
export function useMDXComponents(components: MDXComponents): MDXComponents {
  return buildMDXComponents(components);
}

/**
 * Non-hook helper. Call from Server Components to obtain the merged
 * component map without triggering React's hook conventions.
 */
export function getMDXComponents(
  components: MDXComponents = {},
): MDXComponents {
  return buildMDXComponents(components);
}
