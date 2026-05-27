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

use smol::lock::Semaphore;

/// US-018: cap concurrent title summarizers at two in flight. Each
/// in-flight summarizer holds one of smol's blocking-pool threads
/// for up to [`SUMMARIZE_TIMEOUT`]; uncapped, a rapid burst of
/// [`crate::agents::runtime::RuntimeEvent::TurnEnded`] events would
/// saturate the pool and starve unrelated `smol::unblock` work
/// (session readers, log writers). Const-initialized so it works
/// inside a `static`.
static SUMMARIZER_GATE: Semaphore = Semaphore::new(2);

use gpui::{Context, Task, WeakEntity};
use paneflow_acp::{AgentDiscovery, AgentKind};

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

/// Inputs for [`summarize_thread_title_task`]. Grouped so the
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

/// Build a background task that summarizes the conversation and
/// emits the resulting title through `thread_view` once available.
///
/// Returns the [`Task<()>`] so [`ThreadView`] can park it in
/// `pending_title_generation` and cancel a previous in-flight task
/// before spawning a new one. Cancellation is what Zed's
/// `pending_title_generation` guard at `crates/agent/src/thread.rs:962`
/// gives for free — dropping the `Task` aborts the polling so two
/// concurrent `claude -p` subprocesses can't pile up on a fast retry.
///
/// Returns `None` for agents that already push titles via ACP
/// `SessionInfoUpdate.title` (today: Claude Code) or when the
/// `claude` CLI is missing — caller should treat the missing task as
/// "nothing to do, leave the auto-derived title in place".
pub fn summarize_thread_title_task(
    req: SummarizeRequest,
    cx: &mut Context<ThreadView>,
) -> Option<Task<()>> {
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
        return None;
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
            return None;
        }
    };
    let prompt_body = format!(
        "{SUMMARIZE_PROMPT}\n\n---\n\nUser: {user}\n\nAssistant: {assistant}",
        user = user_prompt.trim(),
        assistant = assistant_response.trim(),
    );
    let task = cx.spawn(async move |_weak_view, cx_async| {
        // US-018 (audit P2-6): bound concurrent summarizers to two
        // in flight. Each summarizer reserves a smol::unblock
        // blocking thread for up to SUMMARIZE_TIMEOUT (60s). With no
        // gate, three TurnEnded events arriving back-to-back during
        // a rapid multi-agent session would saturate smol's blocking
        // pool and stall unrelated `smol::unblock` calls (file I/O,
        // session readers). The permit is held across the unblock
        // future so the cap is real, not just on spawn admission.
        let _permit = SUMMARIZER_GATE.acquire().await;
        let title =
            match smol::unblock(move || run_claude_summary(&claude_path, &cwd, &prompt_body)).await
            {
                Ok(text) => text,
                Err(err) => {
                    log::warn!(
                        target: "paneflow_app::agents::title_summarizer",
                        "title summarization failed for {agent_kind:?}: {err:#}"
                    );
                    // Surface the failure on the ThreadView so the UI / future
                    // retry logic can react instead of silently swallowing.
                    cx_async.update(|cx| {
                        let _ = thread_view.update(cx, |tv, cx| {
                            tv.note_title_generation_failed(cx);
                        });
                    });
                    return;
                }
            };
        let Some(clean) = crate::project::clean_sidebar_title(&title) else {
            cx_async.update(|cx| {
                let _ = thread_view.update(cx, |tv, cx| {
                    tv.note_title_generation_failed(cx);
                });
            });
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
            cx_async.update(|cx| {
                let _ = thread_view.update(cx, |tv, cx| {
                    tv.note_title_generation_failed(cx);
                });
            });
            return;
        }
        let suggested = TitleSuggested {
            title: bounded,
            policy: TitleReplacePolicy::OnlyIfStillEqualTo(title_snapshot),
        };
        cx_async.update(|cx| {
            let _ = thread_view.update(cx, |tv, cx| {
                tv.note_title_generation_succeeded();
                cx.emit(suggested);
            });
        });
    });
    Some(task)
}

/// Find the `claude` binary on PATH using the same `AgentDiscovery`
/// the agents view uses. Falls back to a fresh `which::which`
/// lookup if discovery hasn't populated yet — Paneflow's GUI launch
/// (especially on macOS) augments PATH at startup, so going through
/// the discovery cache picks up that PATH even when `which::which`
/// from a bare `Command` wouldn't. Returns `None` if Claude Code
/// isn't installed -- the summarizer no-ops in that case rather
/// than guessing at an alternate provider.
fn resolve_claude_binary(discovery: &Arc<AgentDiscovery>) -> Option<PathBuf> {
    if let Some(agent) = discovery
        .list()
        .into_iter()
        .find(|a| a.kind == AgentKind::ClaudeCode)
    {
        return Some(agent.binary_path);
    }
    which::which("claude").ok()
}

/// Build a [`Command`] that can spawn `claude` -- handling the Windows
/// case where `which::which("claude")` resolves to a `claude.cmd` /
/// `claude.bat` shim (npm / scoop install pattern). `Command::new` on
/// `.cmd` / `.bat` returns `ERROR_BAD_EXE_FORMAT` because Windows does
/// not treat them as native executables; they must be invoked through
/// `cmd.exe /C`. Linux + macOS pass through unchanged (US-014).
fn build_claude_command(claude_path: &Path) -> Command {
    #[cfg(target_os = "windows")]
    {
        let is_shim = claude_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
        if is_shim {
            let mut cmd = Command::new("cmd.exe");
            cmd.arg("/C").arg(claude_path);
            return cmd;
        }
    }
    Command::new(claude_path)
}

/// Blocking call: run `claude -p --dangerously-skip-permissions`
/// with the prompt piped to stdin, return stdout as the title.
/// Lives in its own function so the async wrapper can fence it
/// behind `smol::unblock` (the subprocess wait is blocking and
/// must not stall the GPUI runtime).
fn run_claude_summary(claude_path: &Path, cwd: &Path, prompt_body: &str) -> anyhow::Result<String> {
    use std::time::Instant;

    let mut child = build_claude_command(claude_path)
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
    //
    // Once `try_wait()` returns `Ok(Some(status))`, the child has been
    // reaped — calling `child.wait_with_output()` after that performs
    // a second `waitpid`, which on Linux returns `ECHILD` and silently
    // produces empty stdout. Read stdout/stderr directly from the
    // already-collected child handles instead.
    let deadline = Instant::now() + SUMMARIZE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                use std::io::Read as _;
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if !status.success() {
                    let mut stderr = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    anyhow::bail!("claude exited with status {status}: {}", stderr.trim());
                }
                return Ok(stdout);
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
