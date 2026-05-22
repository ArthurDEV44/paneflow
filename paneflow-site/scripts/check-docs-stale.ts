#!/usr/bin/env bun
/**
 * check-docs-stale.ts — translation freshness check for content/docs.
 *
 * For every translated MDX file `content/docs/**\/*.<locale>.mdx`, read its
 * `lastSyncedFrom` frontmatter SHA and compare against the current EN
 * source file (`content/docs/**\/*.mdx`). If the EN source has commits
 * past `lastSyncedFrom`, the translation is stale.
 *
 * Outcomes:
 *   - All translations fresh OR zero translated files     -> exit 0
 *   - Any stale / orphan / invalid-SHA translation         -> exit 1
 *   - Missing `lastSyncedFrom` on a translated MDX         -> WARN, not ERROR
 *     (so the first commit of a translated page doesn't break CI)
 *
 * Conventions:
 *   - Locale set is hardcoded (mirrors `scripts/check-translations.ts`).
 *     Adding a locale = a one-line edit here AND in `i18n/routing.ts`.
 *   - Default locale (en) is the source of truth — never gets the
 *     `lastSyncedFrom` field (validated by the schema in source.config.ts).
 *
 * Wired in package.json as `bun run check:docs` (US-008 of
 * prd-fumadocs-docs-i18n.md). Workflow for adding a translated page is
 * documented in AGENTS.md (US-010).
 */
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve, dirname, relative, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const DOCS_DIR = resolve(ROOT, "content/docs");

// Non-default locales. `en` is the source-of-truth, never has translated
// files. Keep in lockstep with `i18n/routing.ts` `routing.locales` minus
// `routing.defaultLocale`.
const TRANSLATED_LOCALES = ["fr", "zh-Hans", "ja", "de", "es"] as const;
type TranslatedLocale = (typeof TRANSLATED_LOCALES)[number];

type Severity = "error" | "warn";
interface Finding {
  severity: Severity;
  locale: TranslatedLocale;
  page: string; // relative to content/docs, e.g. "installation/linux"
  message: string;
}

function readFrontmatterField(filePath: string, field: string): string | null {
  // Minimal YAML extraction: the file MUST start with `---\n`, then key/value
  // lines until a closing `---\n`. We only need a single scalar field, so a
  // tight regex is safer than pulling in gray-matter / js-yaml. Bun build
  // already validates the schema; this is a quick read for the freshness
  // check, not a parser.
  const raw = readFileSync(filePath, "utf-8");
  if (!raw.startsWith("---")) return null;
  const end = raw.indexOf("\n---", 3);
  if (end < 0) return null;
  const block = raw.slice(3, end);
  const re = new RegExp(`^\\s*${field}:\\s*(.+?)\\s*$`, "m");
  const m = block.match(re);
  if (!m) return null;
  // Strip surrounding quotes if present.
  return m[1].replace(/^["']|["']$/g, "");
}

function git(args: string[]): { stdout: string; status: number } {
  const r = spawnSync("git", args, { cwd: ROOT, encoding: "utf-8" });
  return { stdout: (r.stdout ?? "").trim(), status: r.status ?? -1 };
}

function enSourcePath(translatedAbsPath: string, locale: TranslatedLocale): string {
  // e.g. content/docs/installation/linux.fr.mdx -> content/docs/installation/linux.mdx
  return translatedAbsPath.replace(new RegExp(`\\.${locale}\\.mdx$`), ".mdx");
}

function pageId(translatedAbsPath: string, locale: TranslatedLocale): string {
  // "installation/linux.fr.mdx" -> "installation/linux"
  const rel = relative(DOCS_DIR, translatedAbsPath);
  return rel.replace(new RegExp(`\\.${locale}\\.mdx$`), "");
}

function walkMdx(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir)) {
    const abs = join(dir, entry);
    const st = statSync(abs);
    if (st.isDirectory()) walkMdx(abs, out);
    else if (entry.endsWith(".mdx")) out.push(abs);
  }
}

function findTranslatedFiles(): Array<{ abs: string; locale: TranslatedLocale }> {
  const all: string[] = [];
  walkMdx(DOCS_DIR, all);
  const out: Array<{ abs: string; locale: TranslatedLocale }> = [];
  for (const abs of all) {
    for (const locale of TRANSLATED_LOCALES) {
      if (abs.endsWith(`.${locale}.mdx`)) {
        out.push({ abs, locale });
        break;
      }
    }
  }
  return out;
}

function checkOne(
  abs: string,
  locale: TranslatedLocale,
  findings: Finding[],
): void {
  const page = pageId(abs, locale);
  const enAbs = enSourcePath(abs, locale);

  if (!existsSync(enAbs)) {
    findings.push({
      severity: "error",
      locale,
      page,
      message: `orphan translation: EN source ${relative(ROOT, enAbs)} does not exist`,
    });
    return;
  }

  const sha = readFrontmatterField(abs, "lastSyncedFrom");
  if (!sha) {
    findings.push({
      severity: "warn",
      locale,
      page,
      message: "missing `lastSyncedFrom` frontmatter — needs initial sync (run AGENTS.md workflow)",
    });
    return;
  }

  // Verify SHA exists in repo before any comparison. `git cat-file -e <sha>`
  // exits 0 if the object exists, non-zero otherwise. Cheaper than
  // merge-base for the existence check.
  const exists = git(["cat-file", "-e", `${sha}^{commit}`]);
  if (exists.status !== 0) {
    findings.push({
      severity: "error",
      locale,
      page,
      message: `stale or invalid SHA \`${sha}\` (not found in repo — force-pushed branch or typo)`,
    });
    return;
  }

  // Ancestor check: HEAD must be a descendant of (or equal to) the synced
  // SHA. If not, history was rewritten between sync and now.
  const isAncestor = git(["merge-base", "--is-ancestor", sha, "HEAD"]);
  if (isAncestor.status !== 0) {
    findings.push({
      severity: "error",
      locale,
      page,
      message: `stale or invalid SHA \`${sha}\` (not an ancestor of HEAD — history rewritten?)`,
    });
    return;
  }

  // Count commits past the sync SHA that touched the EN source file.
  // If zero, the translation is fresh.
  const enRel = relative(ROOT, enAbs);
  const countResult = git([
    "rev-list",
    "--count",
    `${sha}..HEAD`,
    "--",
    enRel,
  ]);
  const ahead = Number.parseInt(countResult.stdout, 10) || 0;
  if (ahead > 0) {
    const head = git(["rev-parse", "--short", "HEAD"]).stdout || "HEAD";
    const shortSha = sha.length > 7 ? sha.slice(0, 7) : sha;
    findings.push({
      severity: "error",
      locale,
      page,
      message: `stale: EN source has ${ahead} commit(s) past synced SHA ${shortSha} (HEAD=${head}). Re-sync ${enRel} -> ${relative(ROOT, abs)}.`,
    });
  }
}

function main(): void {
  const files = findTranslatedFiles();
  if (files.length === 0) {
    console.log("✓ docs translations up-to-date (0 translated MDX files)");
    process.exit(0);
  }

  const findings: Finding[] = [];
  for (const { abs, locale } of files) {
    checkOne(abs, locale, findings);
  }

  for (const f of findings) {
    const tag = f.severity === "error" ? "ERROR" : "WARN ";
    console.log(`${tag} [${f.locale}] ${f.page}: ${f.message}`);
  }

  const errors = findings.filter((f) => f.severity === "error");
  const warns = findings.filter((f) => f.severity === "warn");
  console.log(
    `\nsummary: ${files.length} translated MDX file(s) checked | ${errors.length} error(s), ${warns.length} warning(s)`,
  );

  if (errors.length === 0) {
    console.log("✓ docs translations up-to-date");
  }
  process.exit(errors.length > 0 ? 1 : 0);
}

try {
  main();
} catch (err) {
  console.error("check-docs-stale: fatal error:", err);
  process.exit(2);
}
