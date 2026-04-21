import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "PaneFlow — GPU-accelerated terminal multiplexer",
  description:
    "A terminal multiplexer built in pure Rust with Zed's GPUI framework. Split, organize, and control your terminal — GPU-accelerated.",
  keywords: [
    "terminal",
    "multiplexer",
    "rust",
    "gpui",
    "gpu",
    "linux",
    "tmux",
    "pane",
  ],
  openGraph: {
    title: "PaneFlow — GPU-accelerated terminal multiplexer",
    description:
      "Split, organize, and control your terminal. Built in pure Rust with Zed's rendering engine.",
    type: "website",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      data-scroll-behavior="smooth"
      className={`${geistSans.variable} ${geistMono.variable} antialiased`}
    >
      <body className="grain">
        {children}
        <Analytics />
      </body>
    </html>
  );
}
