import { source } from "@/lib/source";

const SITE_URL = "https://paneflow.dev";

const HEADER = [
  "# Paneflow",
  "",
  "> Paneflow is a cross-platform terminal multiplexer for agentic CLI workflows. It runs CLI coding agents like Claude Code, Codex, and OpenCode side by side in branch-aware workspaces with native splits, session restore, and built-in markdown panes on Linux and macOS.",
  "",
  "Site: https://paneflow.dev",
  "Repository: https://github.com/ArthurDEV44/paneflow",
  "",
  "Localization: every URL below has hreflang variants for English (default, no prefix), French (/fr/), Simplified Chinese (/zh-Hans/), German (/de/), Spanish (/es/), and Japanese (/ja/). Translated docs ship progressively per `tasks/prd-docs-i18n-content.md`; locales without a translated MDX file fall back to the English canonical with `<html lang>` matching the locale prefix. Raw Markdown twins are served at the same URL plus `.md` (e.g. https://paneflow.dev/docs/installation/linux.md).",
  "",
] as const;

/**
 * Side-by-side comparison pages against other agent-first terminal
 * workspaces. These pages live OUTSIDE `content/docs/` (they're TSX
 * marketing pages under `src/app/compare/`) so they don't appear in
 * the Fumadocs source loader - we surface them in `llms.txt` manually
 * so AI crawlers asking "Paneflow vs X" can discover the dedicated
 * pages. Add new entries here when shipping new comparison pages.
 */
const COMPARE_ENTRIES: Array<{
  title: string;
  url: string;
  description: string;
}> = [
  {
    title: "Compare Paneflow",
    url: "/compare",
    description:
      "Index of side-by-side comparisons against other agent-first terminal workspaces.",
  },
  {
    title: "Paneflow vs cmux",
    url: "/compare/cmux",
    description:
      "Minimal native Rust workspace (sub-200 ms cold start, Zed's GPUI) vs the kitchen-sink macOS toolkit (libghostty, embedded browser, cloud VMs). Architecture, decision guide, FAQ.",
  },
  {
    title: "Paneflow vs WezTerm",
    url: "/compare/wezterm",
    description:
      "Agent-first Rust workspace vs scriptable Rust terminal emulator. Both Rust, both GPU-accelerated, both MIT, both indie-maintained - they diverge on purpose. WezTerm has Lua scripting, built-in SSH multiplexer, FreeBSD + Windows builds; Paneflow has first-class CLI agent panes and branch-aware workspaces.",
  },
  {
    title: "Paneflow vs iTerm2",
    url: "/compare/iterm2",
    description:
      "Cross-platform MIT agent host vs macOS-only GPL-2.0 veteran with vendored multi-vendor AI chat. Both GPU-accelerated (GPUI/Blade vs Metal-direct). Both ship Claude Code integration today (Paneflow via CLI panes; iTerm2 via session-aware hook protocol in v3.7.0beta1).",
  },
  {
    title: "Paneflow vs Warp",
    url: "/compare/warp",
    description:
      "MIT local-first indie vs AGPL-3.0 cloud-leaning agent platform with $20-$50 per-user tiers. Warp open-sourced April 2026 (OpenAI as founding sponsor); backend stays proprietary, Free-tier AI gated by telemetry. Paneflow has no login, no telemetry, no paid tier, no backend.",
  },
];

interface PageTreeNode {
  type: "page" | "folder" | "separator";
  name?: unknown;
  url?: string;
  description?: unknown;
  children?: PageTreeNode[];
  index?: PageTreeNode;
}

interface SourcePage {
  url: string;
  slugs: string[];
  data: {
    title?: string;
    description?: string;
    /**
     * Fumadocs-MDX runtime API. Returns the processed (JSX-stripped,
     * frontmatter-stripped) Markdown body. Available because
     * `postprocess.includeProcessedMarkdown` is enabled on the docs
     * collection in `source.config.ts`. `"raw"` returns the original
     * file contents.
     */
    getText?: (type: "raw" | "processed") => Promise<string>;
  };
}

function coerceName(name: unknown): string {
  if (typeof name === "string") return name;
  if (name && typeof name === "object" && "toString" in name) {
    const str = (name as { toString: () => string }).toString();
    if (str && str !== "[object Object]") return str;
  }
  return "";
}

function prettifySegment(segment: string): string {
  if (!segment) return "";
  return segment
    .split(/[-_]/g)
    .map((word) =>
      word.length > 0 ? word.charAt(0).toUpperCase() + word.slice(1) : "",
    )
    .join(" ");
}

function absoluteUrl(url: string): string {
  return url.startsWith("http") ? url : `${SITE_URL}${url}`;
}

function formatPageBullet(page: {
  title: string;
  url: string;
  description?: string;
}): string {
  const link = `[${page.title}](${absoluteUrl(page.url)})`;
  const desc = page.description?.trim();
  return desc ? `- ${link}: ${desc}` : `- ${link}`;
}

/**
 * llms.txt - index of every docs page grouped by section.
 *
 * Format follows llmstxt.org spec: H1 + summary blockquote + one H2 per
 * section + bulleted links. Reads exclusively from `source.getPageTree()`
 * so any new MDX file shows up automatically.
 */
export function buildLlmsTxt(): string {
  const tree = source.getPageTree() as unknown as PageTreeNode;
  const children = tree.children ?? [];

  if (children.length === 0) {
    return [...HEADER].join("\n").trimEnd() + "\n";
  }

  const out: string[] = [...HEADER];

  // Root-level orphan pages land in a single Overview section. Folders
  // become H2 sections in document order; nested folders flatten one
  // level (sub-pages bullet under the parent folder's H2).
  const rootPages = children.filter(
    (n): n is PageTreeNode & { type: "page"; url: string } =>
      n.type === "page" && typeof n.url === "string",
  );

  if (rootPages.length > 0) {
    out.push("## Overview");
    out.push("");
    for (const node of rootPages) {
      const page = source.getPage(urlToSlugs(node.url)) as SourcePage | null;
      out.push(
        formatPageBullet({
          title: page?.data.title ?? coerceName(node.name),
          url: node.url,
          description: page?.data.description,
        }),
      );
    }
    out.push("");
  }

  for (const node of children) {
    if (node.type !== "folder") continue;
    const sectionName =
      coerceName(node.name) || prettifySegment(coerceName(node.name));
    out.push(`## ${prettifySegment(sectionName) || sectionName}`);
    out.push("");

    const indexUrl =
      node.index?.type === "page" && typeof node.index.url === "string"
        ? node.index.url
        : null;
    if (indexUrl !== null && node.index) {
      const indexPage = source.getPage(
        urlToSlugs(indexUrl),
      ) as SourcePage | null;
      out.push(
        formatPageBullet({
          title: indexPage?.data.title ?? coerceName(node.index.name),
          url: indexUrl,
          description: indexPage?.data.description,
        }),
      );
    }

    for (const child of node.children ?? []) {
      if (child.type !== "page" || typeof child.url !== "string") continue;
      const page = source.getPage(urlToSlugs(child.url)) as SourcePage | null;
      out.push(
        formatPageBullet({
          title: page?.data.title ?? coerceName(child.name),
          url: child.url,
          description: page?.data.description,
        }),
      );
    }
    out.push("");
  }

  // Manual section for comparison pages (TSX marketing pages, not in
  // the Fumadocs source tree). AI crawlers fetching llms.txt need to
  // discover them or they will not be cited for "Paneflow vs X"
  // queries.
  if (COMPARE_ENTRIES.length > 0) {
    out.push("## Compare");
    out.push("");
    for (const entry of COMPARE_ENTRIES) {
      out.push(formatPageBullet(entry));
    }
    out.push("");
  }

  return out.join("\n").trimEnd() + "\n";
}

/**
 * llms-full.txt - same index as llms.txt but with each page's MDX body
 * inlined under a per-page H2. JSX components are stripped so the output
 * is plain Markdown that LLMs can ingest without parsing custom tags.
 */
export async function buildLlmsFullTxt(): Promise<string> {
  const pages = source.getPages() as SourcePage[];

  if (pages.length === 0) {
    return [...HEADER].join("\n").trimEnd() + "\n";
  }

  const out: string[] = [...HEADER];
  out.push(buildLlmsTxt().slice(HEADER.join("\n").length).trimStart());
  out.push("---");
  out.push("");

  // Iterate pages in URL-sort order for deterministic output. The /docs
  // landing comes first naturally.
  const sorted = [...pages].sort((a, b) => a.url.localeCompare(b.url));

  for (const page of sorted) {
    out.push(`## ${page.data.title ?? page.url}`);
    out.push("");
    out.push(`URL: ${absoluteUrl(page.url)}`);
    if (page.data.description) {
      out.push("");
      out.push(`> ${page.data.description}`);
    }
    out.push("");

    const body = await readPageMarkdown(page);
    if (body) {
      out.push(body);
      out.push("");
    }
    out.push("---");
    out.push("");
  }

  return out.join("\n").trimEnd() + "\n";
}

function urlToSlugs(url: string): string[] {
  // /docs/installation/linux -> ["installation", "linux"]
  // /docs -> []
  return url
    .replace(/^\/docs\/?/, "")
    .split("/")
    .filter(Boolean);
}

/**
 * Read the processed Markdown body for a docs page. Two-pass pipeline:
 *
 *   1. **Fumadocs-MDX `getText("processed")`** (AST-based): strips the
 *      YAML frontmatter, drops top-level `import`/`export` statements,
 *      and preserves fenced code blocks correctly. Activated by
 *      `postprocess.includeProcessedMarkdown` in `source.config.ts`.
 *
 *   2. **Regex JSX strip** (this function): drops the remaining
 *      PascalCase MDX components (`<Callout>`, `<VersionBadge/>`,
 *      `<Since v=…/>`, etc.). Fumadocs's `remarkLLMs` hardcodes its
 *      internal `filterElement` to keep these nodes; the user-facing
 *      config silently overrides any custom filter, so a regex pass
 *      finishes the job. Plain HTML tokens (`< 200 KB`, `<lg`) are
 *      preserved because the pattern only matches PascalCase tag names.
 *
 * Returns `null` only when the loader cannot resolve the processed
 * text (defensive - should never happen for a page that was
 * successfully resolved via `source.getPage()`).
 */
export async function readPageMarkdown(
  page: SourcePage,
): Promise<string | null> {
  if (typeof page.data.getText !== "function") return null;
  try {
    const processed = await page.data.getText("processed");
    return stripJsxComponents(processed).trim();
  } catch {
    return null;
  }
}

function stripJsxComponents(text: string): string {
  // PascalCase JSX components (self-closing): <Component foo={..} />.
  let out = text.replace(/<[A-Z][\w.]*\s*(?:[^>]*?)\/>/g, "");
  // PascalCase JSX opening + closing tags - inner content kept.
  out = out.replace(/<\/?[A-Z][\w.]*\b[^>]*>/g, "");
  // Collapse 3+ consecutive blank lines down to 2.
  return out.replace(/\n{3,}/g, "\n\n");
}
