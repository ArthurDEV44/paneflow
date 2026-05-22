"use client";

import { useTransition } from "react";
import { useLocale, useTranslations } from "next-intl";
import type { Locale } from "next-intl";
import { Check, ChevronUp, Globe } from "lucide-react";
import { Menu, MenuItem, MenuPopup, MenuTrigger } from "@/components/ui/menu";
import { usePathname, useRouter } from "@/i18n/navigation";
import { routing } from "@/i18n/routing";
import { track } from "@/lib/analytics";
import { cn } from "@/lib/utils";

// Native labels (autoglossonyms). Identical regardless of which locale
// the page is rendered under - that is the convention for language
// switchers per i18n style guides (W3C, Nielsen Norman). No flag emojis:
// flags conflate language with country and are an i18n anti-pattern.
const LOCALE_LABELS: Record<Locale, string> = {
  en: "English",
  fr: "Français",
  // 简体中文 (not just 中文) disambiguates Simplified from Traditional —
  // same convention Cursor / Vercel / Linear use.
  "zh-Hans": "简体中文",
  ja: "日本語",
  de: "Deutsch",
  es: "Español",
};

// Short ISO-ish tag rendered after the native label in dropdown rows so
// the menu reads "Français (FR)" / "中文 (ZH)" at a glance. Mirrors the
// Vercel / Linear / Stripe convention.
const LOCALE_TAGS: Record<Locale, string> = {
  en: "EN",
  fr: "FR",
  "zh-Hans": "ZH",
  ja: "JA",
  de: "DE",
  es: "ES",
};

// Path prefix per locale. Matches `localePath` in @/lib/i18n-metadata and
// the convention used by sitemap.ts. Used only for the JS-disabled
// <noscript> fallback - the live UI uses next-intl's router.replace.
function noScriptHref(locale: Locale, pathname: string): string {
  if (locale === routing.defaultLocale) return pathname || "/";
  if (pathname === "/" || pathname === "") return `/${locale}`;
  return `/${locale}${pathname}`;
}

interface LanguageSwitcherProps {
  variant?: "dropdown" | "inline" | "pill";
  className?: string;
}

export function LanguageSwitcher({
  variant = "dropdown",
  className,
}: LanguageSwitcherProps) {
  const currentLocale = useLocale();
  const t = useTranslations("Navbar");
  const router = useRouter();
  const pathname = usePathname();
  const [isPending, startTransition] = useTransition();

  function switchTo(nextLocale: Locale) {
    if (nextLocale === currentLocale) return;
    track("language_switched", {
      from: currentLocale,
      to: nextLocale,
      pathname,
    });
    // next-intl's router writes the NEXT_LOCALE cookie automatically;
    // `replace` keeps the URL stack clean (no back-button trap on the
    // pre-switch URL). The locale option is honored by the navigation
    // helper from createNavigation(routing).
    startTransition(() => {
      router.replace(pathname, { locale: nextLocale });
    });
  }

  if (variant === "inline") {
    // Mobile-burger render: three flat rows, matching the surrounding
    // text-2xl link cluster. Each row is a button; the active locale is
    // marked with a check glyph.
    return (
      <div className={cn("flex flex-col", className)}>
        {routing.locales.map((loc) => {
          const isActive = loc === currentLocale;
          return (
            <button
              key={loc}
              type="button"
              onClick={() => switchTo(loc)}
              disabled={isPending}
              aria-current={isActive ? "true" : undefined}
              className={cn(
                "flex items-center justify-between text-2xl py-3 text-left",
                isActive ? "text-text" : "text-text-muted",
              )}
            >
              <span>
                {LOCALE_LABELS[loc]}{" "}
                <span className="text-text-muted text-base font-normal">
                  {LOCALE_TAGS[loc]}
                </span>
              </span>
              {isActive ? <Check className="w-5 h-5" /> : null}
            </button>
          );
        })}
        <noscript>
          {/* JS-disabled fallback: plain anchors. The Menu primitive
              cannot open without JS, so this nested <noscript> is the
              degraded path. Anchors carry no cookie, but next-intl's
              middleware sees the URL prefix and serves the right locale. */}
          <div className="flex flex-col gap-2 pt-2 text-base text-text-muted">
            {routing.locales.map((loc) => (
              <a key={loc} href={noScriptHref(loc, pathname)}>
                {LOCALE_LABELS[loc]} ({LOCALE_TAGS[loc]})
              </a>
            ))}
          </div>
        </noscript>
      </div>
    );
  }

  if (variant === "pill") {
    // Footer pill — matches cursor.com's footer language switcher:
    // rounded-full pill with globe + native label + chevron, dropdown
    // pops UPWARD (side="top") so it doesn't disappear off the bottom
    // of the page. Native language labels in the menu rows; check glyph
    // on the active locale.
    return (
      <div className={cn("relative", className)}>
        <Menu>
          <MenuTrigger
            aria-label={t("aria.languageMenu")}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-full border border-surface-border px-3 py-1 text-sm text-text-muted",
              "hover:text-text hover:border-text-muted/40 transition-colors duration-150",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            )}
          >
            <Globe className="w-3.5 h-3.5" />
            <span>{LOCALE_LABELS[currentLocale]}</span>
            <ChevronUp className="w-3.5 h-3.5 opacity-70" />
          </MenuTrigger>
          <MenuPopup
            side="top"
            align="end"
            className="min-w-44 bg-bg border-surface-border rounded-md shadow-sm"
          >
            {routing.locales.map((loc) => {
              const isActive = loc === currentLocale;
              return (
                <MenuItem
                  key={loc}
                  onClick={() => switchTo(loc)}
                  aria-current={isActive ? "true" : undefined}
                  // Cursor-style hover: keep text color stable, tint the
                  // row with a soft cream surface instead of inverting to
                  // the dark `accent` token (which is near-black in light
                  // theme and produced a Linear-style aggressive flip).
                  className={cn(
                    "rounded-sm px-2 py-1.5 text-sm text-text",
                    "data-highlighted:bg-surface data-highlighted:text-text",
                  )}
                >
                  <span className="flex-1">{LOCALE_LABELS[loc]}</span>
                  {isActive ? <Check className="w-4 h-4 opacity-80" /> : null}
                </MenuItem>
              );
            })}
          </MenuPopup>
        </Menu>
        <noscript>
          <ul className="flex items-center gap-2 text-sm text-text-muted">
            {routing.locales.map((loc) => (
              <li key={loc}>
                <a href={noScriptHref(loc, pathname)}>{LOCALE_LABELS[loc]}</a>
              </li>
            ))}
          </ul>
        </noscript>
      </div>
    );
  }

  // Desktop dropdown render via CossUI / base-ui Menu primitive.
  // base-ui handles keyboard open (Enter/Space), arrow navigation,
  // Escape close, focus trap, and dismissal on outside click. We add
  // a <noscript> fallback adjacent so JS-disabled users still get the
  // links - styled to match the navbar's text-muted register.
  return (
    <div className={cn("relative", className)}>
      <Menu>
        <MenuTrigger
          aria-label={t("aria.languageMenu")}
          className={cn(
            "flex items-center gap-1.5 text-sm text-text-muted hover:text-text transition-colors duration-200",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded",
          )}
        >
          <Globe className="w-4 h-4" />
          <span>{LOCALE_TAGS[currentLocale]}</span>
        </MenuTrigger>
        <MenuPopup align="end" className="min-w-40">
          {routing.locales.map((loc) => {
            const isActive = loc === currentLocale;
            return (
              <MenuItem
                key={loc}
                onClick={() => switchTo(loc)}
                aria-current={isActive ? "true" : undefined}
              >
                <span className="flex-1">
                  {LOCALE_LABELS[loc]}{" "}
                  <span className="text-text-muted text-xs">
                    {LOCALE_TAGS[loc]}
                  </span>
                </span>
                {isActive ? <Check className="w-4 h-4 opacity-80" /> : null}
              </MenuItem>
            );
          })}
        </MenuPopup>
      </Menu>
      <noscript>
        <ul className="flex items-center gap-2 text-sm text-text-muted">
          {routing.locales.map((loc) => (
            <li key={loc}>
              <a href={noScriptHref(loc, pathname)}>{LOCALE_TAGS[loc]}</a>
            </li>
          ))}
        </ul>
      </noscript>
    </div>
  );
}
