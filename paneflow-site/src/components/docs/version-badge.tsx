import type * as React from "react";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipPopup,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export type Stability = "stable" | "beta" | "experimental";

const STABILITY_TO_VARIANT = {
  stable: "success",
  beta: "warning",
  experimental: "error",
} as const;

const STABILITY_TO_LABEL = {
  stable: "Stable",
  beta: "Beta",
  experimental: "Experimental",
} as const;

function resolveStability(value: string | undefined): Stability {
  if (value === "stable" || value === "beta" || value === "experimental") {
    return value;
  }
  return "stable";
}

/**
 * Inline stability chip.
 *
 *   <VersionBadge stability="beta" />
 *
 * Unknown `stability` values fall back to "stable" - no runtime crash.
 */
export function VersionBadge({
  stability,
}: {
  stability?: string;
}): React.ReactElement {
  const resolved = resolveStability(stability);
  return (
    <Badge size="sm" variant={STABILITY_TO_VARIANT[resolved]}>
      {STABILITY_TO_LABEL[resolved]}
    </Badge>
  );
}

/**
 * "Since v<version>" chip. When `date` is provided (release date in any
 * human-readable form), wraps in a Tooltip so hovering reveals it.
 *
 *   <Since v="0.2.9" />
 *   <Since v="0.3.0" date="2026-05-12" />
 */
export function Since({
  v,
  date,
}: {
  v: string;
  date?: string;
}): React.ReactElement {
  const chip = (
    <Badge size="sm" variant="outline">
      Since v{v}
    </Badge>
  );
  if (!date) return chip;
  // base-ui Tooltip auto-wires `aria-describedby` from trigger to popup, so
  // the chip text stays as the accessible name and the date is announced
  // as a description. Adding an `aria-label` here would override the chip
  // text and bury "Since v0.2.9" until the tooltip opens.
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger render={<span className="inline-flex">{chip}</span>} />
        <TooltipPopup>Released {date}</TooltipPopup>
      </Tooltip>
    </TooltipProvider>
  );
}
