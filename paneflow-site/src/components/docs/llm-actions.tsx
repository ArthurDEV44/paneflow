"use client";

import { Check, ChevronDown, Copy } from "lucide-react";
import Image from "next/image";
import { type ComponentType, useState } from "react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";
import {
  Menu,
  MenuItem,
  MenuPopup,
  MenuTrigger,
} from "@/components/ui/menu";
import { deriveDocsSection } from "@/lib/docs-analytics";

/**
 * Per-page LLM action bar shown above each docs body. Provides:
 *   - "Copy Markdown" button: instantly copies the page's stripped markdown
 *     (no JSX, no frontmatter) to the clipboard.
 *   - "Open in AI" dropdown: deep-links the page into Claude, ChatGPT,
 *     Gemini, or Mistral with a prompt prefilled. All four providers use
 *     the `?q=<encoded>` query parameter convention (matches shadcn/ui's
 *     production implementation as of May 2026). Providers that don't
 *     honour the param will open a blank chat - graceful degradation.
 *
 * The component is `"use client"` because it needs clipboard + onClick
 * handlers; the markdown payload itself is rendered server-side and
 * passed as a prop, so there is no extra network round-trip on click.
 */
export function LlmActions({
  markdown,
  pageUrl,
  pagePath,
}: {
  /** Stripped markdown body of the current page (no frontmatter, no JSX). */
  markdown: string;
  /** Absolute URL of the current page, e.g. `https://paneflow.dev/docs/...` */
  pageUrl: string;
  /** Pathname for analytics, e.g. `/docs/installation/linux`. */
  pagePath: string;
}): React.ReactElement {
  const [copied, setCopied] = useState(false);

  function track(event: string, properties: Record<string, unknown>): void {
    if (typeof window === "undefined") return;
    try {
      if (typeof posthog?.capture !== "function") return;
      posthog.capture(event, {
        slug: pagePath,
        section: deriveDocsSection(pagePath),
        ...properties,
      });
    } catch {
      // Silent - analytics must never crash docs.
    }
  }

  async function handleCopy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(markdown);
      setCopied(true);
      track("docs_copy_markdown", {});
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard API blocked (insecure context, permissions). Silent.
    }
  }

  return (
    <div className="not-prose -mt-2 mb-6 flex items-center gap-2">
      <Button
        variant="secondary"
        size="sm"
        onClick={handleCopy}
        aria-label={copied ? "Markdown copied" : "Copy page as Markdown"}
      >
        {copied ? <Check /> : <Copy />}
        {copied ? "Copied" : "Copy Markdown"}
      </Button>

      <Menu>
        <MenuTrigger
          render={
            <Button variant="secondary" size="sm">
              Open in
              <ChevronDown className="opacity-60" />
            </Button>
          }
        />
        <MenuPopup align="start" className="min-w-48">
          {PROVIDERS.map((provider) => {
            const Icon = provider.icon;
            return (
              <MenuItem
                key={provider.id}
                render={
                  <a
                    href={openInUrl(provider.baseUrl, pageUrl)}
                    target="_blank"
                    rel="noopener noreferrer"
                    onClick={() =>
                      track("docs_open_in_llm", { llm: provider.id })
                    }
                  />
                }
                className="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm cursor-pointer outline-none hover:bg-accent hover:text-background data-highlighted:bg-accent data-highlighted:text-background"
              >
                <Icon className="size-4 shrink-0" />
                <span>{provider.label}</span>
              </MenuItem>
            );
          })}
        </MenuPopup>
      </Menu>
    </div>
  );
}

interface Provider {
  id: "claude" | "chatgpt" | "mistral" | "grok";
  label: string;
  baseUrl: string;
  icon: ComponentType<{ className?: string }>;
}

/**
 * OpenAI mark. Inlined as JSX so `fill="currentColor"` resolves to the
 * surrounding text color - the icon flips white in dark theme and back
 * to black on hover (when accent-foreground takes over). Loading the
 * same file through `<Image>` would render a static bitmap and lose
 * the colour-from-context behaviour.
 */
function OpenAIIcon({ className }: { className?: string }): React.ReactElement {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      fillRule="evenodd"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
      className={className}
    >
      <path d="M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z" />
    </svg>
  );
}

/**
 * Grok mark. Same rationale as `OpenAIIcon` - the source SVG uses
 * `fill="currentColor"`, so it must be inlined as JSX to flip white in
 * dark theme and black on hover. Loading through `<Image>` would render
 * a static black bitmap that disappears on dark backgrounds.
 */
function GrokIcon({ className }: { className?: string }): React.ReactElement {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      fillRule="evenodd"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
      className={className}
    >
      <path d="M9.27 15.29l7.978-5.897c.391-.29.95-.177 1.137.272.98 2.369.542 5.215-1.41 7.169-1.951 1.954-4.667 2.382-7.149 1.406l-2.711 1.257c3.889 2.661 8.611 2.003 11.562-.953 2.341-2.344 3.066-5.539 2.388-8.42l.006.007c-.983-4.232.242-5.924 2.75-9.383.06-.082.12-.164.179-.248l-3.301 3.305v-.01L9.267 15.292M7.623 16.723c-2.792-2.67-2.31-6.801.071-9.184 1.761-1.763 4.647-2.483 7.166-1.425l2.705-1.25a7.808 7.808 0 00-1.829-1A8.975 8.975 0 005.984 5.83c-2.533 2.536-3.33 6.436-1.962 9.764 1.022 2.487-.653 4.246-2.34 6.022-.599.63-1.199 1.259-1.682 1.925l7.62-6.815" />
    </svg>
  );
}

/** Brand-colour SVG kept in /public/icons - rendered as a static image. */
function BrandImage({
  src,
  className,
}: {
  src: string;
  className?: string;
}): React.ReactElement {
  return (
    <Image
      src={src}
      alt=""
      width={16}
      height={16}
      className={className}
      unoptimized
    />
  );
}

const ClaudeIcon = ({ className }: { className?: string }) => (
  <BrandImage src="/icons/claude-color.svg" className={className} />
);
const MistralIcon = ({ className }: { className?: string }) => (
  <BrandImage src="/icons/mistral-color.svg" className={className} />
);

// Same `?q=` convention shadcn/ui ships in production (May 2026). Gemini was
// dropped because gemini.google.com ignores the `?q=` parameter on the
// consumer chat surface (only aistudio.google.com - a developer-only
// surface - honours it), so the button opened a blank chat. Mistral Le
// Chat also opens blank for the same reason, but is kept because Le Chat
// is the user-facing surface (no alternative dev surface) and we'd like
// a one-click way to reach it - the prompt remains in the URL for when
// Mistral ships prefill support.
const PROVIDERS: Provider[] = [
  {
    id: "claude",
    label: "Open in Claude",
    baseUrl: "https://claude.ai/new",
    icon: ClaudeIcon,
  },
  {
    id: "chatgpt",
    label: "Open in ChatGPT",
    baseUrl: "https://chatgpt.com",
    icon: OpenAIIcon,
  },
  {
    id: "mistral",
    label: "Open in Mistral",
    baseUrl: "https://chat.mistral.ai/chat",
    icon: MistralIcon,
  },
  {
    id: "grok",
    label: "Open in Grok",
    baseUrl: "https://grok.com",
    icon: GrokIcon,
  },
];

function openInUrl(baseUrl: string, pageUrl: string): string {
  const prompt =
    `I'm looking at this Paneflow documentation: ${pageUrl}.\n` +
    `Help me understand how to use it. Be ready to explain concepts, give examples, or help debug based on it.`;
  return `${baseUrl}?q=${encodeURIComponent(prompt)}`;
}
