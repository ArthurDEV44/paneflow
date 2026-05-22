#!/usr/bin/env bun
/**
 * IndexNow submission script.
 *
 * Pushes the full sitemap URL set to https://api.indexnow.org/indexnow so
 * Bing, Yandex, and Seznam pick up changes within minutes instead of
 * waiting for their crawl cycle. Google does NOT consume IndexNow as of
 * 2026 — for Google, run a manual "URL Inspection" in Search Console
 * post-deploy (see tasks/seo-post-i18n-actions.md § 2.1).
 *
 * Usage:
 *   bun run scripts/indexnow-submit.ts                # submit production
 *   bun run scripts/indexnow-submit.ts --origin=...   # submit any origin
 *   bun run scripts/indexnow-submit.ts --dry-run      # print payload only
 *
 * Key file: public/3c4e40e3217cc107e23f88e8513e7607.txt
 * Reference: https://www.indexnow.org/documentation
 */

const INDEXNOW_KEY = "3c4e40e3217cc107e23f88e8513e7607";
const DEFAULT_ORIGIN = "https://paneflow.dev";
const INDEXNOW_ENDPOINT = "https://api.indexnow.org/indexnow";

interface Args {
  origin: string;
  dryRun: boolean;
}

function parseArgs(argv: string[]): Args {
  let origin = DEFAULT_ORIGIN;
  let dryRun = false;
  for (const arg of argv.slice(2)) {
    if (arg.startsWith("--origin=")) {
      origin = arg.slice("--origin=".length).replace(/\/+$/, "");
    } else if (arg === "--dry-run") {
      dryRun = true;
    } else if (arg === "--help" || arg === "-h") {
      console.log(`Usage: bun run scripts/indexnow-submit.ts [--origin=URL] [--dry-run]`);
      process.exit(0);
    } else {
      console.error(`Unknown arg: ${arg}`);
      process.exit(1);
    }
  }
  return { origin, dryRun };
}

async function fetchSitemap(origin: string): Promise<string> {
  const url = `${origin}/sitemap.xml`;
  const res = await fetch(url, { headers: { "user-agent": "paneflow-indexnow/1.0" } });
  if (!res.ok) {
    throw new Error(`Failed to fetch sitemap at ${url}: HTTP ${res.status}`);
  }
  return res.text();
}

function extractUrls(sitemapXml: string): string[] {
  // Match every <loc>...</loc>. Sitemap entries always carry one loc.
  // Cheap regex parse: sitemap is well-formed Next.js output, not adversarial.
  const out: string[] = [];
  const re = /<loc>([^<]+)<\/loc>/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(sitemapXml)) !== null) {
    out.push(match[1].trim());
  }
  return out;
}

function chunk<T>(arr: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < arr.length; i += size) {
    out.push(arr.slice(i, i + size));
  }
  return out;
}

async function submitChunk(origin: string, urls: string[]): Promise<void> {
  const host = new URL(origin).host;
  const body = {
    host,
    key: INDEXNOW_KEY,
    keyLocation: `${origin}/${INDEXNOW_KEY}.txt`,
    urlList: urls,
  };
  const res = await fetch(INDEXNOW_ENDPOINT, {
    method: "POST",
    headers: { "content-type": "application/json; charset=utf-8" },
    body: JSON.stringify(body),
  });
  // IndexNow returns 200 (accepted), 202 (accepted, processing), or
  // 4xx for bad keys/hosts. 422 means partial acceptance; payload is
  // still queued. Anything else surfaces as an error.
  if (res.status !== 200 && res.status !== 202) {
    const text = await res.text().catch(() => "");
    throw new Error(`IndexNow HTTP ${res.status}: ${text || res.statusText}`);
  }
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv);
  console.log(`[indexnow] origin=${args.origin} dryRun=${args.dryRun}`);

  const xml = await fetchSitemap(args.origin);
  const urls = extractUrls(xml);
  console.log(`[indexnow] discovered ${urls.length} URLs in sitemap`);

  if (urls.length === 0) {
    console.error("[indexnow] sitemap returned 0 URLs — abort");
    process.exit(1);
  }

  // Filter to URLs whose origin matches the submission origin: IndexNow
  // requires host parity between `host` field and every URL in urlList.
  const matching = urls.filter((u) => u.startsWith(args.origin + "/") || u === args.origin);
  const skipped = urls.length - matching.length;
  if (skipped > 0) {
    console.warn(`[indexnow] skipping ${skipped} URLs whose origin != ${args.origin}`);
  }

  // IndexNow caps each request at 10 000 URLs; chunk defensively at 1k.
  const chunks = chunk(matching, 1000);
  console.log(`[indexnow] submitting in ${chunks.length} chunk(s) of <=1000`);

  if (args.dryRun) {
    console.log("[indexnow] dry-run — payload preview:");
    console.log(JSON.stringify({
      host: new URL(args.origin).host,
      key: INDEXNOW_KEY,
      keyLocation: `${args.origin}/${INDEXNOW_KEY}.txt`,
      urlListCount: matching.length,
      sample: matching.slice(0, 5),
    }, null, 2));
    return;
  }

  for (let i = 0; i < chunks.length; i++) {
    process.stdout.write(`[indexnow] chunk ${i + 1}/${chunks.length} (${chunks[i].length} URLs) ... `);
    try {
      await submitChunk(args.origin, chunks[i]);
      console.log("OK");
    } catch (err) {
      console.log("FAIL");
      console.error(err);
      process.exit(1);
    }
  }
  console.log(`[indexnow] done — ${matching.length} URLs submitted to ${INDEXNOW_ENDPOINT}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
