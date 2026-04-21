"use client";

import { useEffect, useState } from "react";
import type { LinuxArch } from "./release";

// `navigator.userAgentData` shape (not in lib.dom.d.ts for TS 5.x — the
// Client Hints API is defined in a draft spec). `architecture` on the
// low-entropy object is empty by default; `getHighEntropyValues` is the
// documented async path to ask for it.
type UserAgentDataLike = {
  architecture?: string;
  getHighEntropyValues?: (hints: string[]) => Promise<Record<string, string>>;
};

/**
 * Client-side arch detection for the Linux-binary CTAs. Defaults to
 * `x86_64` during SSR and the first client paint (so hydration matches
 * the server markup) — ARM64 detection only fires on Chrome/Edge over
 * HTTPS (Client Hints API). Firefox / Safari / HTTP origins stay on
 * the x86_64 default; ARM users there fall back to the full matrix on
 * `/download`.
 *
 * Called from both `Hero` and `DownloadView`'s primary card.
 */
export function useDetectedLinuxArch(): LinuxArch {
  const [arch, setArch] = useState<LinuxArch>("x86_64");

  useEffect(() => {
    let cancelled = false;
    const uaData = (
      navigator as unknown as { userAgentData?: UserAgentDataLike }
    ).userAgentData;

    // Low-entropy `architecture` is often empty string; treat "arm" as
    // the only positive signal, everything else stays x86_64.
    if (uaData?.architecture === "arm") {
      // Defer to microtask — Next.js-strict
      // react-hooks/set-state-in-effect rejects synchronous setState
      // inside an effect body.
      queueMicrotask(() => {
        if (!cancelled) setArch("aarch64");
      });
    } else if (uaData?.getHighEntropyValues) {
      uaData
        .getHighEntropyValues(["architecture"])
        .then((values: Record<string, string>) => {
          if (!cancelled && values.architecture === "arm") {
            setArch("aarch64");
          }
        })
        .catch(() => {
          // Permission policy / transient rejection — keep x86_64.
        });
    }

    return () => {
      cancelled = true;
    };
  }, []);

  return arch;
}
