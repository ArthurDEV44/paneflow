//! `spawn_acp_agent`: Paneflow's single entry point for launching an
//! ACP-compatible CLI agent. See US-001 of `tasks/prd-agents-view.md`.

use agent_client_protocol::{AcpAgent, LineDirection};
use std::str::FromStr;

/// Environment variable set by a running Claude Code process. If inherited
/// by a child `claude-code-acp` wrapper, the wrapper refuses to launch
/// ("Claude Code cannot launch inside another Claude Code session"). The
/// ACP SDK exposes no env-unset hook, so we scrub it from the parent
/// process instead. See [`spawn_acp_agent`] for the safety argument.
const CLAUDECODE_ENV: &str = "CLAUDECODE";

/// Spawn an ACP-compatible CLI agent through Paneflow's centralized helper.
///
/// `cmd` is a space-separated command string (e.g. the canonical
/// `"bunx -y @zed-industries/claude-code-acp@latest"`). Leading `NAME=value`
/// tokens are interpreted as env vars to *set* on the child by the ACP SDK
/// (additive only). To unset `CLAUDECODE`, this helper mutates the parent
/// process env instead (see the safety note on [`scrub_claudecode_env`]).
///
/// The returned [`AcpAgent`] is pre-wired with a `with_debug` callback that
/// streams every wire-level line through the `tracing` crate at `TRACE`
/// level under the target `paneflow_acp::wire`. Filter via
/// `RUST_LOG=paneflow_acp::wire=trace`.
///
/// # Errors
/// Returns an error naming the missing binary if the first non-env token of
/// `cmd` cannot be resolved by [`which::which`]. The binary check runs
/// before any subprocess spawn so the caller never sees a generic
/// `No such file or directory` deep inside the ACP SDK.
pub async fn spawn_acp_agent(cmd: &str) -> anyhow::Result<AcpAgent> {
    let binary = first_binary_token(cmd).ok_or_else(|| {
        anyhow::anyhow!("spawn_acp_agent: empty command (expected at least one token)")
    })?;
    which::which(binary).map_err(|err| {
        anyhow::anyhow!("spawn_acp_agent: binary `{binary}` not found on PATH ({err})")
    })?;

    scrub_claudecode_env();

    let agent = AcpAgent::from_str(cmd)
        .map_err(|err| anyhow::anyhow!("spawn_acp_agent: failed to parse `{cmd}`: {err}"))?
        .with_debug(trace_wire_line);

    tracing::debug!(target: "paneflow_acp", %binary, "spawned ACP agent");
    Ok(agent)
}

/// Remove `CLAUDECODE` from the current process environment so future
/// subprocesses (which inherit it by default) do not see it.
///
/// SAFETY: `std::env::remove_var` is `unsafe` on Rust >= 1.85 because it
/// races with concurrent env reads/writes from other threads. The mutation
/// is safe in Paneflow because: (1) `CLAUDECODE` is never read by any
/// Paneflow code path -- only by the to-be-spawned agent wrapper; (2) all
/// agent spawns funnel through this helper, so the order is always
/// "mutate parent env, then spawn child"; (3) the operation is idempotent
/// -- subsequent calls are no-ops once the var is gone.
pub(crate) fn scrub_claudecode_env() {
    // SAFETY: see function-level doc comment.
    unsafe {
        std::env::remove_var(CLAUDECODE_ENV);
    }
}

/// Wire-level debug callback installed by [`spawn_acp_agent`]. Emits one
/// `tracing` event per line, tagged with the direction (stdin / stdout /
/// stderr) so a subscriber can filter or route per stream.
fn trace_wire_line(line: &str, direction: LineDirection) {
    let direction = match direction {
        LineDirection::Stdin => "stdin",
        LineDirection::Stdout => "stdout",
        LineDirection::Stderr => "stderr",
    };
    tracing::trace!(target: "paneflow_acp::wire", %direction, "{line}");
}

/// Return the first token of `cmd` that does not look like a `NAME=value`
/// env assignment. The ACP SDK's `AcpAgent::from_str` parses leading
/// `NAME=value` tokens as env vars to set, then treats the next token as
/// the binary; this helper mirrors that lookup so we can validate the
/// binary on PATH before delegating to the SDK.
pub(crate) fn first_binary_token(cmd: &str) -> Option<&str> {
    cmd.split_whitespace().find(|tok| !tok.contains('='))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_token_skips_leading_env_assignments() {
        assert_eq!(
            first_binary_token("bunx -y @scope/pkg@latest"),
            Some("bunx")
        );
        assert_eq!(first_binary_token("FOO=bar bunx -y pkg"), Some("bunx"));
        assert_eq!(
            first_binary_token("FOO=bar BAZ=qux node script.js"),
            Some("node"),
        );
    }

    #[test]
    fn binary_token_empty_and_whitespace() {
        assert_eq!(first_binary_token(""), None);
        assert_eq!(first_binary_token("   \t  "), None);
    }

    #[tokio::test]
    async fn spawn_rejects_missing_binary_naming_it() {
        let result = spawn_acp_agent("paneflow-acp-missing-xyz-binary --foo").await;
        let err = result.expect_err("missing binary must error").to_string();
        assert!(
            err.contains("paneflow-acp-missing-xyz-binary"),
            "error must name the missing binary, got: {err}",
        );
        assert!(
            err.contains("PATH"),
            "error should mention PATH, got: {err}"
        );
    }

    #[tokio::test]
    async fn spawn_rejects_empty_command() {
        let result = spawn_acp_agent("   ").await;
        let err = result.expect_err("empty command must error").to_string();
        assert!(err.contains("empty command"), "got: {err}");
    }

    #[test]
    fn scrub_claudecode_is_idempotent() {
        // SAFETY: test-only -- single-threaded test runner step. Sets, scrubs,
        // and re-scrubs to confirm the second call does not panic.
        unsafe {
            std::env::set_var(CLAUDECODE_ENV, "1");
        }
        scrub_claudecode_env();
        assert!(std::env::var(CLAUDECODE_ENV).is_err());
        scrub_claudecode_env();
        assert!(std::env::var(CLAUDECODE_ENV).is_err());
    }
}
