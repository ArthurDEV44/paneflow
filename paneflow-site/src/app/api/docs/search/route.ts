import { createFromSource } from "fumadocs-core/search/server";
import { source } from "@/lib/source";

export const revalidate = false;
export const dynamic = "force-static";

// `staticGET` emits a pre-built Orama index at build time. The client
// fetches it once on first search dialog open via the `type: "static"`
// adapter in `useDocsSearch`, so the index is never re-fetched between
// queries (in-memory after first download).
export const { staticGET: GET } = createFromSource(source);
