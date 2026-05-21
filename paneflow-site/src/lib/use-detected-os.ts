"use client";

import { useEffect, useState } from "react";

export type DetectedOS =
  | "linux"
  | "macos"
  | "windows"
  | "mobile"
  | "unknown";

type UserAgentDataLike = {
  platform?: string;
  mobile?: boolean;
};

/**
 * Client-side OS detection for the primary download CTAs. Defaults to
 * "unknown" during SSR and the first client paint so the SSR markup
 * matches hydration — only after `useEffect` runs do we set the real
 * value. Uses the User-Agent Client Hints `navigator.userAgentData`
 * when available (Chromium-based browsers), falls back to legacy
 * `navigator.userAgent` parsing on Firefox / Safari.
 *
 * The "mobile" return covers phones and tablets where no desktop
 * binary is downloadable; callers should route those visitors to the
 * /download page with its mobile-explainer panel instead of trying to
 * serve a binary.
 */
export function useDetectedOS(): DetectedOS {
  const [os, setOS] = useState<DetectedOS>("unknown");

  useEffect(() => {
    const detected = detectOS();
    if (detected !== "unknown") setOS(detected);
  }, []);

  return os;
}

function detectOS(): DetectedOS {
  if (typeof navigator === "undefined") return "unknown";

  const uaData = (
    navigator as unknown as { userAgentData?: UserAgentDataLike }
  ).userAgentData;

  if (uaData) {
    if (uaData.mobile) return "mobile";
    const platform = (uaData.platform || "").toLowerCase();
    if (platform.includes("mac")) return "macos";
    if (platform.includes("windows")) return "windows";
    if (platform.includes("linux") || platform.includes("chromeos")) return "linux";
  }

  // Legacy UA fallback — covers Firefox, Safari, and older Chromium.
  const ua = navigator.userAgent;
  if (/iPhone|iPad|iPod|Android/i.test(ua)) return "mobile";
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  if (/Linux|X11|CrOS/i.test(ua)) return "linux";

  return "unknown";
}
