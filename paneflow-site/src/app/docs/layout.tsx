import { DocsLayout } from "fumadocs-ui/layouts/docs";
import type * as React from "react";
import { source } from "@/lib/source";

/**
 * Docs segment layout. Delegates to fumadocs-ui's <DocsLayout>, which
 * ships the sidebar, header, search trigger, theme toggle, and mobile
 * navigation out of the box. Tree comes from `loader().pageTree` and is
 * generated from `content/docs/...` at build time.
 *
 * `nav.title` is the brand label rendered in the sidebar header.
 * `links` adds top-level navbar items (GitHub link, external CTAs).
 */
export default function Layout({
  children,
}: {
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <DocsLayout
      tree={source.pageTree}
      nav={{ title: "Paneflow" }}
      githubUrl="https://github.com/ArthurDEV44/paneflow"
    >
      {children}
    </DocsLayout>
  );
}
