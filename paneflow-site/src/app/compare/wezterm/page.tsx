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
    question: "Is Paneflow a fork of WezTerm?",
    answer:
      "No. Paneflow and WezTerm are independent Rust codebases that share no source. WezTerm started in late 2017 and is maintained by Wez Furlong (~98% of the ~7 400 commits); Paneflow started in April 2026 and is built on Zed's GPUI engine with upstream alacritty_terminal as the VT layer. The two projects are architectural peers - both pure Rust, both GPU-accelerated, both MIT - but they aim at different purposes.",
  },
  {
    question:
      "Both are Rust + GPU + MIT. What is the actual difference?",
    answer:
      "Purpose. WezTerm is a highly configurable terminal emulator that you script in Lua. Paneflow is an agent-first workspace where Claude Code, Codex, and OpenCode appear as first-class panes with branch-aware workspaces, dev-server port detection, and session restore. WezTerm has none of that built in; Paneflow does not aim to compete with WezTerm on Lua-driven extensibility. If you want a configurable terminal, WezTerm wins. If you want an agent workspace, Paneflow does.",
  },
  {
    question: "Does WezTerm have any AI agent integration today?",
    answer:
      "No native integration. WezTerm's repo contains no CLAUDE.md, no AGENTS.md, no .cursorrules, no AI dependency in wezterm-gui/Cargo.toml. You can spawn any CLI agent (Claude Code, Codex, OpenCode) from a WezTerm pane with a keybinding by writing Lua, but that is user configuration rather than a product feature. Paneflow ships dedicated UI buttons and a workspace model designed around running agents in parallel.",
  },
  {
    question:
      "Why pick Paneflow if WezTerm has 26 k stars and an eight-year codebase?",
    answer:
      "If you need an agent-first workspace today (Paneflow ships that, WezTerm does not), or you want active 2026 development (Paneflow shipped its first stable in April 2026 with weekly minor releases; WezTerm's latest tagged stable is 20240203-110809, a 15+ month gap, even though main is still receiving commits), or you want a smaller config surface (JSON vs Lua), Paneflow is the better fit. If you want a fully configurable terminal you can script in Lua and run on FreeBSD, WezTerm is the better fit.",
  },
  {
    question: "Can I migrate my WezTerm Lua config to Paneflow?",
    answer:
      "There is no automatic path. WezTerm uses a Lua script at ~/.wezterm.lua that returns a config table; Paneflow uses a static JSON file at ~/.config/paneflow/paneflow.json on Linux. Lua functions and event handlers cannot be expressed in JSON. The migration is a fresh setup that takes about ten minutes - default shell, theme, keybindings - rather than a translation.",
  },
  {
    question: "Does Paneflow run on Windows or FreeBSD like WezTerm does?",
    answer:
      "Not yet. WezTerm ships builds for Linux, macOS, Windows, and FreeBSD. Paneflow ships Linux and macOS today; Windows is planned (native port, not WSL) and has no shipping ETA. FreeBSD and NetBSD are not on Paneflow's roadmap. If cross-BSD support or a current Windows build is non-negotiable, WezTerm is the better tool.",
  },
  {
    question:
      "Does Paneflow have a built-in SSH multiplexer like WezTerm's mux?",
    answer:
      "No. WezTerm ships its own multiplexer protocol with SSH: and SSHMUX: domains - you can attach to a remote WezTerm and have your session survive disconnects without running tmux. Paneflow does not ship a multiplexer. If you need persistent remote sessions, use tmux or zellij inside Paneflow panes, or stay on WezTerm.",
  },
  {
    question: "Is WezTerm still actively maintained?",
    answer:
      "Yes on the development branch, but on a slow stable cadence. The latest tagged release at github.com/wezterm/wezterm is 20240203-110809-5046fc22, dated 2024-02-03 - 15+ months before this comparison was written. The main branch is active (last push 2026-05-01), and rolling-release packagers track main. Users who track tagged releases are still on the February 2024 build.",
  },
];

export const metadata: Metadata = {
  title:
    "Paneflow vs WezTerm (2026): agent-first workspace vs scriptable Rust terminal",
  description:
    "Paneflow vs WezTerm: both Rust + GPU + MIT. Paneflow is an agent-first workspace with first-class Claude Code, Codex, OpenCode panes. WezTerm is the configurable Rust terminal you script in Lua. Honest decision guide, architecture, FAQ.",
  alternates: {
    canonical: "/compare/wezterm",
  },
  openGraph: {
    title: "Paneflow vs WezTerm (2026)",
    description:
      "Agent-first Rust workspace vs scriptable Lua-configured Rust terminal. Performance, architecture, license, and decision guide for Claude Code, Codex, OpenCode workflows.",
    type: "article",
  },
};

export default function CompareWeztermPage() {
  const jsonLd = buildCompareJsonLd({
    competitorName: "WezTerm",
    competitorSlug: "wezterm",
    headline: "Paneflow vs WezTerm (2026)",
    description:
      "Agent-first Rust workspace vs scriptable Rust terminal emulator. Both Rust, both GPU-accelerated, both MIT - they diverge on purpose. Architecture, feature, and pricing comparison.",
    dateModified: DATE_MODIFIED,
    faq: FAQ,
  });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader
        title="Paneflow vs WezTerm"
        tldr={
          <>
            Paneflow and WezTerm are architectural peers: both pure Rust,
            both GPU-accelerated, both MIT, both built by an indie
            maintainer.{" "}
            <strong className="text-text">
              They diverge on purpose.
            </strong>{" "}
            <strong className="text-text">Paneflow</strong>{" "}is an
            agent-first workspace - Claude Code, Codex, and OpenCode are
            first-class panes with branch-aware workspaces, dev-server port
            detection, and session restore, on Zed&rsquo;s GPUI engine with
            sub-200&nbsp;ms cold start and sub-4&nbsp;ms keystroke-to-pixel
            latency.{" "}
            <strong className="text-text">WezTerm</strong> is a highly
            configurable Rust terminal emulator with a Lua scripting
            surface, a built-in SSH multiplexer, and cross-platform reach
            (Linux, macOS, Windows, FreeBSD). Pick Paneflow if you want an
            agent workspace that gets out of the way. Pick WezTerm if you
            want a terminal you can program in Lua and run on FreeBSD.
          </>
        }
      />

      <CompareSection
        id="context"
        title="A note on context"
      >
        <p>
          WezTerm is the closest architectural peer to Paneflow. Same
          language (Rust), same renderer family (Paneflow uses GPUI&rsquo;s
          Blade over Vulkan/Metal; WezTerm uses wgpu 25.0.2, the same
          family), same MIT license, same indie-maintainer story (Wez
          Furlong holds ~98% of WezTerm&rsquo;s commits). The two projects
          could share a lineage page; instead they diverge sharply on
          purpose.
        </p>
        <p>
          WezTerm is a configurable terminal emulator. Its identity is
          &ldquo;a terminal where you can script everything in Lua.&rdquo;
          Paneflow is an agent workspace - a single binary that opens with
          panes, workspaces, and a UI for launching CLI coding agents,
          designed around the workflow of running Claude Code, Codex, and
          OpenCode side by side. WezTerm has no concept of an AI agent
          pane, no workspace-per-project model with branch detection, no
          dev-server port banner. Paneflow has no Lua scripting surface,
          no built-in SSH multiplexer, no FreeBSD build. The rest of this
          page maps the tradeoff.
        </p>
      </CompareSection>

      <CompareSection id="quick-comparison" title="Quick comparison">
        <p>
          Grouped in three zones - performance &amp; agent surface (where
          Paneflow leads), core terminal parity (where both are
          equivalent), and the configuration &amp; ecosystem (where
          WezTerm has shipped more surface).
        </p>

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          Performance &amp; agent surface
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Cold start", "<200 ms", "not published"],
            ["Keystroke-to-pixel latency", "<4 ms", "not published"],
            [
              "AI agents (dedicated UI buttons)",
              "3 (Claude Code, Codex, OpenCode)",
              "n/a",
            ],
            [
              "AI agents (any CLI)",
              "Unlimited (launch any binary in a pane)",
              "Unlimited via Lua spawn, no first-class UI",
            ],
            ["Branch-aware workspace badges", "Yes", "n/a"],
            ["Dev-server port detection", "Yes", "n/a"],
            [
              "Latest stable release",
              "v0.2.16 (May 2026, active weekly)",
              "20240203-110809 (Feb 2024, 15+ month gap; main is active)",
            ],
            [
              "Workspace + branch persistence",
              "Yes (workspaces, layouts, CWD on disk)",
              "Workspaces concept exists, no branch awareness",
            ],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          Core terminal parity
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Language", "Rust", "Rust"],
            [
              "GPU stack",
              "GPUI/Blade over Vulkan + Metal",
              "wgpu 25.0.2 over Vulkan / Metal / DX12",
            ],
            ["License", "MIT", "MIT"],
            ["VT emulator", "alacritty_terminal 0.26 (upstream)", "Built-in"],
            ["Pane layout", "N-ary tree, 4 preset layouts", "Tabs + freeform splits"],
            ["Session restore", "Yes (workspaces + CWD)", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + Lua override"],
            ["Mouse selection + hyperlinks", "Yes", "Yes"],
            ["True color / ligatures", "Yes", "Yes"],
            ["Maintainer model", "Indie (Arthur Jean)", "Indie (Wez Furlong, ~98% commits)"],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          Configuration &amp; ecosystem
        </h3>
        <CompareTable
          headers={["", "Paneflow", "WezTerm"]}
          rows={[
            ["Config language", "JSON (static)", "Lua (full scripting surface)"],
            ["Config path", "~/.config/paneflow/paneflow.json", "~/.wezterm.lua"],
            [
              "Event handlers in config",
              "n/a",
              "Yes (Lua callbacks, format-tab-title, mux-startup, etc.)",
            ],
            [
              "Built-in SSH multiplexer",
              "n/a",
              "Yes (SSH: and SSHMUX: domains)",
            ],
            ["GitHub stars", "Small, growing", "26 210 (fetched 2026-05-20)"],
            ["Codebase age", "~1 month (first commit April 2026)", "~8 years (first commit Dec 2017)"],
            ["Platform: Linux", "Yes", "Yes"],
            ["Platform: macOS", "Yes", "Yes"],
            ["Platform: Windows", "Planned", "Yes"],
            ["Platform: FreeBSD", "n/a", "Yes"],
          ]}
        />

        <p className="text-xs text-text-subtle mt-8 leading-relaxed">
          <strong className="text-text-muted">Versions:</strong> Paneflow
          v0.2.16 (May 2026), first commit 2026-04-01, v0.1.0 tagged
          2026-04-16. WezTerm latest stable 20240203-110809-5046fc22 (Feb
          2024), first commit 2017-12-07 (c53ca64c). WezTerm&rsquo;s main
          branch is active (last push 2026-05-01) but no stable release
          has been cut since February 2024.{" "}
          <strong className="text-text-muted">Pricing:</strong> both free.
          Both MIT.
        </p>
      </CompareSection>

      <CompareSection id="decision-guide" title="Which one is right for you?">
        <p>
          The honest version: WezTerm is the right answer if you want a
          terminal you script. Paneflow is the right answer if you want a
          workspace where agents are first-class. The bullets below
          capture the realistic picks for each side.
        </p>
        <DecisionGuide
          left={{
            heading: "Choose Paneflow if",
            bullets: [
              "You run multiple CLI coding agents (Claude Code, Codex, OpenCode) and want first-class UI buttons + a workspace model designed for it",
              "You care about performance - Paneflow boots in <200 ms with <4 ms keystroke-to-pixel latency on Zed's GPUI rendering pipeline",
              "You want branch-aware workspaces, dev-server port detection, and session restore baked in (no Lua scripting required)",
              "You prefer a static JSON config you can read at a glance over a Lua scripting surface with full programmability",
              "You want fresh active development - April 2026 first stable, weekly minor releases through May",
              "You back an indie dev who uses Paneflow every day and ships frequently",
            ],
          }}
          right={{
            heading: "Choose WezTerm if",
            bullets: [
              "You want to script your terminal in Lua - format tab titles, react to mux events, build custom keymaps with logic",
              "You need a built-in SSH multiplexer (SSH: and SSHMUX: domains) so your session survives disconnects without tmux",
              "You run FreeBSD or want a Windows build today (Paneflow Windows is planned, not shipped)",
              "You value a 26 k+ star community with 8 years of accumulated themes, plugins, and Stack Overflow answers",
              "You do not run AI coding agents and want a terminal that stays out of the AI conversation entirely",
              "You prefer wez's Lua-driven design philosophy where everything is overridable in user code",
            ],
          }}
        />
      </CompareSection>

      <CompareSection id="architecture" title="Architecture deep-dive">
        <p>
          <strong className="text-text">WezTerm</strong> is a Rust
          workspace crate with a GUI subcrate (<code>wezterm-gui</code>)
          that consumes <code>wgpu 25.0.2</code> and <code>mlua</code>.
          The renderer dispatches through wgpu to whichever backend the
          host provides: Vulkan on Linux, Metal on macOS, DX12 on Windows
          - cross-API without forking. Configuration is a Lua script
          (<code>~/.wezterm.lua</code>) that returns a config table; the
          script runs in-process via mlua, so any Lua expression
          (callbacks, format functions, mux event handlers) is allowed.
          The terminal grid and VT state machine are built in; the SSH
          multiplexer is a separate WezTerm protocol that connects two
          WezTerm processes over SSH.
        </p>
        <p>
          <strong className="text-text">Paneflow</strong>{" "}is also a Rust
          workspace, also GPU-accelerated, but the integration story is
          tighter and the configuration story is leaner. UI is built on
          GPUI - Zed&rsquo;s UI framework - so the keystroke pipeline
          stays in pure Rust from <code>KeyDownEvent</code> through to
          paint. Terminal emulation is upstream{" "}
          <code>alacritty_terminal</code> 0.26 from crates.io, the
          public Rust VT crate, not a fork. The GPU layer is Blade
          (GPUI&rsquo;s renderer) over Vulkan on Linux and Metal on
          macOS. Configuration is static JSON at{" "}
          <code>~/.config/paneflow/paneflow.json</code>; there is no
          scripting surface and no event-handler hook, by design - the
          goal is a config you can read top-to-bottom in a minute.
        </p>
        <p>
          The two projects also diverge on what the &ldquo;product&rdquo;
          is. WezTerm is a terminal you spend time configuring to fit
          your workflow. Paneflow is a workspace you launch and use
          immediately with three preset AI agent buttons, branch
          detection, and four pane layouts already wired up - the
          tradeoff is that there is no Lua escape hatch when Paneflow
          does not do what you want yet.
        </p>
      </CompareSection>

      <CompareSection id="pricing" title="Pricing">
        <p>
          Both projects are free, both MIT, both with no commercial tier.
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Paneflow</strong>: MIT,
              Copyright (c) 2026 Arthur Jean. No dual licensing. Embed it
              in commercial products without questions.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">WezTerm</strong>: MIT,
              Copyright (c) 2018-Present Wez Furlong. Same embedding
              freedom; no commercial license is offered (none is needed).
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection
        id="migrating"
        title="Migrating from WezTerm to Paneflow"
      >
        <p>
          There is no clean translation between WezTerm&rsquo;s Lua
          script and Paneflow&rsquo;s JSON. The install is a fresh
          setup that takes about ten minutes. The good news: Paneflow
          ships with sensible defaults for the things WezTerm makes you
          configure (workspace layout, agent buttons, theme), so the
          empty-config experience is closer to &ldquo;ready to use&rdquo;
          than to WezTerm&rsquo;s.
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Shell + theme</strong>: set
              <code> default_shell</code> and <code>theme</code> in{" "}
              <code>~/.config/paneflow/paneflow.json</code>. The two
              bundled themes are <em>One Dark</em> (default) and{" "}
              <em>Paneflow Light</em>.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Keybindings</strong>: see
              the{" "}
              <Link
                href="/docs/configuration/schema"
                className="text-text underline underline-offset-4 decoration-surface-border-hover"
              >
                schema reference
              </Link>
              . Default actions cover splits, focus, workspaces. If you
              have a Lua keymap with logic, that logic does not port -
              you will pick the closest static binding.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Lua event handlers</strong>:
              no equivalent in Paneflow. If you rely on
              <code> format-tab-title</code>, <code>mux-startup</code>, or
              other Lua callbacks, stay on WezTerm.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">SSH multiplexer</strong>:
              Paneflow does not ship a mux. Run tmux or zellij in a
              Paneflow pane, or keep WezTerm for remote sessions.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="when-not" title="When NOT to choose Paneflow">
        <p>
          The honest dealbreakers. If any of the five below matters to
          you, WezTerm is the right tool today - no point fighting it:
        </p>
        <ol className="space-y-3 text-sm">
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">1.</span>
            <span>
              <strong className="text-text">
                You want a Lua scripting surface.
              </strong>{" "}
              WezTerm lets you express keymap logic, dynamic tab titles,
              mux event hooks, and full callbacks in Lua. Paneflow uses
              static JSON with no scripting hook. If you have a Lua
              config you love, you will hate Paneflow.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">2.</span>
            <span>
              <strong className="text-text">
                You need a built-in SSH multiplexer.
              </strong>{" "}
              WezTerm&rsquo;s SSH: and SSHMUX: domains keep your session
              alive across disconnects without tmux. Paneflow has no
              multiplexer and no current plan to ship one.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">3.</span>
            <span>
              <strong className="text-text">
                You run Windows or FreeBSD.
              </strong>{" "}
              WezTerm ships builds for Linux, macOS, Windows, and FreeBSD
              today. Paneflow ships Linux and macOS; Windows is in the
              roadmap with no shipping ETA, FreeBSD is not on the roadmap.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">4.</span>
            <span>
              <strong className="text-text">
                You want a large mature ecosystem.
              </strong>{" "}
              WezTerm has 26 k+ GitHub stars, eight years of accumulated
              themes, plugins, and Stack Overflow answers. Paneflow is one
              month into its first public stable - the answers you need
              may not be on Google yet.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">5.</span>
            <span>
              <strong className="text-text">
                You do not run AI coding agents.
              </strong>{" "}
              Paneflow&rsquo;s entire reason for existing is the agent
              workspace. If you do not use Claude Code, Codex, OpenCode,
              or similar, you are using maybe 30% of the product and
              paying for the rest. WezTerm is the better fit for a
              traditional terminal workflow.
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
          . Curious about WezTerm instead?{" "}
          <a
            href="https://github.com/wezterm/wezterm"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
            rel="noopener noreferrer"
            target="_blank"
          >
            WezTerm is on GitHub
          </a>{" "}
          - it is a genuinely excellent Rust terminal that solves a
          different problem from Paneflow, and we recommend it for any
          workflow where Lua scripting is the point.
        </p>
      </CompareSection>
    </CompareLayout>
  );
}
