"use client";

import { useEffect } from "react";
import { useSearchParams } from "next/navigation";
import { useSearchContext } from "fumadocs-ui/contexts/search";

/**
 * Opens the Fumadocs search dialog when `?q=<term>` is present in the URL.
 * Backs the SearchAction declared in `src/app/layout.tsx`'s websiteSchema,
 * which advertises `https://paneflow.dev/docs?q={search_term_string}` as the
 * site search target (Google sitelinks-searchbox + AI search engines).
 *
 * Fumadocs' default SearchDialog does not natively read URL params; this
 * client effect bridges the gap so the declared schema is honest. The query
 * input is opened but not pre-filled (the default dialog exposes only
 * `setOpenSearch(boolean)` on the search context; pre-filling would require
 * a custom SearchDialog override). Users land on /docs with the search
 * dialog open and the query they typed into Google's searchbox visible in
 * the URL, ready to retype or refine.
 */
export function SearchUrlSync(): null {
  const params = useSearchParams();
  const { setOpenSearch } = useSearchContext();

  useEffect(() => {
    const q = params.get("q");
    if (q && q.length > 0) {
      setOpenSearch(true);
    }
  }, [params, setOpenSearch]);

  return null;
}
