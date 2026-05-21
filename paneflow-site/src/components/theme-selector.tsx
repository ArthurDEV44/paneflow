"use client";

import { useEffect, useState } from "react";
import { useTheme } from "next-themes";
import { Monitor, Moon, Sun } from "lucide-react";

const OPTIONS = [
  { value: "system", label: "System theme", Icon: Monitor },
  { value: "light", label: "Light theme", Icon: Sun },
  { value: "dark", label: "Dark theme", Icon: Moon },
] as const;

/**
 * 3-state segmented theme selector — system / light / dark.
 * Used in the footer bottom bar; mirrors the same control on
 * cursor.com's footer. This is the only theme-switching surface on the
 * site (no duplicate binary toggle in the navbar).
 */
export function ThemeSelector() {
  const { theme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);

  // next-themes returns undefined during SSR + first paint. Render an
  // empty placeholder of the same size to avoid layout shift before the
  // active button highlights.
  useEffect(() => {
    queueMicrotask(() => setMounted(true));
  }, []);
  if (!mounted) {
    return <div className="h-7 w-[78px]" aria-hidden />;
  }

  return (
    <div
      role="radiogroup"
      aria-label="Theme"
      className="inline-flex items-center gap-0.5 rounded-md border border-surface-border p-0.5"
    >
      {OPTIONS.map(({ value, label, Icon }) => {
        const active = theme === value;
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={label}
            onClick={() => setTheme(value)}
            className={`flex h-6 w-6 items-center justify-center rounded transition-colors duration-150 ${
              active
                ? "bg-surface text-text"
                : "text-text-muted hover:text-text"
            }`}
          >
            <Icon className="h-3.5 w-3.5" />
          </button>
        );
      })}
    </div>
  );
}
