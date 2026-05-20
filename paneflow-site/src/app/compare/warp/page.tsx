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
    question: "Is Paneflow a fork of Warp?",
    answer:
      "No. Warp is a Rust + Metal terminal built by Denver Technologies, Inc., open-sourced in April 2026 with a hybrid license (AGPL-3.0 for the client, MIT for the warpui_core and warpui crates). Paneflow is a separate Rust + GPUI codebase by Arthur Jean, MIT throughout. The two share no source code and ship under different business models (Warp has paid tiers + a closed-source backend; Paneflow is a single MIT binary).",
  },
  {
    question:
      "Both are Rust + GPU + cross-platform. What is the actual difference?",
    answer:
      "Incentive structure and agent architecture. Warp is AGPL-3.0 on the client but the server backend stays closed-source, Free-tier AI requires telemetry, and team features (Warp Drive, SSO, ZDR, shared credits) sit behind $20-$50 per-user-per-month tiers. Paneflow is MIT throughout, no login, no telemetry, no paid tier, no backend. Warp ships a built-in AI agent plus Oz (cloud orchestration); Paneflow only launches the CLI agents (Claude Code, Codex, OpenCode) you already use.",
  },
  {
    question:
      "Is Warp's source code actually public? I checked github.com/warpdotdev/Warp and saw 'currently closed-source.'",
    answer:
      "Yes, the source is public, on the master branch - which is a non-obvious twist. github.com/warpdotdev/Warp has two branches: main is configured as an issues-only repo with a stale LICENSE file that still reads 'currently closed-source,' while master holds the actual open-sourced Rust workspace (Cargo.toml, LICENSE-AGPL, LICENSE-MIT, full source tree). The April 2026 announcement is real. If you only check main, you will conclude Warp is still closed-source - check master.",
  },
  {
    question:
      "What does 'AGPL-3.0 client + MIT UI framework' mean for embedding?",
    answer:
      "Warp's repo splits the license at the crate boundary: the warpui_core and warpui crates are MIT, the rest of the codebase (the terminal core, the AI surface, the orchestration code) is AGPL-3.0-only per the workspace Cargo.toml. You can pull warpui into a closed-source product; you cannot pull the rest. Paneflow's MIT covers everything - the terminal core, GPUI integration, and IPC server are all MIT - so embedders do not have to track a license boundary inside the codebase.",
  },
  {
    question:
      "Why does Warp's Free tier require telemetry but Paneflow does not?",
    answer:
      "Different business models. Warp's Free tier is the funnel: telemetry feeds the AI product, the AI product generates conversion to paid tiers. Per Warp's own docs at docs.warp.dev: 'Telemetry must be enabled to use AI features on the Free plan, while paid plans can opt out at any time and continue using Warp, including AI.' Paneflow has no paid tier, no AI service to feed, and no business reason to collect telemetry. Login was also lifted from Warp in November 2024, but the telemetry gate on Free-tier AI remains in 2026.",
  },
  {
    question:
      "What about Oz? Is that the same as the in-terminal Warp AI agent?",
    answer:
      "No. Oz is a separate cloud-orchestration layer from the Build plan ($20/user/month): it spawns parallel coding agents server-side, with a 'handoff' that moves a session from the local terminal to Oz for more compute. The in-terminal Warp AI agent is the local interactive feature. Paneflow has neither - it launches local CLI agents in panes, no cloud layer, no orchestration service.",
  },
  {
    question:
      "Does Paneflow have team pricing or SSO like Warp's Business tier?",
    answer:
      "No. Paneflow is a single MIT binary with no team tier, no SSO, no SCIM, no team admin console, no per-seat billing. If you need any of those, Warp Business at $50/user/month is the right tool - it ships SAML SSO, enforced team-wide Zero Data Retention, shared credit pools, and central billing up to 50 seats. Paneflow is built for individual developers; if your org needs centralized terminal management with auth controls, look at Warp.",
  },
  {
    question: "Can I migrate my Warp settings to Paneflow?",
    answer:
      "There is no automatic path. Warp uses a YAML config plus cloud-synced settings; Paneflow uses a static JSON file at ~/.config/paneflow/paneflow.json on Linux. Plan ten minutes for a fresh setup: default shell, theme, keybindings, agent buttons. Workflows that depend on Warp Drive (the shared command-and-snippet library) do not have a Paneflow equivalent - if Warp Drive is core to your team workflow, stay on Warp.",
  },
];

export const metadata: Metadata = {
  title:
    "Paneflow vs Warp (2026): MIT local CLI host vs AGPL cloud-leaning agent platform",
  description:
    "Paneflow vs Warp: both Rust + GPU + cross-platform, both ship AI today. Paneflow is MIT, no login, no telemetry, no paid tier, launches the CLI agents you already use. Warp is AGPL-3.0 client + MIT UI, with paid tiers, a built-in agent, and a cloud-orchestration layer (Oz). Honest decision guide.",
  alternates: {
    canonical: "/compare/warp",
  },
  openGraph: {
    title: "Paneflow vs Warp (2026)",
    description:
      "MIT local-first indie vs AGPL-3.0 cloud-leaning agent platform with $20-$50 tiers. Architecture, license, telemetry, and decision guide for Claude Code, Codex, OpenCode workflows.",
    type: "article",
  },
};

export default function CompareWarpPage() {
  const jsonLd = buildCompareJsonLd({
    competitorName: "Warp",
    competitorSlug: "warp",
    headline: "Paneflow vs Warp (2026)",
    description:
      "MIT local-first Rust agent host vs AGPL-3.0 cloud-leaning Rust agent platform with paid team tiers. Both Rust, both GPU-accelerated, both ship AI - they diverge on incentive structure. Decision guide.",
    dateModified: DATE_MODIFIED,
    faq: FAQ,
  });

  return (
    <CompareLayout jsonLd={jsonLd}>
      <CompareHeader
        title="Paneflow vs Warp"
        tldr={
          <>
            Paneflow and Warp are both Rust + GPU + cross-platform
            terminals that ship AI today.{" "}
            <strong className="text-text">
              They diverge on incentive structure, not category.
            </strong>{" "}
            <strong className="text-text">Paneflow</strong>{" "}is MIT
            throughout, no login, no telemetry, no paid tier, no server
            backend - a single binary that launches the CLI agents
            (Claude Code, Codex, OpenCode) you already use, on
            Zed&rsquo;s GPUI engine.{" "}
            <strong className="text-text">Warp</strong> is AGPL-3.0 on
            the client (MIT for the warpui crates) with a closed-source
            backend, paid tiers at $20-$50 per user per month, a
            telemetry requirement on the Free tier for AI access, and a
            separate cloud-orchestration product called Oz. OpenAI is
            the founding sponsor of the new open-source repo. Pick
            Paneflow if you want a local-first, MIT, no-strings agent
            host. Pick Warp if you want a built-in AI agent, a shared
            team command library (Warp Drive), and an enterprise
            commercial path.
          </>
        }
      />

      <CompareSection
        id="context"
        title="A note on context"
      >
        <p>
          Warp open-sourced its client codebase in late April 2026 - a
          real shift from the pre-2026 closed-source SaaS model, and
          worth acknowledging honestly before comparing. Per
          Warp&rsquo;s own master/README.md{" "}
          <em>NOTE</em>{" "}block: &ldquo;OpenAI is the founding sponsor of
          the new, open-source Warp repository, and the new agentic
          management workflows are powered by GPT models.&rdquo;
          Anyone evaluating Warp should treat the open-source pivot as
          the headline change of 2026.
        </p>
        <p>
          One non-obvious twist: the source code lives on the{" "}
          <code>master</code> branch of{" "}
          <code>github.com/warpdotdev/Warp</code>, not on{" "}
          <code>main</code> (which is still an issues-only repo with a
          stale &ldquo;currently closed-source&rdquo; LICENSE file).
          The repo&rsquo;s actual <code>Cargo.toml</code>,{" "}
          <code>LICENSE-AGPL</code>, and <code>LICENSE-MIT</code> all
          sit on <code>master</code>. A reader who checks{" "}
          <code>main</code> alone will reach the wrong conclusion -
          worth flagging up front because it changes how you read the
          rest of this page. All Warp source-code references below
          point at <code>master</code> URLs.
        </p>
        <p>
          The open-source pivot does not erase the underlying business
          model. Warp&rsquo;s server backend stays closed-source, the
          Free tier gates AI behind a telemetry requirement, and paid
          tiers ($20-$50 per user per month) are the path to AI without
          telemetry and to team features (Warp Drive, SSO, Zero Data
          Retention). Paneflow has none of that machinery - the entire
          product is a single MIT binary.
        </p>
      </CompareSection>

      <CompareSection id="quick-comparison" title="Quick comparison">
        <p>
          Grouped in three zones - license &amp; data posture (where
          Paneflow leads), core terminal parity (where both are
          equivalent), and the agent + team surface (where Warp has
          shipped much more).
        </p>

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-6 mb-2">
          License &amp; data posture
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            [
              "License (client)",
              "MIT throughout",
              "AGPL-3.0 client + MIT for warpui crates",
            ],
            [
              "Backend",
              "n/a (no server)",
              "Closed-source server stays proprietary",
            ],
            ["Login required to use terminal", "No", "No (lifted Nov 2024)"],
            [
              "Telemetry required for AI on Free tier",
              "n/a (no Free tier; no AI service)",
              "Yes (Warp docs verbatim)",
            ],
            ["Per-seat pricing", "n/a", "Build $20/mo, Business $50/user/mo"],
            ["Paid tiers / credit caps", "n/a", "Free 75 cr/mo (150 first 2 mo), Build 1500/mo, Business 1500/user/mo"],
            ["Founding sponsor / commercial backing", "Indie (Arthur Jean)", "Denver Technologies, Inc.; OpenAI founding sponsor of OSS repo"],
            [
              "Embed in closed-source product",
              "Yes (MIT)",
              "warpui crates only (MIT); rest is AGPL-3.0",
            ],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          Core terminal parity
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            ["Language", "Rust", "Rust"],
            [
              "GPU stack",
              "GPUI/Blade over Vulkan + Metal",
              "Custom Rust + Metal-direct (forked Alacritty grid model, custom renderer)",
            ],
            ["OS support", "Linux + macOS (Windows planned)", "Linux + macOS + Windows GA"],
            ["VT emulator", "alacritty_terminal 0.26 (upstream)", "Forked from Alacritty grid model"],
            ["Pane layout + tabs", "Yes", "Yes"],
            ["Session restore", "Yes", "Yes"],
            ["Themes", "Bundled + JSON override", "Bundled + YAML override (cloud-synced)"],
            ["Cross-platform breadth", "Linux + macOS now", "Linux + macOS + Windows now"],
          ]}
        />

        <h3 className="text-xs font-mono uppercase tracking-wider text-text-subtle mt-8 mb-2">
          Agent &amp; team surface
        </h3>
        <CompareTable
          headers={["", "Paneflow", "Warp"]}
          rows={[
            [
              "AI agent model",
              "Launches external CLI agents (Claude Code, Codex, OpenCode)",
              "Built-in AI agent + supports launching CLI agents",
            ],
            ["Cloud orchestration layer (parallel server-side agents)", "n/a", "Oz (available from Build $20/mo)"],
            ["Shared team command library", "n/a", "Warp Drive"],
            ["SAML / SSO", "n/a", "Business tier ($50/user/mo)"],
            ["SCIM / team admin console", "n/a", "Business tier"],
            ["Zero Data Retention enforcement", "n/a (no data collected)", "Business tier (team-wide enforced)"],
            ["Shared credits pool", "n/a", "Business tier"],
            ["Commercial support contract", "n/a", "Available via Enterprise tier"],
          ]}
        />

        <p className="text-xs text-text-subtle mt-8 leading-relaxed">
          <strong className="text-text-muted">Versions:</strong> Paneflow
          v0.2.16 (May 2026), first commit 2026-04-01. Warp client
          open-sourced 2026-04-28 (Denver Technologies, Inc., copyright
          2020-2026).{" "}
          <strong className="text-text-muted">Pricing:</strong> Paneflow
          free MIT. Warp Free 75 credits/month (150 for the first two
          months) + Build $20/month + Business $50/user/month +
          Enterprise custom.
        </p>
      </CompareSection>

      <CompareSection id="decision-guide" title="Which one is right for you?">
        <p>
          The honest version: the choice is mostly about how you want
          your terminal to relate to a vendor backend. If you want zero
          relationship with a backend, Paneflow. If you want a polished
          built-in AI agent + a team-scale path, Warp.
        </p>
        <DecisionGuide
          left={{
            heading: "Choose Paneflow if",
            bullets: [
              "You want local-first - no login, no telemetry, no server backend, no paid tier ever",
              "You want MIT throughout, so embedding terminal code in a closed-source product is straightforward",
              "Your agent workflow is launching the CLI agents (Claude Code, Codex, OpenCode) you already use, not vendoring a chat surface",
              "You prefer an indie maintainer aligned with the Zed philosophy over a venture-backed product with paid tiers",
              "You do not need shared team command libraries, SSO, or commercial support",
              "You want a single binary you can audit, fork, and ship without a license boundary inside the codebase",
            ],
          }}
          right={{
            heading: "Choose Warp if",
            bullets: [
              "You want a polished built-in AI agent inside the terminal, not just CLI agents launched in a pane",
              "You want a shared team command library (Warp Drive) and cloud-synced settings",
              "You need SAML SSO, SCIM, team admin features, or central billing",
              "You want Oz cloud orchestration to spawn parallel coding agents server-side at scale",
              "You are comfortable with AGPL-3.0 on the client (or only need the MIT warpui crates) and accept a closed-source backend",
              "You want OpenAI as the founding sponsor and direct commercial support paths via Enterprise tier",
              "You will pay $20-$50 per user per month for AI credits, ZDR enforcement, and team features",
            ],
          }}
        />
      </CompareSection>

      <CompareSection id="architecture" title="Architecture deep-dive">
        <p>
          <strong className="text-text">Warp</strong> is a Rust
          workspace with crates including <code>app</code>,{" "}
          <code>warp_terminal</code>, <code>warpui</code>,{" "}
          <code>warp_completer</code>, <code>editor</code>,{" "}
          <code>markdown_parser</code>, and others - the workspace{" "}
          <code>Cargo.toml</code> declares{" "}
          <code>license = &quot;AGPL-3.0-only&quot;</code>{" "}with the
          warpui crates carved out as MIT. The renderer is custom: Warp
          forked Alacritty&rsquo;s grid and VT model but wrote its own
          GPU renderer that talks to Metal directly on macOS, via about
          200 lines of custom shaders. Alongside the local app, Warp
          ships a closed-source server backend that
          powers AI features, Warp Drive, and cloud-synced settings;
          Oz, the cloud orchestration product, runs agents server-side
          and connects via &ldquo;handoff&rdquo; from the local
          terminal.
        </p>
        <p>
          <strong className="text-text">Paneflow</strong>{" "}is a pure-Rust
          application built on Zed&rsquo;s GPUI engine - no language
          boundary between UI and terminal, no closed-source backend,
          no AI service to feed. Terminal emulation is upstream{" "}
          <code>alacritty_terminal</code> 0.26 from crates.io (where
          Warp uses its own fork of the grid model). The GPU layer is
          GPUI&rsquo;s Blade renderer over Vulkan on Linux and Metal on
          macOS. The agent surface is intentionally thin: Paneflow
          provides workspaces, panes, branch detection, and three
          first-class CLI agent buttons (Claude Code, Codex, OpenCode).
          Any binary you can run in a shell runs in a Paneflow pane;
          unlike Warp, there is no vendored multi-vendor chat. No
          telemetry, no analytics service, no login.
        </p>
        <p>
          The IPC surfaces are also instructive. Warp talks to its
          backend over GraphQL (the <code>graphql</code>{" "}crate ships in
          the workspace) plus a telemetry pipeline that is required for
          Free-tier AI per Warp&rsquo;s own docs. Paneflow exposes a
          single JSON-RPC 2.0 server over a Unix socket at{" "}
          <code>$XDG_RUNTIME_DIR/paneflow/paneflow.sock</code> with
          roughly thirteen methods - no telemetry, no analytics. For
          most developers the difference is invisible; for one who
          cares about local-first posture, it is the whole product.
        </p>
      </CompareSection>

      <CompareSection id="pricing" title="Pricing">
        <p>The licensing and pricing models differ sharply.</p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Paneflow</strong>: MIT,
              Copyright (c) 2026 Arthur Jean. Single free tier, no
              paid plans, no per-seat pricing, no AI credits, no
              telemetry. Embed it in commercial products without
              concerns.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Warp</strong>: hybrid
              license (AGPL-3.0 client + MIT warpui crates), Copyright
              (c) 2020-2026 Denver Technologies, Inc. Four tiers:
              Free ($0, 75 AI credits/month after a 2-month bump,
              telemetry required for AI), Build ($20/month, 1 500
              credits + BYOK), Business ($50/user/month, SSO + enforced
              team-wide Zero Data Retention + shared credits), and
              Enterprise (custom, with commercial support).
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection
        id="migrating"
        title="Migrating from Warp to Paneflow"
      >
        <p>
          There is no automatic path. Warp uses a YAML config plus
          cloud-synced settings tied to your Warp account; Paneflow
          uses a static JSON file at{" "}
          <code>~/.config/paneflow/paneflow.json</code> on Linux and{" "}
          <code>~/Library/Application Support/paneflow/paneflow.json</code>
          {" "}on macOS. Plan ten minutes for a fresh setup: default
          shell, theme, keybindings, agent buttons. Three Warp features
          do not have Paneflow equivalents and the migration plan is
          &ldquo;keep using Warp for those&rdquo; rather than
          &ldquo;recreate locally:&rdquo;
        </p>
        <ul className="space-y-2.5 text-sm">
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Warp Drive</strong>:
              shared team command-and-snippet library. No Paneflow
              equivalent. If Warp Drive is core to your team, stay on
              Warp.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Built-in AI agent</strong>:
              Paneflow does not ship a vendored chat surface. The
              migration is to use the CLI agent of your choice (Claude
              Code, Codex, OpenCode) launched as a pane.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">Oz cloud orchestration</strong>:
              no Paneflow equivalent. If you need parallel server-side
              agents at scale, Oz is the right tool.
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="text-text-muted/60 select-none mt-0.5">-</span>
            <span>
              <strong className="text-text">SSO + team admin</strong>:
              no Paneflow equivalent. Paneflow is built for individual
              developers, not for orgs that need centralized terminal
              management.
            </span>
          </li>
        </ul>
      </CompareSection>

      <CompareSection id="when-not" title="When NOT to choose Paneflow">
        <p>
          The honest dealbreakers. If any of the five below matters to
          you, Warp is the right tool today - no point fighting it:
        </p>
        <ol className="space-y-3 text-sm">
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">1.</span>
            <span>
              <strong className="text-text">
                You want a vendored, built-in AI agent inside the
                terminal.
              </strong>{" "}
              Warp ships a polished in-terminal AI chat - Paneflow
              only launches external CLI agents (Claude Code, Codex,
              OpenCode) as panes. If your workflow is &ldquo;chat
              about my terminal output without leaving the terminal
              app,&rdquo; Warp is the right fit.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">2.</span>
            <span>
              <strong className="text-text">
                You want a shared team command library (Warp Drive).
              </strong>{" "}
              Warp Drive is the unique product feature that Paneflow
              cannot replicate without a backend. If your team depends
              on shared commands, snippets, and notebooks, Warp is the
              right tool.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">3.</span>
            <span>
              <strong className="text-text">
                You need SAML SSO, SCIM, or central team admin.
              </strong>{" "}
              Warp Business at $50/user/month ships these for orgs
              that need centralized auth, audit, and provisioning.
              Paneflow has no team tier and no admin console.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">4.</span>
            <span>
              <strong className="text-text">
                You want a commercial support contract.
              </strong>{" "}
              Warp Enterprise offers paid support; Paneflow is an
              indie project with no commercial-support track. If you
              need a phone number to call when something breaks, Warp
              is the right fit.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="text-text-muted/60 select-none mt-0.5">5.</span>
            <span>
              <strong className="text-text">
                You want cloud-synced settings.
              </strong>{" "}
              Warp syncs settings, themes, and workflows through your
              account. Paneflow stores everything in a local JSON
              file - portable via git or dotfiles, but no built-in
              cloud sync.
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
          . Curious about Warp instead?{" "}
          <a
            href="https://github.com/warpdotdev/Warp/tree/master"
            className="text-text underline underline-offset-4 decoration-surface-border-hover"
            rel="noopener noreferrer"
            target="_blank"
          >
            Warp&rsquo;s open-source code is on GitHub
          </a>{" "}
          (master branch, not main) - the open-source pivot was the
          headline change of 2026 and worth respecting on principle,
          even if Paneflow lands on the other side of the local-vs-cloud
          tradeoff.
        </p>
      </CompareSection>
    </CompareLayout>
  );
}
