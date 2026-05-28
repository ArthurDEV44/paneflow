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
/// US-011 (cli-hardening-followup-2026-Q3): this helper MUST be
/// called from the very first lines of `main()`, before any
/// `std::thread::spawn`, `tokio::runtime::Builder::build`, or smol
/// executor initialization. Rust 1.85 made `std::env::remove_var`
/// `unsafe` because it races with concurrent `getenv` from any
/// other thread; the runtime sub-systems above all read env on
/// startup, so calling this from inside `spawn_acp_agent` (the
/// pre-US-011 site) was a real race window. Calling it before any
/// thread exists is genuinely safe by construction. Subsequent
/// calls from `spawn_acp_agent` are now no-op idempotent guards.
///
/// SAFETY (residual): even with the early-main call, `set_var` from
/// the `spawn` test below briefly puts the var back -- the test
/// must remain single-threaded (it is; standard `#[test]` harness).
pub fn scrub_claudecode_env() {
    // SAFETY: called from main() before any thread::spawn or async
    // runtime init (US-011) -- no concurrent getenv possible.
    unsafe {
        std::env::remove_var(CLAUDECODE_ENV);
    }
}

/// Wire-level debug callback installed by [`spawn_acp_agent`]. Emits one
/// `tracing` event per line, tagged with the direction (stdin / stdout /
/// stderr) so a subscriber can filter or route per stream.
///
/// US-013 (cli-hardening-followup-2026-Q3): the wire payload is
/// passed through [`redact_secrets`] before emission. Without this,
/// running with `RUST_LOG=paneflow_acp::wire=trace` (the
/// documented debugging workflow) writes every ACP init frame and
/// per-turn token to `paneflow-debug.log`. A wrapper that injects
/// `ANTHROPIC_API_KEY=sk-ant-...` as a `NAME=value` prefix token
/// would leak the key to disk. Redaction is applied before
/// `tracing::trace!` so even users who pipe TRACE through
/// `tee debug.log` cannot accidentally ship the unredacted form.
fn trace_wire_line(line: &str, direction: LineDirection) {
    let direction = match direction {
        LineDirection::Stdin => "stdin",
        LineDirection::Stdout => "stdout",
        LineDirection::Stderr => "stderr",
    };
    let redacted = redact_secrets(line);
    tracing::trace!(target: "paneflow_acp::wire", %direction, "{redacted}");
}

/// US-013 (cli-hardening-followup-2026-Q3): mask substrings that
/// look like API keys before emitting wire-level TRACE.
///
/// Two patterns are handled with a hand-rolled byte scanner (avoids
/// adding `regex` + `once_cell` for a single call site). Both
/// anchors are ASCII so byte-offset arithmetic over a `&str` is
/// safe -- we only treat non-anchor bytes as opaque copy targets
/// and never split inside a multi-byte UTF-8 sequence.
///
/// 1. **`sk-` prefix + >= 32 alphanumeric (or `-` / `_`) chars**:
///    Anthropic (`sk-ant-api03-...`), OpenAI (`sk-...`), and
///    similar provider shapes.
/// 2. **`<...>_API_KEY=<value>`**: env-style assignments. The key
///    name is preserved (`ANTHROPIC_API_KEY=`) so debug context
///    stays actionable; only the value (up to the next whitespace,
///    quote, comma, or `}`) is replaced with `[REDACTED]`.
///
/// Returns the original line unchanged when no anchor is present,
/// so the common case allocates nothing.
pub(crate) fn redact_secrets(line: &str) -> std::borrow::Cow<'_, str> {
    if !line.contains("sk-") && !line.contains("_API_KEY=") {
        return std::borrow::Cow::Borrowed(line);
    }
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        // Pattern 2: `_API_KEY=<value>`. The key name preceding
        // `_API_KEY` is already in `out` because we copy
        // byte-by-byte until we hit the anchor.
        if bytes[i..].starts_with(b"_API_KEY=") {
            out.push_str("_API_KEY=");
            i += 9;
            // Skip the value up to the first terminator.
            while i < bytes.len() {
                let b = bytes[i];
                if b == b' '
                    || b == b'\t'
                    || b == b'\n'
                    || b == b'\r'
                    || b == b'"'
                    || b == b'\''
                    || b == b','
                    || b == b'}'
                {
                    break;
                }
                i += 1;
            }
            out.push_str("[REDACTED]");
            continue;
        }
        // Pattern 1: `sk-` + >= 32 alphanumeric / `-` / `_`.
        if bytes[i..].starts_with(b"sk-") {
            let start = i + 3;
            let mut j = start;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j - start >= 32 {
                out.push_str("[REDACTED]");
                i = j;
                continue;
            }
        }
        // Copy the next UTF-8 character intact. The leading-byte
        // mask tells us the sequence length; non-anchor bytes are
        // never split inside a multi-byte sequence because the
        // anchors above are ASCII (single-byte) and only match at
        // ASCII boundaries.
        let ch_len = utf8_char_len(bytes[i]);
        // SAFETY: `line` is &str (valid UTF-8) and `i..i+ch_len`
        // is a valid char boundary derived from `utf8_char_len`.
        out.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    std::borrow::Cow::Owned(out)
}

/// UTF-8 character length from the leading byte. Returns 1 for an
/// invalid / continuation byte to guarantee forward progress.
fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
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

    /// US-013 (cli-hardening-followup-2026-Q3): the wire-trace
    /// redactor must mask Anthropic / OpenAI shape keys and
    /// `*_API_KEY=` env assignments before they reach
    /// `paneflow-debug.log` via `RUST_LOG=paneflow_acp::wire=trace`.
    #[test]
    fn redact_secrets_masks_api_key_shapes() {
        // Anthropic shape (sk-ant-...)
        let line = r#"{"init":"sk-ant-api03-1234567890abcdef1234567890abcdef-xyz"}"#;
        let out = super::redact_secrets(line);
        assert!(
            !out.contains("api03-1234"),
            "anthropic key not masked: {out}"
        );
        assert!(
            out.contains("[REDACTED]"),
            "missing redaction marker: {out}"
        );

        // Plain OpenAI shape (sk-XXXXX...)
        let line = "Authorization: Bearer sk-1234567890abcdef1234567890abcdefAB end";
        let out = super::redact_secrets(line);
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("1234567890abcdef"));
        assert!(out.contains("end"), "tail must survive: {out}");

        // _API_KEY= env-shape: key name preserved, value masked
        let line = r#"ANTHROPIC_API_KEY=sk-ant-secretvalue123456 STAGE=prod"#;
        let out = super::redact_secrets(line);
        assert!(
            out.contains("ANTHROPIC_API_KEY=[REDACTED]"),
            "expected key-preserved redaction, got: {out}"
        );
        assert!(out.contains("STAGE=prod"), "tail must survive: {out}");

        // No anchor: line returned unchanged via Cow::Borrowed.
        let line = r#"{"session/new":{"model":"sonnet"}}"#;
        let out = super::redact_secrets(line);
        assert_eq!(out, line);
        assert!(
            matches!(out, std::borrow::Cow::Borrowed(_)),
            "no-secret path must not allocate"
        );

        // sk- with too-short suffix: not a key, left alone.
        let line = "sk-short";
        let out = super::redact_secrets(line);
        assert_eq!(out, line);

        // Multi-byte UTF-8 around a redaction must not split chars.
        let line = "préfixe sk-1234567890abcdef1234567890abcdefAB suffïxe";
        let out = super::redact_secrets(line);
        assert!(out.contains("préfixe"));
        assert!(out.contains("suffïxe"));
        assert!(out.contains("[REDACTED]"));
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
