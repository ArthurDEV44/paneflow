import type { Metadata } from "next";
import Link from "next/link";
import {
  CompareFaq,
  CompareHeader,
  CompareLayout,
  CompareSection,
  CompareTable,
  DecisionGuide,
} from "@/components/compare/compare-layout";
import { buildCompareJsonLd } from "@/lib/json-ld-compare";

const DATE_MODIFIED = "2026-05-20";

const FAQ = [
  {
    question: "Is Paneflow a fork of iTerm2?",
    answer:
      "No. iTerm2 is a 16-year-old hybrid Objective-C and Swift codebase by George Nachman that targets macOS only (the AppKit + Metal stack is hard-locked to Apple platforms). Paneflow is a one-month-old pure-Rust codebase by Arthur Jean that runs on Linux and macOS today, with Windows planned. The two share no source code and aim at different audiences.",
  },
  {
    question:
      "Both ship Claude Code integration in 2026. What is the actual difference?",
    answer:
      "Architecture and portability. iTerm2's v3.7.0beta1 (April 2026) ships a session-aware integration that parses the Claude Code hook event protocol via ClaudeCodeHookEvent.swift and surfaces hooks through a vendored multi-vendor AI chat (OpenAI, Anthropic, Gemini, DeepSeek per iTerm2's own AI live harness). Paneflow runs the Claude Code CLI as a first-class pane with a dedicated UI button - same agent binary you already use on the command line, with workspace and session restore around it. Pick the iTerm2 model if you want a vendored chat experience; pick Paneflow if you want the CLI agents you already use, in a cross-platform host.",
  },
  {
    question:
      "iTerm2 has AppleScript and a Python API. What does Paneflow offer?",
    answer:
      "Paneflow exposes a single JSON-RPC 2.0 server over a Unix socket with roughly thirteen methods covering workspaces and panes. iTerm2 ships a far broader scripting surface: an AppleScript .sdef bundle, a Python scripting API with long-running script support, and an extensive shell integration that exposes triggers, smart selection, and prompt-aware features. If you depend on AppleScript automation or the Python scripting API today, Paneflow does not have a replacement.",
  },
  {
    question:
      "Why pick Paneflow if iTerm2 has 16 years of polish and a 17 k star community?",
    answer:
      "If you need cross-platform (Linux + macOS now, Windows planned), or you want MIT instead of GPL-2.0 for embedding flexibility, or you specifically want the agent-host architecture where Claude Code, Codex, and OpenCode run as the same CLI binaries you already use, Paneflow is the better fit. If you are macOS-only, want maximum polish, and value AppleScript / Python automation, iTerm2 is the better fit.",
  },
  {
    question: "Does iTerm2 run on Linux or Windows?",
    answer:
      "No. iTerm2 is macOS-only by design. The rendering stack is Metal-direct (PTYTextView is Metal-accelerated per iTerm2's own AGENTS.md) which is an Apple-only API, and the UI is AppKit + SwiftUI. There is no Linux build and none planned. Paneflow ships Linux and macOS today and has Windows on the roadmap.",
  },
  {
    question: "Can I migrate my iTerm2 settings to Paneflow?",
    answer:
      "There is no automatic path. iTerm2 stores configuration in a binary plist that syncs through iCloud; Paneflow uses a static JSON file at ~/.config/paneflow/paneflow.json on Linux. The schemas, the keystroke notation, and the AI integration model all differ. Plan ten minutes for a fresh setup: default shell, theme, keybindings.",
  },
  {
    question: "Is iTerm2 still single-maintainer?",
    answer:
      "Yes. George Nachman holds 15 169 of iTerm2's commits as of fetch date 2026-05-20 - roughly 98% of the project, with the next contributor at 149 commits. Same structural pattern as Paneflow (Arthur Jean, solo) and WezTerm (Wez Furlong, ~98% solo). Single-maintainer is the norm across this competitive set; the difference is that iTerm2's solo run has now lasted 16 years.",
  },
  {
    question:
      "What about iTerm2's Workgroups feature in v3.7? Does Paneflow have an equivalent?",
    answer:
      "Workgroups (new in v3.7.0beta1) lets you transform a session into a group of related sessions (peers, split panes, or tabs) each with its own toolbar, plus a Code Review mode that shows an in-session prompt overlay before the program runs. Paneflow's workspace model is conceptually different: each workspace groups panes around a project root with branch detection, and Code Review-style overlays are not on the roadmap. If you specifically want Workgroups, iTerm2 is the better fit.",
  },
];

export const metadata: Metadata = {
  title:
    "Paneflow vs iTerm2 (2026): cross-platform agent host vs macOS veteran with built-in chat",
  description:
    "Paneflow vs iTerm2: cross-platform (Linux + macOS, Windows planned) MIT-licensed agent workspace running the CLI agents you already use, vs macOS-only GPL-2.0 veteran with vendored multi-vendor AI chat. Honest decision guide, architecture, FAQ.",
  alternates: {
    canonical: "/compare/iterm2",
  },
  openGraph: {
    title: "Paneflow vs iTerm2 (2026)",
    description:
      "Cross-platform MIT agent host vs macOS-only GPL-2.0 veteran. Both ship Claude Code integration; the architectures and portability stories differ. Decision guide for Claude Code, Codex, OpenCode workflows.",
    type: "article",
  },
};

export default function CompareIterm2Page() {
  const jsonLd = buildCompareJsonLd({
    competitorName: "iTerm2",
    competitorSlug: "iterm2",
    headline: "Paneflow vs iTerm2 (2026)",
    description:
      "Cross-platform Rust agent host vs macOS-only Objective-C/Swift veteran. Both GPU-accelerated, both ship Claude Code integration; portability and agent architecture differ. Decision guide.",
    dateModified: DATE_MODIFIED,
    faq: FAQ,
  });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader
        title="Paneflow vs iTerm2"
        tldr={
          <>
            iTerm2 is the macOS veteran (16-year codebase, 17 500+ stars,
            GPL-2.0) that ships in v3.7.0beta1 with a vendored multi-vendor
            AI chat and a session-aware Claude Code integration. Paneflow
            is the cross-platform indie newcomer (one month old, MIT, pure
            Rust on Zed&rsquo;s GPUI) that runs Claude Code, Codex, and
            OpenCode as first-class CLI panes.{" "}
            <strong className="text-text">
              Both are GPU-accelerated. Both ship Claude Code integration
              today.
            </strong>{" "}
            They diverge on portability (Paneflow runs on Linux + macOS
            now with Windows planned; iTerm2 is macOS-only by design),
            license (MIT vs GPL-2.0), and agent architecture (Paneflow
            launches the CLI agents you already use; iTerm2 ships a
            vendored chat surface plus hooks). Pick Paneflow if you want
            a cross-platform agent host. Pick iTerm2 if you want 16 years
            of macOS polish with AppleScript and Python automation built
            in.
          </>
        }
      />

      <CompareSection
        id="context"
        title="A note on context"
      >
        <p>
          Some things the input research for this page got wrong, which
          this section flags up front so the rest reads honestly. iTerm2
          is NOT CPU-bound - its own AGENTS.md states &ldquo;PTYTextView
          - Metal-accelerated rendering.&rdquo; The cleanest USP framing
          is not &ldquo;modern GPU vs CPU veteran;&rdquo; it is{" "}
          <em>Metal (macOS-only) vs GPUI/Blade (cross-platform)</em>. And
          iTerm2&rsquo;s Claude Code integration is NOT a launcher stub:
          the file <code>ClaudeCodeHookEvent.swift</code>{" "}at the iTerm2
          repo root parses the upstream Claude Code hook event protocol
          via full Codable types, with companion files for onboarding,
          health monitoring, and a workgroup mode controller. The honest
          USP framing is not &ldquo;agent-first vs agent-free;&rdquo; it
          is <em>CLI-agent-host (Paneflow) vs vendored multi-vendor chat
          (iTerm2)</em>.
        </p>
        <p>
          With that out of the way: the two products solve overlapping
          problems with different priorities. iTerm2 has spent 16 years
          becoming the most thoroughly polished macOS terminal in
          existence, and in 2026 it added a serious AI surface
          (multi-vendor chat covering OpenAI, Anthropic, Gemini, and
          DeepSeek per its own AI live harness, plus Claude Code session
          hooks). Paneflow has spent one month becoming a cross-platform
          agent host where Claude Code, Codex, and OpenCode are the CLI
          binaries you already use, mounted into a workspace with branch
          detection, dev-server port banners, and session restore.
        </p>
      </CompareSection>

      <CompareSection id="quick-comparison" title="Quick comparison">
        <p>
          Grouped in three zones - portability &amp; agent surface (where
          Paneflow leads), core terminal parity (where both are
          equivalent today), and the macOS-native polish (where iTerm2
          has shipped much more surface).
        </p>

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          Portability &amp; agent surface
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["Cold start", "<200 ms", "not published"],
            ["Keystroke-to-pixel latency", "<4 ms", "not published"],
            ["OS support", "Linux + macOS (Windows planned)", "macOS only"],
            ["License", "MIT", "GPL-2.0"],
            [
              "Agent model",
              "Launches CLI agents (Claude Code, Codex, OpenCode) as panes",
              "Vendored multi-vendor chat (OpenAI, Anthropic, Gemini, DeepSeek) + Claude Code session hooks",
            ],
            [
              "AI agents (dedicated UI buttons)",
              "3 (Claude Code, Codex, OpenCode)",
              "Multi-vendor chat panel + Claude Code workgroup mode",
            ],
            ["Branch-aware workspace badges", "Yes", "n/a"],
            ["Dev-server port detection", "Yes", "n/a"],
            [
              "Latest release",
              "v0.2.16 (May 2026, active weekly)",
              "v3.7.0beta1 (April 2026, marked work-in-progress)",
            ],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          Core terminal parity
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["GPU rendering", "Yes (GPUI/Blade over Vulkan + Metal)", "Yes (Metal-direct via PTYTextView)"],
            ["Language", "Rust", "Hybrid Objective-C + Swift"],
            [
              "VT emulator",
              "alacritty_terminal 0.26 (upstream crate)",
              "Built-in (VT100Parser, VT100Terminal, VT100Screen, VT100Grid)",
            ],
            ["Tabs + splits", "Yes", "Yes"],
            ["Session restore on relaunch", "Yes", "Yes"],
            ["True color, mouse, hyperlinks", "Yes", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + custom color schemes"],
            ["Pricing", "Free (MIT)", "Free (GPL-2.0, donation-supported)"],
            ["Maintainer model", "Indie (Arthur Jean, solo)", "Indie (George Nachman, ~98% commits over 16 years)"],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          macOS-native polish
        </h3>
        <CompareTable
          headers={["", "Paneflow", "iTerm2"]}
          rows={[
            ["AppleScript .sdef scripting", "n/a", "Yes"],
            ["Python scripting API", "n/a", "Yes (long-running scripts supported)"],
            [
              "Shell integration (triggers, smart selection, prompt awareness)",
              "n/a",
              "Yes (extensive)",
            ],
            ["Workgroups (session grouping + Code Review mode)", "n/a", "Yes (new in v3.7.0beta1)"],
            [
              "Concurrent instances (independent settings)",
              "n/a",
              "Yes (`--suite=com.iterm2.<id>`)",
            ],
            ["Tab Status system + per-pane status sorting", "n/a", "Yes (new in v3.7)"],
            ["Hotkey window", "n/a", "Yes"],
            ["GitHub stars", "Small, growing", "17 500+ (16 years of accumulation)"],
            ["Codebase age", "~1 month", "~16 years (first commit 2010-07-20)"],
          ]}
        />

        <p className="text-xs text-text-subtle mt-8 leading-relaxed">
          <strong className="text-text-muted">Versions:</strong> Paneflow
          v0.2.16 (May 2026), first commit 2026-04-01. iTerm2 v3.7.0beta1
          released 2026-04-20 (commit{" "}
          <code>2b7b5bff</code>), with v3.6.10 (2026-04-20) and v3.5.15
          (2025-08-13) as older stable lines. iTerm2&rsquo;s own release
          notes mark Claude Code integration as &ldquo;still a work in
          progress as of 3.7.0beta1.&rdquo;{" "}
          <strong className="text-text-muted">Pricing:</strong> both
          free. Paneflow MIT; iTerm2 GPL-2.0 with donation support.
        </p>
      </CompareSection>

      <CompareSection id="decision-guide" title="Which one is right for you?">
        <p>
          The honest version: this is a clean either/or for most users.
          The two bullet lists below capture who each tool is genuinely
          built for. Most decisions land on portability + agent
          architecture vs macOS-native depth.
        </p>
        <DecisionGuide
          left={{
            heading: "Choose Paneflow if",
            bullets: [
              "You need cross-platform - Linux + macOS today, Windows planned. iTerm2 is macOS-only by design (Metal is an Apple API)",
              "You want MIT for embedding flexibility, not GPL-2.0",
              "Your agent workflow is launching CLI binaries you already use (Claude Code, Codex, OpenCode) with a dedicated UI button per agent",
              "You want a workspace model with branch-aware badges, dev-server port detection, and session restore baked in",
              "You prefer a JSON config you can read at a glance over a binary plist with iCloud sync semantics",
              "You back an indie dev shipping weekly minor releases through Zed-philosophy lineage",
            ],
          }}
          right={{
            heading: "Choose iTerm2 if",
            bullets: [
              "You are macOS-only and want the most polished native experience in existence today",
              "You depend on AppleScript automation or the Python scripting API",
              "You want a vendored multi-vendor AI chat (OpenAI, Anthropic, Gemini, DeepSeek) inside the terminal rather than launching external CLI agents",
              "You use iTerm2's shell integration features - triggers, smart selection, prompt-aware navigation, marks",
              "You want the new Workgroups feature with Code Review mode and per-session toolbars",
              "You value a 16-year-old codebase with consistent maintenance and a 17 k+ star community",
              "You are comfortable with GPL-2.0 or never plan to embed terminal code in a closed-source product",
            ],
          }}
        />
      </CompareSection>

      <CompareSection id="architecture" title="Architecture deep-dive">
        <p>
          <strong className="text-text">iTerm2</strong>{" "}is a hybrid
          Objective-C / Swift application targeting macOS. The
          application flow is App -&gt; Window/Tab -&gt; Session -&gt;
          Terminal Emulation -&gt; Rendering. The terminal emulation
          stack is its own (<code>VT100Parser</code>,{" "}
          <code>VT100Terminal</code>, <code>VT100ScreenMutableState</code>
          , <code>VT100Screen</code>, <code>VT100Grid</code>), and the
          renderer is <code>PTYTextView</code> with Metal acceleration -
          the renderer is hard-locked to Apple platforms. The AI surface
          lives in <code>sources/AITerm/</code> (about 81 files including
          AIConversation, AIPluginClient, and a Chat* family of view
          controllers) plus <code>sources/ClaudeCode/</code> (six files:
          ClaudeCodeIntegrationMenuController, ClaudeCodeOnboarding,
          ClaudeIntegrationHealthMonitor, ClaudeWatcher,
          GlobalJobMonitor, PeerSessionSettingsViewController). The hook
          surface at the repo root - <code>ClaudeCodeHookEvent.swift</code>
          {" "}- is a full set of Codable types matching the upstream
          Claude Code hook protocol, so iTerm2 can react to session
          events from the Claude CLI as they happen.
        </p>
        <p>
          <strong className="text-text">Paneflow</strong>{" "}is a pure-Rust
          application built on Zed&rsquo;s GPUI engine. There is no
          language boundary between the UI and the terminal - keystrokes
          travel through a single Rust pipeline from{" "}
          <code>KeyDownEvent</code>{" "}to paint, which is why
          keystroke-to-pixel latency stays under 4&nbsp;ms and cold
          start under 200&nbsp;ms. Terminal emulation is upstream{" "}
          <code>alacritty_terminal</code> 0.26 from crates.io (no fork
          to maintain). The GPU stack is GPUI&rsquo;s Blade renderer
          over Vulkan on Linux and Metal on macOS - so the renderer
          travels across platforms with the rest of the app. The agent
          surface is a workspace model: each workspace groups panes
          around a project root, with branch detection, port banners,
          and three first-class CLI agent buttons (Claude Code, Codex,
          OpenCode). External CLI agents are also unlimited: any binary
          you can run in a shell runs in a Paneflow pane.
        </p>
        <p>
          The IPC surfaces are equally telling about each
          project&rsquo;s era. iTerm2 exposes AppleScript (a long
          legacy .sdef bundle) and a Python scripting API with
          long-running script support, plus extensive shell integration
          (triggers, smart selection, prompt-aware features). Paneflow
          exposes a single JSON-RPC 2.0 server over a Unix socket at{" "}
          <code>$XDG_RUNTIME_DIR/paneflow/paneflow.sock</code>{" "}with
          roughly thirteen methods - smaller surface, faster to learn,
          far less coverage than iTerm2&rsquo;s sixteen years of
          scripting accretion. Catching up to iTerm2 on automation is
          not on the immediate Paneflow roadmap.
        </p>
      </CompareSection>

      <CompareSection id="pricing" title="Pricing">
        <p>
          Both projects are free. The licensing models differ:
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Paneflow</strong>: MIT,
              Copyright (c) 2026 Arthur Jean. Embed it in commercial
              products without concerns; no copyleft constraint.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">iTerm2</strong>: GPL-2.0,
              donation-supported. Strong copyleft - if you redistribute
              modified versions you must also release source. Embedding
              iTerm2 code in a closed-source product is functionally not
              an option. For an end user this difference is invisible;
              for an embedder it is the whole difference.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection
        id="migrating"
        title="Migrating from iTerm2 to Paneflow"
      >
        <p>
          There is no direct config translation. iTerm2 stores settings
          in a binary plist (<code>com.googlecode.iterm2.plist</code>)
          that syncs through iCloud; Paneflow uses a static JSON file at{" "}
          <code>~/.config/paneflow/paneflow.json</code> on Linux and{" "}
          <code>~/Library/Application Support/paneflow/paneflow.json</code>
          {" "}on macOS. The install is a fresh setup that takes about
          ten minutes - default shell, theme, keybindings, AI agent
          buttons. The good news: Paneflow ships sensible defaults for
          the things iTerm2 makes you configure manually (workspace
          layout, agent buttons, theme), so the empty-config experience
          is closer to ready-to-use than to iTerm2&rsquo;s.
        </p>
        <p>
          What does NOT port across:
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">AppleScript / Python scripts</strong>:
              no equivalent in Paneflow. If you have automation written
              against iTerm2&rsquo;s Python API or AppleScript surface,
              stay on iTerm2.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Shell integration features</strong>:
              triggers, smart selection, prompt-aware navigation, marks
              - these are iTerm2-specific. Paneflow uses a different,
              smaller integration surface.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Multi-vendor AI chat</strong>:
              Paneflow does not ship a vendored chat. The migration is
              to use the CLI agent of your choice (Claude Code, Codex,
              OpenCode) launched as a pane.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Workgroups + Code Review mode</strong>:
              no Paneflow equivalent. The closest concept is a workspace
              with multiple panes, but the Code Review prompt overlay
              and per-session toolbars do not have analogues.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="when-not" title="When NOT to choose Paneflow">
        <p>
          The honest dealbreakers. If any of the five below matters to
          you, iTerm2 is the right tool today - no point fighting it:
        </p>
        <ol className="space-y-3 text-sm">
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">1.</span>
            <span>
              <strong className="text-text">
                You depend on AppleScript or iTerm2&rsquo;s Python API.
              </strong>{" "}
              iTerm2 has a long-established .sdef AppleScript bundle and
              a Python scripting API with long-running script support.
              Paneflow has a thirteen-method JSON-RPC and that&rsquo;s
              it. Sixteen years of automation work does not port.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">2.</span>
            <span>
              <strong className="text-text">
                You want a vendored multi-vendor AI chat in the terminal.
              </strong>{" "}
              iTerm2 ships a multi-vendor chat covering OpenAI,
              Anthropic, Gemini, and DeepSeek with a unified UI inside
              the terminal pane. Paneflow only launches external CLI
              agents - there is no in-terminal chat panel.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">3.</span>
            <span>
              <strong className="text-text">
                You rely on iTerm2 shell integration features.
              </strong>{" "}
              Triggers, smart selection, prompt-aware navigation, marks
              - these are iTerm2-specific. If you rely on any of them,
              Paneflow does not have a replacement on the immediate
              roadmap.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">4.</span>
            <span>
              <strong className="text-text">
                You want Workgroups with Code Review mode.
              </strong>{" "}
              v3.7&rsquo;s Workgroups + Code Review mode is genuinely
              novel: it shows an in-session prompt overlay before the
              program runs, exposes the entered text as a variable for
              swifty interpolation, and lets you build review pipelines
              around it. Paneflow has no equivalent.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">5.</span>
            <span>
              <strong className="text-text">
                You want sixteen years of polish and a 17 k+ star
                community.
              </strong>{" "}
              iTerm2 is at v3.7 with sixteen years of accumulated bug
              fixes, edge cases, and ecosystem (terminfo entries, dot
              files, Stack Overflow). Paneflow is at v0.2.x with a
              fraction of that ground-truth time.
            </span>
          </li>
        </ol>
      </CompareSection>

      <CompareSection id="faq" title="Frequently asked questions">
        <CompareFaq
          entries={FAQ.map(({ question, answer }) => ({
            question,
            answer,
          }))}
        />
      </CompareSection>

      <CompareSection id="next" title="Next steps">
        <p>
          Ready to try Paneflow?{" "}
          <Link
            href="/download"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
          >
            Download the latest release
          </Link>{" "}
          or read the{" "}
          <Link
            href="/docs"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
          >
            getting-started guide
          </Link>
          . Curious about iTerm2 instead?{" "}
          <a
            href="https://github.com/gnachman/iTerm2"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
            rel="noopener noreferrer"
            target="_blank"
          >
            iTerm2 is on GitHub
          </a>{" "}
          - it is the most polished macOS terminal in existence and a
          worthy alternative for any macOS-only workflow that values
          AppleScript or Python automation.
        </p>
      </CompareSection>
    </CompareLayout>
  );
}
