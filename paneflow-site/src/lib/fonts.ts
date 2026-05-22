import {
  Geist,
  Geist_Mono,
  Hanken_Grotesk,
  Noto_Sans_JP,
  Noto_Sans_SC,
} from "next/font/google";

export const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

export const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const hankenGrotesk = Hanken_Grotesk({
  variable: "--font-hanken-sans",
  subsets: ["latin"],
  display: "swap",
});

// Noto Sans SC (variable wght 100-900). The `subsets` parameter only
// controls Latin / Cyrillic / Vietnamese bundles - `chinese-simplified`
// is not a valid Google Fonts subset name (the PRD's wording predates
// this check). Chinese glyphs are always served by Google as
// unicode-range chunks and fetched on demand by the browser, so a
// single `latin` subset is enough alongside the auto-chunked CJK
// ranges.
//
// `preload: false`: next/font's per-route font manifest is keyed by
// route module (e.g. `[locale]/page`), not by the resolved locale
// param, so a default preload would emit the SC Latin woff2 in the
// `<link rel="preload">` cluster on EN and FR pages too. Disabling
// preload here keeps EN / FR pages free of any CJK font artifacts; on
// zh-Hans pages the browser fetches the woff2 chunks lazily via CSS
// (the `font-display: swap` fallback chain in globals.css paints
// PingFang SC / Hiragino Sans GB / Microsoft YaHei first, then swaps
// in Noto Sans SC).
export const notoSansSC = Noto_Sans_SC({
  variable: "--font-noto-sans-sc",
  subsets: ["latin"],
  display: "swap",
  preload: false,
});

// Noto Sans JP for the ja locale. Same rationale as Noto Sans SC above:
// `preload: false` keeps the Japanese woff2 chunks off EN/FR/DE/ES pages
// (next/font's per-route manifest is keyed by route module, not by
// resolved locale param). On /ja routes the browser fetches the CJK
// ranges lazily and the CSS fallback chain (Hiragino Sans / Yu Gothic /
// Meiryo) paints first.
export const notoSansJP = Noto_Sans_JP({
  variable: "--font-noto-sans-jp",
  subsets: ["latin"],
  display: "swap",
  preload: false,
});
