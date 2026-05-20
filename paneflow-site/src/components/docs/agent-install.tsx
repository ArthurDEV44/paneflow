"use client";

import { Check, Copy } from "lucide-react";
import { useState } from "react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";

const INSTALL_PROMPT = `Install Paneflow on my machine.

1. Read https://paneflow.dev/llms.txt to discover the install guides.
2. Detect my OS and architecture (uname -a on Linux/macOS).
3. Fetch the matching install page as raw markdown by appending .md to its URL (e.g. https://paneflow.dev/docs/installation/linux.md).
4. Pick the recommended format for my system: AppImage for Linux, .dmg for macOS. If two paths look equally good, ask me before picking.
5. Show me every command you plan to run BEFORE executing it. Wait for my confirmation. Never run sudo without explicit approval.
6. After install, verify with \`paneflow --version\` and tell me how to launch it.`;

/**
 * Copy-paste prompt for installing Paneflow via any agentic CLI (Claude
 * Code, Codex, OpenCode, Cursor agent, etc.). The prompt instructs the
 * agent to self-discover the docs via llms.txt, detect OS + arch, fetch
 * the right per-OS install page as raw markdown (the .md rewrite),
 * and confirm every command with the user before executing it - the
 * sudo safety rail is non-negotiable.
 */
export function AgentInstall(): React.ReactElement {
  const [copied, setCopied] = useState(false);

  async function handleCopy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(INSTALL_PROMPT);
      setCopied(true);
      if (typeof posthog?.capture === "function") {
        posthog.capture("docs_copy_install_prompt", {});
      }
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard API blocked (insecure context / permissions). Silent.
    }
  }

  return (
    <div className="not-prose relative my-8 overflow-hidden rounded-xl border border-surface-border bg-bg-elevated">
      <Button
        variant="secondary"
        size="sm"
        onClick={handleCopy}
        className="absolute top-3 right-3 z-10"
        aria-label={copied ? "Prompt copied" : "Copy install prompt"}
      >
        {copied ? <Check /> : <Copy />}
        {copied ? "Copied" : "Copy"}
      </Button>
      <pre className="overflow-x-auto px-5 py-4 sm:px-6 text-xs sm:text-sm leading-relaxed text-text-muted">
        <code className="font-mono whitespace-pre">{INSTALL_PROMPT}</code>
      </pre>
    </div>
  );
}
