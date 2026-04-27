// US-008 — French language signal for the privacy page.
//
// Next.js 16 reserves <html> and <body> to the ROOT layout
// (src/app/layout.tsx). Segment layouts cannot override <html lang>.
// Verified against node_modules/next/dist/docs/01-app/03-api-reference/03-file-conventions/layout.md
// ("The root layout must define <html> and <body> tags").
//
// Practical fallback per the PRD AC: wrap this segment in a
// <div lang="fr"> so Google, screen readers, and translation widgets
// pick up French as the primary language for /legal/privacy. The
// hreflang signal is emitted by the page's metadata.alternates.languages.
//
// Do NOT relocate the URL to /fr/legal/privacy — that triggers a redirect
// chain and is gated behind a separate i18n PRD (out of scope for US-008).

export default function PrivacyLocaleLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <div lang="fr">{children}</div>;
}
