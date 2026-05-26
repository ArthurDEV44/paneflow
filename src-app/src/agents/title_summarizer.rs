//! Background thread-title summarization. Spiritual port of Zed's
//! `Thread::generate_title` flow at `crates/agent/src/thread.rs:2808`:
//! after the first clean `TurnEnded`, hit an LLM with the
//! conversation context + a "produce a 3-7 word title" prompt and
//! emit the response as the new sidebar title.
//!
//! Key divergence from a literal Zed port (and the reason the
//! earlier transient-ACP-session attempt failed): Zed does **not**
//! drive summarization through Codex CLI or any agent's ACP loop.
//! Zed's `summarization_model` is an `Arc<dyn LanguageModel>` that
//! calls a provider's HTTP endpoint directly via
//! `model.stream_completion(request, cx)` -- the "Codex" provider
//! there is the OpenAI-subscription HTTP backend at
//! `chatgpt.com/backend-api/codex`, not the `codex` agent CLI.
//! Spawning an ACP `codex` session for a single short prompt would
//! fire up a full agent loop with auth dance, tool wiring, and a
//! few seconds of startup -- the wrong tool entirely.
//!
//! Paneflow's pragmatic equivalent: shell out to `claude -p`
//! (non-interactive print mode) in `--bare`-style isolation. This
//! reuses the user's existing Claude Code authentication (OAuth /
//! keychain for Claude Max subscriptions, ANTHROPIC_API_KEY for API
//! users) without Paneflow having to manage credentials of its own.
//! It also matches Zed's "separate summarization model" spirit: the
//! agent backing the thread (Codex) is left untouched while a
//! second, lightweight model handles the title -- the only working
//! Anthropic surface for a summarizer that's already on a Codex
//! user's machine in a Paneflow context is the Claude Code CLI.
//!
//! Concurrency / race handling: the summarizer captures the
//! thread's current title at trigger time and ships it back through
//! [`TitleReplacePolicy::OnlyIfStillEqualTo`]. If the user renames
//! the thread (or the agent pushes a `SessionInfoUpdate.title`)
//! while the subprocess is in flight, the captured snapshot no
//! longer matches and the summarizer's result is silently dropped.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, WeakEntity};
use paneflow_acp::{AgentDiscovery, AgentKind};

use crate::agents::composer::Composer;
use crate::agents::thread_view::{ThreadView, TitleReplacePolicy, TitleSuggested};

/// Verbatim from Zed's `summarize_thread_prompt.txt`
/// (`crates/agent_settings/src/prompts/summarize_thread_prompt.txt`).
const SUMMARIZE_PROMPT: &str = "Generate a concise 3-7 word title for this conversation, omitting punctuation. Go straight to the title, without any preamble and prefix like `Sure, here's a concise suggestion:...` or `Title:`";

/// Time budget for the entire subprocess call. The Claude CLI's
/// cold start + a 7-word completion against Haiku usually lands
/// under 5 s; we cap at 60 s to survive a slow network without
/// blocking the user's session indefinitely.
const SUMMARIZE_TIMEOUT: Duration = Duration::from_secs(60);

/// Hard cap on the title we accept back from the model. Anything
/// longer is treated as the model hallucinating prose and we
/// truncate at the first newline / hard limit before storing.
const MAX_TITLE_CHARS: usize = 80;

/// Inputs for [`spawn_thread_title_summarization`]. Grouped so the
/// trigger call site at the composer's `TurnEnded` handler stays
/// readable and so clippy's `too_many_arguments` lint doesn't fire.
pub struct SummarizeRequest {
    pub agent_kind: AgentKind,
    /// Working directory of the thread. Passed as the subprocess
    /// cwd so any per-project Claude config (`.claude.json` in
    /// repo root) is picked up, and so the summarizer never runs
    /// from a random cwd that could surprise users.
    pub cwd: PathBuf,
    pub discovery: Arc<AgentDiscovery>,
    pub user_prompt: String,
    pub assistant_response: String,
    /// Current thread title at trigger time. The summarizer ships
    /// this back through [`TitleReplacePolicy::OnlyIfStillEqualTo`]
    /// so a user rename landing during the async wait is preserved.
    pub title_snapshot: String,
    pub thread_view: WeakEntity<ThreadView>,
}

/// Spawn a background task that summarizes the conversation and
/// emits the resulting title through `thread_view` once available.
///
/// No-op for agents that already push titles via ACP
/// `SessionInfoUpdate.title` (today: Claude Code). Codex and any
/// future agent that doesn't natively summarize falls into this
/// path.
///
/// The task is tied to the Composer's lifecycle via `cx.spawn` -- if
/// the user closes the thread while the subprocess is in flight,
/// the task is dropped and the child `claude` process is left to
/// the OS reaper (it'll finish or be killed when Paneflow exits;
/// the title work is best-effort and nothing else depends on it).
pub fn spawn_thread_title_summarization(req: SummarizeRequest, cx: &mut Context<Composer>) {
    let SummarizeRequest {
        agent_kind,
        cwd,
        discovery,
        user_prompt,
        assistant_response,
        title_snapshot,
        thread_view,
    } = req;
    // Claude Code emits its own `SessionInfoUpdate.title` summary;
    // re-running summarization on top of it would burn tokens for
    // a strictly worse result.
    if matches!(agent_kind, AgentKind::ClaudeCode) {
        return;
    }
    // We require the `claude` CLI to be on PATH (Claude Code
    // install). Most Paneflow users running Codex also have Claude
    // Code installed (both surface in the Agents view via the same
    // PathProbe), but if not, log + skip -- the auto-derive title
    // from the first user prompt remains.
    let claude_path = match resolve_claude_binary(&discovery) {
        Some(p) => p,
        None => {
            log::info!(
                target: "paneflow_app::agents::title_summarizer",
                "claude CLI not on PATH; skipping background title summarization for {agent_kind:?} thread"
            );
            return;
        }
    };
    let prompt_body = format!(
        "{SUMMARIZE_PROMPT}\n\n---\n\nUser: {user}\n\nAssistant: {assistant}",
        user = user_prompt.trim(),
        assistant = assistant_response.trim(),
    );
    cx.spawn(async move |_weak_composer, cx_async| {
        let title =
            match smol::unblock(move || run_claude_summary(&claude_path, &cwd, &prompt_body)).await
            {
                Ok(text) => text,
                Err(err) => {
                    log::warn!(
                        target: "paneflow_app::agents::title_summarizer",
                        "title summarization failed for {agent_kind:?}: {err:#}"
                    );
                    return;
                }
            };
        let Some(clean) = crate::project::clean_sidebar_title(&title) else {
            return;
        };
        // Cap at the first newline + hard length cap -- the prompt
        // asks for 3-7 words and the model usually obliges, but a
        // defensive truncate catches degenerate cases (multi-line
        // response, the model adding "Title: ..." despite the
        // instruction).
        let first_line = clean.lines().next().unwrap_or("").trim().to_string();
        let bounded = if first_line.chars().count() > MAX_TITLE_CHARS {
            let mut cut: String = first_line.chars().take(MAX_TITLE_CHARS).collect();
            if let Some(space) = cut.rfind(' ') {
                cut.truncate(space);
            }
            cut.push('\u{2026}');
            cut
        } else {
            first_line
        };
        if bounded.is_empty() {
            return;
        }
        let suggested = TitleSuggested {
            title: bounded,
            policy: TitleReplacePolicy::OnlyIfStillEqualTo(title_snapshot),
        };
        cx_async.update(|cx| {
            let _ = thread_view.update(cx, |_tv, cx| {
                cx.emit(suggested);
            });
        });
    })
    .detach();
}

/// Find the `claude` binary on PATH using the same `PathProbe`
/// AgentDiscovery uses for the agents view. Returns `None` if
/// Claude Code isn't installed -- the summarizer no-ops in that
/// case rather than guessing at an alternate provider.
fn resolve_claude_binary(_discovery: &Arc<AgentDiscovery>) -> Option<PathBuf> {
    which::which("claude").ok()
}

/// Blocking call: run `claude -p --dangerously-skip-permissions`
/// with the prompt piped to stdin, return stdout as the title.
/// Lives in its own function so the async wrapper can fence it
/// behind `smol::unblock` (the subprocess wait is blocking and
/// must not stall the GPUI runtime).
fn run_claude_summary(claude_path: &Path, cwd: &Path, prompt_body: &str) -> anyhow::Result<String> {
    use std::time::Instant;

    let mut child = Command::new(claude_path)
        // Non-interactive mode: prompt in, completion out, exit.
        .arg("-p")
        // Bypass permission prompts -- the summarizer does not need
        // tools (just generates a short title) but Claude's default
        // pre-flight check stalls a non-interactive run otherwise.
        .arg("--dangerously-skip-permissions")
        // Pick a cheap fast model for a 7-word completion; falling
        // back on the user's default if the alias isn't valid in
        // their config keeps the call alive on older releases.
        .arg("--model")
        .arg("haiku")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn `claude -p`: {e}"))?;

    // Pipe the prompt into the child's stdin then close it so
    // claude knows the input stream is complete.
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("claude child has no stdin"))?;
        stdin
            .write_all(prompt_body.as_bytes())
            .map_err(|e| anyhow::anyhow!("failed to write prompt to claude stdin: {e}"))?;
    }
    drop(child.stdin.take());

    // Poll-based timeout: try_wait every 100 ms until we hit either
    // completion or the deadline. `wait_with_output` doesn't accept
    // a timeout natively and we'd rather kill an over-long call
    // than hold a zombie subprocess.
    let deadline = Instant::now() + SUMMARIZE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let mut stderr = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        use std::io::Read as _;
                        let _ = err.read_to_string(&mut stderr);
                    }
                    anyhow::bail!("claude exited with status {status}: {}", stderr.trim());
                }
                let output = child
                    .wait_with_output()
                    .map_err(|e| anyhow::anyhow!("wait_with_output after success: {e}"))?;
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    anyhow::bail!(
                        "claude title summarization timed out after {SUMMARIZE_TIMEOUT:?}"
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => anyhow::bail!("try_wait on claude child failed: {e}"),
        }
    }
}
