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
    question: "Is Paneflow a fork of cmux?",
    answer:
      "No. Paneflow is an independent Rust implementation inspired by cmux's agent-first design. The two codebases share no source code. Paneflow uses GPUI (Zed's framework) + upstream alacritty_terminal; cmux uses AppKit + libghostty via the GhosttyKit.xcframework.",
  },
  {
    question: "How many AI agents does each terminal support?",
    answer:
      "cmux ships first-class integration for 15+ agents: Claude Code, Codex, Grok, OpenCode, Cursor, Copilot, Gemini, Antigravity, Rovo Dev, Hermes, CodeBuddy, Factory, Qoder, Amp, Pi, plus a custom-agent slot. Paneflow ships first-class buttons for three today (Claude Code, Codex, OpenCode). If you rotate through several different agents, cmux has the broader zoo.",
  },
  {
    question: "Why pick Paneflow if cmux is more mature?",
    answer:
      "If you need Linux today (Paneflow ships it, cmux is macOS-only), or you specifically want MIT-licensed tooling without a separate commercial license question, or you prefer a small Rust codebase you can audit and contribute to, Paneflow is the better fit. For macOS-only feature-rich workflows in May 2026, cmux is more polished and has a much larger surface (cloud VMs, command palette, AppleScript, tmux compatibility, embedded browser).",
  },
  {
    question: "Will Paneflow run on Windows?",
    answer:
      "Native Windows is planned, no shipping ETA yet. WSL2 + the Linux build is not currently viable: GPUI's renderer needs the VK_EXT_inline_uniform_block Vulkan extension which WSLg's dzn driver does not implement, so it would fall back to llvmpipe software rendering - unusable for a terminal multiplexer.",
  },
  {
    question: "Can I migrate my cmux config to Paneflow?",
    answer:
      "Partially. Both projects use a JSON config file with similar shape (default shell, theme, keybindings, AI agent buttons). The keystroke notation matches on the modifier (Cmd on macOS, Ctrl on Linux). Session files are NOT portable - the on-disk format differs. A migration helper script is on the roadmap; for now the manual translation takes about ten minutes.",
  },
  {
    question:
      "Why does Paneflow use alacritty_terminal instead of Ghostty?",
    answer:
      "alacritty_terminal is published on crates.io with stable Rust semver guarantees and integrates cleanly with GPUI's render loop. Ghostty is a C library that cmux accesses through the GhosttyKit.xcframework - perfectly fine in cmux's Swift app, but Paneflow's pure-Rust stack prefers a pure-Rust VT emulator. This is an architectural preference, not a quality judgment. Both engines are excellent.",
  },
  {
    question: "Does Paneflow support running agents on remote machines?",
    answer:
      "Not today. cmux ships a Go daemon that auto-deploys over SSH (`cmux ssh user@host`) to create a remote workspace transparently. Paneflow's roadmap does not currently include this feature. If your workflow depends on remote agents, cmux is the better choice today.",
  },
  {
    question: "Is there an embedded browser in Paneflow?",
    answer:
      "No. cmux ships a full WKWebView-based browser with omnibar, profile import from Chrome/Firefox/Safari/Brave/Edge/Arc, and tab management. Paneflow does not include a browser - it focuses on terminal panes plus a markdown viewer. If you want a browser inside your terminal workspace, cmux is the better choice.",
  },
];

export const metadata: Metadata = {
  title: "Paneflow vs cmux (2026): cross-platform Rust vs macOS-native Swift",
  description:
    "Honest comparison of Paneflow and cmux, two agent-first terminal workspaces. Paneflow is cross-platform Rust + MIT; cmux is macOS-only Swift + GPL with a deeper feature set. Decision guide, architecture deep-dive, FAQ.",
  alternates: {
    canonical: "/compare/cmux",
  },
  openGraph: {
    title: "Paneflow vs cmux (2026)",
    description:
      "Cross-platform Rust agent-first terminal vs macOS-only Swift incumbent. Honest comparison covering architecture, features, pricing, and when each is the right pick.",
    type: "article",
  },
};

export default function CompareCmuxPage() {
  const jsonLd = buildCompareJsonLd({
    competitorName: "cmux",
    competitorSlug: "cmux",
    headline: "Paneflow vs cmux (2026)",
    description:
      "Cross-platform Rust agent-first terminal vs macOS-only Swift incumbent. Architecture, feature, and pricing comparison.",
    dateModified: DATE_MODIFIED,
    faq: FAQ,
  });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader
        title="Paneflow vs cmux"
        tldr={
          <>
            Both Paneflow and cmux are agent-first terminal workspaces for
            running Claude Code, Codex, OpenCode, and other CLI coding agents
            in parallel. <strong className="text-text">cmux</strong> is
            macOS-only, more mature (17 500+ GitHub stars, v0.64), and
            feature-rich (embedded browser, SSH daemon, 150+ command socket
            API). <strong className="text-text">Paneflow</strong> is younger
            (v0.2.x), cross-platform (Linux + macOS, Windows planned), MIT
            licensed, and built in pure Rust on Zed&rsquo;s GPUI engine.
            Choose based on your OS, your appetite for early-stage tooling,
            and whether you need cmux&rsquo;s heavier feature set.
          </>
        }
      />

      <CompareSection id="inspiration" title="A note on inspiration">
        <p>
          Paneflow&rsquo;s design is openly inspired by cmux. cmux was the
          first project to ship a polished agent-first terminal multiplexer
          for developers running multiple AI coding agents in parallel, and
          the &ldquo;workspace per project, panes per agent&rdquo; mental
          model came from it.
        </p>
        <p>
          Paneflow is not a fork. It is an independent Rust codebase that
          reimplements that mental model with different architectural
          choices: pure Rust instead of Swift, GPUI instead of AppKit,
          upstream <code>alacritty_terminal</code> instead of libghostty,
          and cross-platform from day one instead of macOS-first. Both
          projects exist in parallel and solve overlapping problems with
          different tradeoffs - the rest of this page lays them out.
        </p>
      </CompareSection>

      <CompareSection id="quick-comparison" title="Quick comparison">
        <CompareTable
          headers={["", "Paneflow", "cmux"]}
          rows={[
            ["OS support", "Linux, macOS (Windows planned)", "macOS only"],
            ["License", "MIT", "GPL-3.0-or-later (+ commercial license)"],
            ["Language", "Rust", "Swift (with Go SSH daemon)"],
            ["UI framework", "GPUI (same as Zed)", "AppKit + SwiftUI"],
            [
              "Terminal engine",
              "alacritty_terminal 0.26 (upstream crates.io)",
              "libghostty via GhosttyKit.xcframework",
            ],
            ["GPU stack", "Vulkan (Linux) / Metal (macOS) via Blade", "Metal via CAMetalLayer"],
            ["Workspaces", "Yes", "Yes"],
            ["Pane layout", "N-ary tree, 4 preset layouts", "N-ary tree via Bonsplit"],
            [
              "AI agent integrations",
              "3 (Claude Code, Codex, OpenCode)",
              "15+ (Claude Code, Codex, Grok, OpenCode, Cursor, Copilot, Gemini, Antigravity, Rovo Dev, Hermes, CodeBuddy, Factory, Qoder, Amp, Pi + custom)",
            ],
            ["Customizable shortcut actions", "~30 actions", "71 actions"],
            ["Session restore", "Yes (workspaces + CWD)", "Yes"],
            ["Dev-server port detection", "Yes", "Yes"],
            ["Branch-aware workspace badges", "Yes", "Yes"],
            [
              "IPC",
              "JSON-RPC 2.0 over Unix socket (~13 methods)",
              "Dual socket: V1 space-delimited text + V2 newline-delimited JSON, several hundred commands",
            ],
            ["Command palette", "No", "Yes (fuzzy-search)"],
            ["AppleScript scripting", "No", "Yes (.sdef bundle)"],
            ["Tmux compatibility shim", "No", "Yes (capture-pane, pipe-pane, bind-key, paste-buffer, set-hook)"],
            ["Embedded browser", "No", "Yes (WKWebView, Chrome/Firefox/Safari/Brave/Edge/Arc profile import)"],
            ["Cloud VM provisioning", "No", "Yes (`cmux vm new`)"],
            [
              "Notifications",
              "Basic (per-pane)",
              "Persistent store with unread tracking, jump-to-unread, set-status/progress commands",
            ],
            ["Markdown panes", "Yes (beta)", "Yes"],
            ["Right sidebar panels", "Workspaces sidebar only", "5 panels: Files, Find, Vault, Feed, Dock"],
            ["SSH remote workspaces", "No", "Yes (auto-deployed Go daemon over scp/SSH)"],
            ["Per-directory config", "No (cut from scope)", "Yes (ancestor-walk) with trust modes"],
            ["Socket access control modes", "Single trust mode", "5 modes (off, cmuxOnly, automation, password, allowAll)"],
            ["Current version", "v0.2.16 (May 2026)", "v0.64.7 (May 19, 2026) — also tagged v1.38.1"],
            ["GitHub stars", "Early (under 100)", "17 500+"],
            ["First release", "Late 2025", "February 2026"],
            ["Pricing", "Free, MIT", "Free (GPL); commercial license available"],
          ]}
        />
      </CompareSection>

      <CompareSection id="decision-guide" title="Which one is right for you?">
        <p>
          The honest version: this is a clean either/or for most users. The
          two bullet lists below capture who each tool is genuinely built
          for. If you fit neither, you probably want something simpler
          (raw tmux, WezTerm, iTerm2).
        </p>
        <DecisionGuide
          left={{
            heading: "Choose Paneflow if",
            bullets: [
              "You run Linux as your primary OS (or you need cross-platform support across your team)",
              "You want a single static binary with no dual-license question",
              "You prefer a small, auditable Rust codebase and want to contribute",
              "You value upstream alacritty_terminal over a forked VT engine",
              "You are comfortable with v0.x tooling and you want to shape what ships next",
              "You eventually need a Windows build (planned, not shipping yet)",
            ],
          }}
          right={{
            heading: "Choose cmux if",
            bullets: [
              "You are macOS-only and want the most polished native experience today",
              "You need SSH remote workspaces (cmux is the only one shipping this)",
              "You need an embedded browser with Chrome/Firefox/Safari/Brave/Edge/Arc profile import",
              "You rotate through 15+ AI agents and want first-class integration for all of them",
              "You want a 17 500+ star community, hundreds of socket commands, and active commercial backing",
              "You want cloud VM provisioning, AppleScript scripting, or tmux compatibility built in",
              "You are comfortable with GPL-3.0 (or you will pay for the commercial license)",
            ],
          }}
        />
      </CompareSection>

      <CompareSection id="architecture" title="Architecture deep-dive">
        <p>
          <strong className="text-text">cmux</strong> is a native macOS
          application built on AppKit and SwiftUI. Terminal emulation is
          delegated to libghostty, the C library that powers the Ghostty
          terminal emulator, bridged into the Swift app through
          <code> GhosttyKit.xcframework</code>. Rendering goes through a
          custom <code>GhosttyMetalLayer</code> subclass of CAMetalLayer
          (Ghostty drives Metal directly, no MTKView). Pane layout uses
          Bonsplit, an N-ary tree layout library that exposes adjacency
          queries (<code>adjacentPane(to:direction:)</code>) and snapshots
          for session persistence.
        </p>
        <p>
          <strong className="text-text">Paneflow</strong> is built in pure
          Rust on top of GPUI, the same UI framework Zed uses. Terminal
          emulation is upstream <code>alacritty_terminal</code> 0.26 from
          crates.io - the public, stable Rust VT crate (no fork). The
          GPU layer is Blade (GPUI&rsquo;s renderer) over Vulkan on Linux
          and Metal on macOS. Pane layout is a hand-rolled N-ary tree
          designed for the four preset layouts
          (<em>even horizontal</em>, <em>even vertical</em>,
          <em> main-vertical</em>, <em>tiled</em>).
        </p>
        <p>
          On the IPC side, both projects expose a Unix socket. cmux runs
          two protocols in parallel on the same socket: V1 is a
          space-delimited text protocol
          (e.g. <code>new-workspace</code>, <code>send-key &lt;args&gt;</code>),
          V2 is newline-delimited JSON. The dispatcher inspects the first
          byte of each line to route. Combined surface area is several
          hundred commands covering windows, panes, sessions, notifications,
          and remote workspaces. Paneflow ships a single JSON-RPC 2.0
          protocol with roughly thirteen methods covering workspaces and
          panes - smaller surface, faster to learn, far less coverage.
          Catching up to cmux on the IPC API surface is on the Paneflow
          roadmap.
        </p>
      </CompareSection>

      <CompareSection id="pricing" title="Pricing">
        <p>
          Both projects are free to use. The licensing models differ:
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Paneflow</strong>: MIT
              license, no dual licensing, no commercial tier. Use it
              however you want, including embedding it inside commercial
              products.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">cmux</strong>: GPL-3.0 by
              default with a separate commercial license available for
              organizations that need non-copyleft terms. Pricing for
              the commercial license is not publicly listed; contact the
              maintainers.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection
        id="migrating"
        title="Migrating from cmux to Paneflow"
      >
        <p>
          You can move most of a cmux setup to Paneflow in about ten
          minutes. The config schemas are similar JSON shapes; the
          keystroke notation matches; the AI agent button concept is the
          same.
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Config file</strong>: copy
              your <code>~/.config/cmux/cmux.json</code> to
              <code> ~/.config/paneflow/paneflow.json</code> on Linux or
              <code> ~/Library/Application Support/paneflow/paneflow.json</code>
              on macOS. Rename keys that differ (see the
              {" "}<Link
                href="/docs/configuration/schema"
                className="text-text underline underline-offset-4 decoration-surface-border-hover"
              >
                schema reference
              </Link>).
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Keybindings</strong>: defaults
              match for the core actions
              (<code>Cmd/Ctrl+Shift+D</code> split horizontal,
              <code> Cmd/Ctrl+Shift+E</code> split vertical,
              <code> Alt+Arrow</code> focus, <code>Cmd/Ctrl+1-9</code>
              workspace switch). Custom overrides translate directly.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Sessions</strong>: not
              portable. On-disk session formats differ; you will need to
              recreate workspaces on first launch. Branch detection and
              CWD restoration kick in automatically after that.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">SSH workflows</strong>: no
              direct migration path. If you depend on
              <code> cmux ssh user@host</code>, stay on cmux until
              Paneflow ships remote workspace support.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="when-not" title="When NOT to choose Paneflow">
        <p>
          The honest, non-marketing list. cmux has shipped years of
          surface that Paneflow has not built yet. If any of these matter
          to you today, cmux is the better tool:
        </p>
        <ol className="space-y-3 text-sm">
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">1.</span>
            <span>
              <strong className="text-text">
                You need SSH remote workspaces.
              </strong>{" "}
              cmux ships an auto-deploying Go daemon (scp + SSH local-
              forward to a Unix socket) for this. Paneflow has no
              equivalent and it is not on the immediate roadmap.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">2.</span>
            <span>
              <strong className="text-text">
                You rotate through more than three AI agents.
              </strong>{" "}
              cmux ships first-class support for 15+ agents (Claude Code,
              Codex, Grok, OpenCode, Cursor, Copilot, Gemini, Antigravity,
              Rovo Dev, Hermes, CodeBuddy, Factory, Qoder, Amp, Pi, plus
              custom). Paneflow ships three today.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">3.</span>
            <span>
              <strong className="text-text">
                You want a command palette, AppleScript, or tmux
                compatibility.
              </strong>{" "}
              cmux has a fuzzy-search command palette, AppleScript
              scripting via <code>.sdef</code>, and a tmux compatibility
              shim (<code>capture-pane</code>, <code>pipe-pane</code>,
              <code> bind-key</code>, <code>paste-buffer</code>,
              <code> set-hook</code>) that lets tmux-aware tools target
              cmux. Paneflow has none of these.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">4.</span>
            <span>
              <strong className="text-text">
                You need an embedded browser inside the terminal.
              </strong>{" "}
              cmux&rsquo;s WKWebView panel imports profiles from Chrome,
              Firefox, Safari, Brave, Edge, and Arc. Paneflow does not
              plan a browser surface.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">5.</span>
            <span>
              <strong className="text-text">
                You need cloud VMs from the terminal.
              </strong>{" "}
              cmux ships integrated cloud VM provisioning
              (<code>cmux vm new</code>). Paneflow has no equivalent.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">6.</span>
            <span>
              <strong className="text-text">
                You want a several-hundred-command IPC surface.
              </strong>{" "}
              cmux exposes hundreds of commands over a dual V1 text + V2
              JSON socket protocol, covering windows, panes, sessions,
              notifications, and remote workspaces. Paneflow&rsquo;s
              JSON-RPC has thirteen methods today.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">7.</span>
            <span>
              <strong className="text-text">
                You want a richer side-panel surface or per-directory
                trust modes.
              </strong>{" "}
              cmux ships a right sidebar with five panels (Files, Find,
              Vault, Feed, Dock), persistent notification storage with
              unread tracking, ancestor-walk per-directory config, and
              five socket access control modes. Paneflow ships a single
              workspaces sidebar.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">8.</span>
            <span>
              <strong className="text-text">
                You need production-grade maturity now.
              </strong>{" "}
              cmux is at v0.64 with 17 500+ GitHub stars and active
              commercial backing. Paneflow is at v0.2.x with a small
              community. Config and IPC schemas may still shift between
              minor versions until v1.0.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">9.</span>
            <span>
              <strong className="text-text">
                You want the most polished native macOS feel.
              </strong>{" "}
              cmux&rsquo;s AppKit UI matches macOS conventions more
              tightly than GPUI&rsquo;s custom client-side decorations
              do today.
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
          . Curious about cmux instead?{" "}
          <a
            href="https://github.com/manaflow-ai/cmux"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
            rel="noopener noreferrer"
            target="_blank"
          >
            cmux is on GitHub
          </a>{" "}
          and worth a look - it solves a problem the Paneflow team
          respects deeply.
        </p>
      </CompareSection>
    </CompareLayout>
  );
}
