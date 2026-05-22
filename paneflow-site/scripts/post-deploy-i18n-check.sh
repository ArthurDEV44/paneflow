#!/usr/bin/env bash
# Post-deploy verification for the paneflow.dev i18n rollout (US-020).
# Exits 0 on full pass, non-zero on any failure. Stdout is human-readable;
# stderr captures individual failure details for grep-ability in CI.
#
# Usage:
#   bash scripts/post-deploy-i18n-check.sh                       # defaults to https://paneflow.dev
#   bash scripts/post-deploy-i18n-check.sh https://paneflow.dev
#   bash scripts/post-deploy-i18n-check.sh https://<preview>.vercel.app
#
# Companion file: tasks/i18n-rollout-runbook.md § 2
# PRD reference:  tasks/prd-i18n-fr-zh-Hans.md US-020 1-hour AC

set -u
ORIGIN="${1:-https://paneflow.dev}"
ORIGIN="${ORIGIN%/}"
FAIL=0
PASS=0

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
gray()  { printf '\033[90m%s\033[0m\n' "$*"; }

pass() { green "  PASS  $1"; PASS=$((PASS+1)); }
fail() { red   "  FAIL  $1"; FAIL=$((FAIL+1)); echo "FAIL: $1" >&2; }

header() { echo; echo "=== $1 ==="; }

# Force HTTP/1.1 so the parser stays simple; Vercel speaks both.
CURL=(curl -sS --max-time 20 --http1.1 -L)

curl_status_path() {
  # $1=path, returns http status of HEAD-equivalent (GET with -I via curl is HEAD).
  local path="$1"
  "${CURL[@]}" -o /dev/null -w '%{http_code}' "${ORIGIN}${path}"
}

curl_redirect_target() {
  # Get the first Location header on a non-redirected HEAD request.
  local path="$1"
  curl -sS -I --max-time 20 --http1.1 -H 'Accept-Encoding: identity' "${ORIGIN}${path}" \
    | awk 'BEGIN{IGNORECASE=1} /^location:/ {sub(/^location:[ \t]*/,"",$0); print; exit}' \
    | tr -d '\r\n'
}

curl_status_code() {
  # First status line code of HEAD; for redirect detection.
  local path="$1"
  shift
  curl -sS -I --max-time 20 --http1.1 -H 'Accept-Encoding: identity' "$@" "${ORIGIN}${path}" \
    | awk '/^HTTP/ {print $2; exit}'
}

html_lang() {
  local path="$1"
  "${CURL[@]}" "${ORIGIN}${path}" | grep -oE '<html[^>]*lang="[^"]+"' | head -1 \
    | sed -E 's/.*lang="([^"]+)".*/\1/'
}

inlang_set() {
  local path="$1"
  "${CURL[@]}" "${ORIGIN}${path}" | grep -oE '"inLanguage":"[^"]+"' | sort -u | tr '\n' ',' | sed 's/,$//'
}

count_hreflang_in_sitemap() {
  local code="$1"
  curl -sS --max-time 30 --http1.1 "${ORIGIN}/sitemap.xml" | grep -c "hreflang=\"${code}\""
}

count_urls_in_sitemap() {
  curl -sS --max-time 30 --http1.1 "${ORIGIN}/sitemap.xml" | grep -cE '<url>'
}

echo
echo "post-deploy i18n check :: origin = ${ORIGIN}"
echo

header "1. Core marketing routes (HTTP 200 + correct <html lang>)"
for triple in "/|en" "/about|en" "/download|en" "/compare/warp|en" \
              "/fr|fr" "/fr/about|fr" "/fr/download|fr" "/fr/compare/warp|fr" \
              "/zh-Hans|zh-Hans" "/zh-Hans/about|zh-Hans" "/zh-Hans/download|zh-Hans" "/zh-Hans/compare/warp|zh-Hans"; do
  PATH_="${triple%|*}"
  EXPECT="${triple#*|}"
  CODE=$(curl_status_path "$PATH_")
  LANG=$(html_lang "$PATH_")
  if [ "$CODE" = "200" ] && [ "$LANG" = "$EXPECT" ]; then
    pass "${PATH_} -> HTTP ${CODE}, lang=${LANG}"
  else
    fail "${PATH_} -> HTTP ${CODE}, lang=${LANG} (expected 200 + lang=${EXPECT})"
  fi
done

header "2. /legal/privacy 308 redirect (Vercel edge, US-011)"
CODE=$(curl_status_code "/legal/privacy")
TARGET=$(curl_redirect_target "/legal/privacy")
if [ "$CODE" = "308" ] && [[ "$TARGET" == */fr/legal/privacy ]]; then
  pass "/legal/privacy -> ${CODE} -> ${TARGET}"
else
  fail "/legal/privacy expected 308 to /fr/legal/privacy, got ${CODE} to ${TARGET}"
fi

CODE=$(curl_status_path "/fr/legal/privacy")
LANG=$(html_lang "/fr/legal/privacy")
if [ "$CODE" = "200" ] && [ "$LANG" = "fr" ]; then
  pass "/fr/legal/privacy -> HTTP 200, lang=fr"
else
  fail "/fr/legal/privacy -> HTTP ${CODE}, lang=${LANG} (expected 200 + fr)"
fi

header "3. Accept-Language + NEXT_LOCALE cookie negotiation (edge)"
CODE=$(curl_status_code "/" -H 'Accept-Language: fr-FR,fr;q=0.9')
TARGET=$(curl -sS -I --max-time 20 --http1.1 -H 'Accept-Language: fr-FR,fr;q=0.9' "${ORIGIN}/" \
  | awk 'BEGIN{IGNORECASE=1} /^location:/ {sub(/^location:[ \t]*/,"",$0); print; exit}' | tr -d '\r\n')
if [ "$CODE" = "307" ] && [[ "$TARGET" == */fr* ]]; then
  pass "Accept-Language: fr -> 307 -> ${TARGET}"
else
  fail "Accept-Language: fr expected 307 to /fr, got ${CODE} to ${TARGET}"
fi

CODE=$(curl_status_code "/" -H 'Cookie: NEXT_LOCALE=zh-Hans')
TARGET=$(curl -sS -I --max-time 20 --http1.1 -H 'Cookie: NEXT_LOCALE=zh-Hans' "${ORIGIN}/" \
  | awk 'BEGIN{IGNORECASE=1} /^location:/ {sub(/^location:[ \t]*/,"",$0); print; exit}' | tr -d '\r\n')
if [ "$CODE" = "307" ] && [[ "$TARGET" == */zh-Hans* ]]; then
  pass "NEXT_LOCALE=zh-Hans -> 307 -> ${TARGET}"
else
  fail "NEXT_LOCALE=zh-Hans expected 307 to /zh-Hans, got ${CODE} to ${TARGET}"
fi

header "4. Sitemap (49 URLs, 27 hreflang per non-default locale)"
URLS=$(count_urls_in_sitemap)
FR_HL=$(count_hreflang_in_sitemap "fr")
ZH_HL=$(count_hreflang_in_sitemap "zh-Hans")
XD_HL=$(count_hreflang_in_sitemap "x-default")
if [ "$URLS" -ge 49 ]; then
  pass "sitemap.xml total <url> entries = ${URLS} (>= 49)"
else
  fail "sitemap.xml total <url> entries = ${URLS} (expected >= 49)"
fi
if [ "$FR_HL" -ge 27 ]; then
  pass "sitemap.xml hreflang=fr entries = ${FR_HL} (>= 27)"
else
  fail "sitemap.xml hreflang=fr entries = ${FR_HL} (expected >= 27)"
fi
if [ "$ZH_HL" -ge 27 ]; then
  pass "sitemap.xml hreflang=zh-Hans entries = ${ZH_HL} (>= 27)"
else
  fail "sitemap.xml hreflang=zh-Hans entries = ${ZH_HL} (expected >= 27)"
fi
if [ "$XD_HL" -ge 27 ]; then
  pass "sitemap.xml hreflang=x-default entries = ${XD_HL} (>= 27)"
else
  fail "sitemap.xml hreflang=x-default entries = ${XD_HL} (expected >= 27)"
fi

header "5. hreflang cluster + JSON-LD inLanguage on sample pages"
# /zh-Hans/compare/warp must list 4 alternates + 1 canonical, and JSON-LD inLanguage = zh-Hans (x2)
HTML=$("${CURL[@]}" "${ORIGIN}/zh-Hans/compare/warp")
# Use grep -o to count matches not matching lines (Next ships hreflang links on one line).
CANON=$(echo "$HTML" | grep -oE '<link[^>]*rel="canonical"' | wc -l)
ALTS=$(echo "$HTML" | grep -oE 'hrefLang="(en|fr|zh-Hans|x-default)"' | wc -l)
INLANG=$(echo "$HTML" | grep -oE '"inLanguage":"[^"]+"' | sort -u | tr '\n' ',' | sed 's/,$//')
if [ "$CANON" = "1" ] && [ "$ALTS" = "4" ]; then
  pass "/zh-Hans/compare/warp -> 1 canonical + 4 alternates"
else
  fail "/zh-Hans/compare/warp -> ${CANON} canonical, ${ALTS} alternates (expected 1 + 4)"
fi
if [[ "$INLANG" == *'"inLanguage":"zh-Hans"'* ]]; then
  pass "/zh-Hans/compare/warp JSON-LD inLanguage contains zh-Hans (${INLANG})"
else
  fail "/zh-Hans/compare/warp JSON-LD inLanguage did not contain zh-Hans (${INLANG})"
fi

HTML=$("${CURL[@]}" "${ORIGIN}/fr/compare/warp")
INLANG=$(echo "$HTML" | grep -oE '"inLanguage":"[^"]+"' | sort -u | tr '\n' ',' | sed 's/,$//')
if [[ "$INLANG" == *'"inLanguage":"fr-FR"'* ]]; then
  pass "/fr/compare/warp JSON-LD inLanguage contains fr-FR (${INLANG})"
else
  fail "/fr/compare/warp JSON-LD inLanguage did not contain fr-FR (${INLANG})"
fi

header "6. Excluded paths still served (proxy matcher)"
for PATH_ in "/docs" "/docs/first-session" "/robots.txt" "/sitemap.xml"; do
  CODE=$(curl_status_path "$PATH_")
  if [ "$CODE" = "200" ]; then
    pass "${PATH_} -> HTTP 200 (untouched by proxy)"
  else
    fail "${PATH_} -> HTTP ${CODE} (expected 200; proxy may be over-matching)"
  fi
done

# /fr/api/waitlist must 404 (proxy must NOT prefix /api)
CODE=$(curl_status_path "/fr/api/waitlist")
if [ "$CODE" = "404" ]; then
  pass "/fr/api/waitlist -> HTTP 404 (proxy correctly excludes /api/)"
else
  fail "/fr/api/waitlist -> HTTP ${CODE} (expected 404; proxy is prefixing /api)"
fi

echo
gray "---"
if [ "$FAIL" -eq 0 ]; then
  green "POST-DEPLOY CHECK PASS  (${PASS} checks)"
  echo
  echo "Next steps:"
  echo "  - Manually verify PostHog live events: see tasks/i18n-rollout-runbook.md § 2 'PostHog'"
  echo "  - Schedule the 24-hour gate: see tasks/i18n-rollout-runbook.md § 3"
  echo "  - Once 24-hour gate is GREEN: update memory research_paneflow_site_i18n.md to IMPLEMENTED + mark US-020 DONE"
  exit 0
else
  red "POST-DEPLOY CHECK FAIL  (${FAIL} failure(s), ${PASS} pass)"
  echo
  echo "Rollback procedure: tasks/i18n-rollout-runbook.md § 4"
  echo "Do NOT fix-forward on main. Promote prior deployment in the Vercel dashboard first."
  exit 1
fi
