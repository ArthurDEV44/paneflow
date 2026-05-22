#!/usr/bin/env bun
import { readFileSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const MSG = resolve(ROOT, "messages");

const LOCALES = ["fr", "zh-Hans"] as const;

type Catalog = Record<string, string>;
type Finding = {
  severity: "error" | "warn";
  locale: string;
  key: string;
  message: string;
  sample?: string;
};

function flatten(obj: unknown, prefix = "", out: Catalog = {}): Catalog {
  if (typeof obj === "string") {
    out[prefix] = obj;
  } else if (obj && typeof obj === "object" && !Array.isArray(obj)) {
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      flatten(v, prefix ? `${prefix}.${k}` : k, out);
    }
  }
  return out;
}

function loadCatalog(locale: string): { raw: string; flat: Catalog } {
  const file = resolve(MSG, `${locale}.json`);
  if (!existsSync(file)) throw new Error(`missing ${file}`);
  const raw = readFileSync(file, "utf-8");
  try {
    const parsed = JSON.parse(raw);
    return { raw, flat: flatten(parsed) };
  } catch (e) {
    console.error(`[${locale}] failed to parse JSON: ${(e as Error).message}`);
    process.exit(1);
  }
}

const ALLOW_EQUAL_EN = new Set<string>([
  // Brand and logo
  "Navbar.brand",
  "Navbar.logoAlt",
  "Navbar.github",
  "Footer.brand",
  // Short Latin nav labels accepted as-is in FR
  "Navbar.links.docs",
  "AboutPage.sections.links.studio",
  // Form placeholders / dash glyphs
  "Waitlist.placeholder.email",
  "Download.matrix.placeholders.empty",
  // Platform names (Latin proper nouns)
  "Download.matrix.platforms.macos",
  "Download.matrix.platforms.windows",
  "Download.matrix.platforms.linux",
  // File-format and installer labels (technical Latin tokens; identical across locales)
  "Download.matrix.items.dmgAppleSilicon",
  "Download.matrix.items.appImageX64",
  "Download.matrix.items.appImageArm64",
  "Download.matrix.items.debX64",
  "Download.matrix.items.debArm64",
  "Download.matrix.items.rpmX64",
  "Download.matrix.items.rpmArm64",
  "Download.matrix.items.tarGzX64",
  "Download.matrix.items.tarGzArm64",
  "Download.matrix.items.windowsMsi",
  "Download.matrix.items.wingetInstall",
  // Competitor proper names
  "ComparePage.comparisons.cmux.competitor",
  "ComparePage.comparisons.wezterm.competitor",
  "ComparePage.comparisons.iterm2.competitor",
  "ComparePage.comparisons.warp.competitor",
  // Comparison page titles formatted "Paneflow vs X" — "vs" is borrowed and idiomatic in FR
  "ComparePage.cardTitle",
  "CompareWarp.Metadata.ogTitle",
  "CompareWarp.schema.headline",
  "CompareWarp.header.title",
  "CompareIterm2.Metadata.ogTitle",
  "CompareIterm2.schema.headline",
  "CompareIterm2.header.title",
  "CompareWezterm.Metadata.ogTitle",
  "CompareWezterm.schema.headline",
  "CompareWezterm.header.title",
  "CompareCmux.Metadata.ogTitle",
  "CompareCmux.schema.headline",
  "CompareCmux.header.title",
]);

function isAllowedEqualToEn(key: string): boolean {
  if (ALLOW_EQUAL_EN.has(key)) return true;
  if (key.startsWith("LegalPrivacy.")) return true;
  return false;
}

const FR_BANNED_EN_TOKENS: { pattern: RegExp; label: string }[] = [
  { pattern: /\bWorkspaces?\b/, label: "Workspace(s)" },
  { pattern: /\bPanes?\b/, label: "Pane(s)" },
  { pattern: /\bSplits?\b/, label: "Split(s)" },
  { pattern: /\bDrop-in\b/i, label: "Drop-in" },
  { pattern: /\bGet started\b/i, label: "Get started" },
  { pattern: /\bComing soon\b/i, label: "Coming soon" },
  { pattern: /\bLightweight\b/i, label: "Lightweight" },
  { pattern: /\bCross-platform\b/i, label: "Cross-platform" },
  { pattern: /\bSelf-hosted\b/i, label: "Self-hosted" },
];

const ZH_BANNED_TRANSLITERATIONS = ["派恩弗洛", "派恩弗罗", "板流", "特米诺尔", "多克"];

const findings: Finding[] = [];

const { flat: en } = loadCatalog("en");

for (const locale of LOCALES) {
  const { raw, flat: cat } = loadCatalog(locale);
  const enKeys = new Set(Object.keys(en));
  const locKeys = new Set(Object.keys(cat));

  for (const k of enKeys) {
    if (!locKeys.has(k))
      findings.push({ severity: "error", locale, key: k, message: "missing key" });
  }
  for (const k of locKeys) {
    if (!enKeys.has(k))
      findings.push({ severity: "warn", locale, key: k, message: "extra key (not in en.json)" });
  }

  for (const [k, v] of Object.entries(cat)) {
    if (v === "") {
      findings.push({ severity: "error", locale, key: k, message: "empty string" });
      continue;
    }
    if (v === en[k] && !isAllowedEqualToEn(k)) {
      findings.push({
        severity: "error",
        locale,
        key: k,
        message: "value identical to English source (not in allow-list)",
        sample: v.slice(0, 80),
      });
    }
  }

  if (locale === "fr") {
    for (const [k, v] of Object.entries(cat)) {
      if (k.startsWith("LegalPrivacy.")) continue;
      for (const { pattern, label } of FR_BANNED_EN_TOKENS) {
        if (pattern.test(v)) {
          findings.push({
            severity: "warn",
            locale,
            key: k,
            message: `banned English token: ${label}`,
            sample: v.slice(0, 80),
          });
          break;
        }
      }
    }
  }

  if (locale === "zh-Hans") {
    let cjkChars = 0;
    for (const [k, v] of Object.entries(cat)) {
      if (k.startsWith("LegalPrivacy.")) continue;
      cjkChars += (v.match(/[一-鿿]/g) || []).length;
      const matches = v.match(/[一-鿿][a-zA-Z0-9]|[a-zA-Z0-9][一-鿿]/g);
      if (matches) {
        findings.push({
          severity: "error",
          locale,
          key: k,
          message: `Pangu spacing missing (${matches.length} hit${matches.length > 1 ? "s" : ""}): ${matches.slice(0, 3).join(", ")}`,
          sample: v.slice(0, 80),
        });
      }
    }
    if (cjkChars === 0) {
      findings.push({
        severity: "error",
        locale,
        key: "(file)",
        message: "no CJK characters found in non-LegalPrivacy values",
      });
    }
    for (const b of ZH_BANNED_TRANSLITERATIONS) {
      if (raw.includes(b)) {
        findings.push({
          severity: "error",
          locale,
          key: "(file)",
          message: `banned transliteration in file: ${b}`,
        });
      }
    }
  }
}

const errors = findings.filter((f) => f.severity === "error");
const warns = findings.filter((f) => f.severity === "warn");

for (const f of findings) {
  const tag = f.severity === "error" ? "ERROR" : "WARN ";
  const sample = f.sample ? ` | ${JSON.stringify(f.sample)}` : "";
  console.log(`${tag} [${f.locale}] ${f.key}: ${f.message}${sample}`);
}

console.log(
  `\nsummary: ${Object.keys(en).length} en keys | ${errors.length} error(s), ${warns.length} warning(s)`,
);

process.exit(errors.length > 0 ? 1 : 0);
