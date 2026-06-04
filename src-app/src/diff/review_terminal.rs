//! Diff review-terminal model (prd-ai-in-diff-2026-Q3.md, revised 2026-06-01).
//!
//! Human-in-the-loop by design: clicking Review picks one or more CLIs and opens
//! a REAL terminal pane per CLI in the branch's worktree, launches the CLI, and
//! PRE-FILLS its input with a compact review prompt (the user submits). No
//! headless ACP session — you see exactly what the agent does, in a real
//! terminal. See [[feedback-human-in-loop-no-headless]]. This module owns the
//! embedded terminal entity, CLI table, prompt builder, and shell-aware launch
//! request used by `DiffView`.

use gpui::{Entity, SharedString};

/// A review CLI running in a real terminal embedded under a diff column.
pub(crate) struct ReviewTerminal {
    pub(crate) label: SharedString,
    pub(crate) terminal: Entity<crate::terminal::TerminalView>,
}

/// A CLI coding agent Paneflow can launch in a terminal for a review. Unlike the
/// ACP layer (Claude Code + Codex only), the terminal path supports every CLI —
/// it just spawns the binary in a shell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReviewCli {
    ClaudeCode,
    Codex,
    OpenCode,
    Pi,
}

impl ReviewCli {
    /// All targets, in menu order.
    pub(crate) fn all() -> [ReviewCli; 4] {
        [Self::ClaudeCode, Self::Codex, Self::OpenCode, Self::Pi]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    /// Shell-aware command that clears the pane and launches the interactive
    /// CLI. Mirrors the existing pane launch buttons (`pane.rs`).
    pub(crate) fn launch_command(self, config: &paneflow_config::schema::PaneFlowConfig) -> String {
        crate::terminal::shell::clear_then(self.command(), config.default_shell.as_deref())
    }
}

/// Build the compact, human-in-loop review prompt to PRE-FILL into the CLI input.
/// The CLI runs in the worktree cwd, so it inspects the diff itself via git —
/// transparent (you see it run `git diff`) and tiny (no pasted diff). When
/// `adversarial`, ask it to play the skeptical second reviewer (used for the
/// 2nd CLI in a multi-CLI "second opinion").
pub(crate) fn build_cli_review_prompt(branch: &str, base: &str, adversarial: bool) -> String {
    let base = if base.trim().is_empty() {
        "the base branch".to_string()
    } else {
        base.to_string()
    };
    let lens = if adversarial {
        "Be a skeptical second reviewer: actively hunt for what a first pass would miss. "
    } else {
        ""
    };
    format!(
        "Review the changes this branch (`{branch}`) adds vs `{base}`, including uncommitted work. \
         Inspect the diff yourself with git (e.g. `git diff $(git merge-base HEAD {base})` plus \
         `git status`). {lens}Review ONLY the changed lines for bugs, security issues, regressions, \
         and broken invariants — skip style nits unless harmful. Give a one-line verdict (SAFE or \
         the top concern), then findings as `path:line [blocker|suggestion|nit] note`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_commands_are_distinct_and_bare() {
        let cmds: Vec<&str> = ReviewCli::all().iter().map(|cli| cli.command()).collect();
        assert_eq!(cmds.len(), 4);
        assert!(cmds.contains(&"claude"));
        assert!(cmds.contains(&"codex"));
        assert!(cmds.contains(&"opencode"));
        assert!(cmds.contains(&"pi"));
    }

    #[test]
    fn prompt_references_branch_base_and_git_not_pasted_diff() {
        let p = build_cli_review_prompt("feat/x", "develop", false);
        assert!(p.contains("feat/x"));
        assert!(p.contains("develop"));
        assert!(p.contains("git diff"));
        assert!(p.contains("path:line"));
        assert!(!p.contains("@@")); // no pasted diff
        assert!(!p.contains("skeptical second reviewer"));
    }

    #[test]
    fn empty_base_has_sensible_fallback() {
        let p = build_cli_review_prompt("feat/x", "", false);
        assert!(p.contains("the base branch"));
    }

    #[test]
    fn adversarial_adds_skeptic_framing() {
        let p = build_cli_review_prompt("feat/x", "develop", true);
        assert!(p.contains("skeptical second reviewer"));
    }
}
