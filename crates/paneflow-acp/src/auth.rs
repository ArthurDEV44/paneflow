//! Auth-required detection + missing-agents guidance (US-005, logic
//! layer).
//!
//! When an agent answers `session/prompt` with the ACP `AuthRequired`
//! error code (-32000), or advertises auth methods during
//! `initialize`, paneflow-acp turns that into an [`AuthRequirement`]
//! that the UI later renders as an inline card (US-013, US-016) with
//! an embedded login terminal hook (US-018 wires the PTY).
//!
//! When PATH discovery returns no agents at all, the UI shows the
//! [`MISSING_AGENTS_GUIDANCE`] copy with install commands instead of
//! the agent picker.
//!
//! No credentials are stored, persisted, or echoed anywhere in this
//! module -- it only translates ACP wire shapes into UI-ready data
//! and computes the canonical CLI command Paneflow runs on the user's
//! behalf. AC #6 ("the auth card never stores credentials") is a
//! design property of this code path: we never accept secrets as
//! input, never write to disk, never log credential material.

use crate::discovery::AgentKind;
use agent_client_protocol::schema::{AuthMethod, ErrorCode, InitializeResponse};
use agent_client_protocol::Error;

/// User-actionable instructions extracted from an agent's auth advertisement
/// or `AuthRequired` error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequirement {
    /// Agent we are talking to (drives the canonical login command).
    pub agent: AgentKind,
    /// Human-readable summary of what the user must do. If the agent
    /// supplied a description, we surface it verbatim; otherwise we
    /// fall back to a generic "Authenticate with <agent>" string so
    /// the card is never blank.
    pub description: String,
    /// Auth methods the agent advertised (e.g. ChatGPT login,
    /// `CODEX_API_KEY` env, `OPENAI_API_KEY` env for Codex). Rendered
    /// verbatim in the card so users can pick the path that matches
    /// their setup. Empty iff the agent advertised none.
    pub methods: Vec<AuthMethodSummary>,
    /// CLI command Paneflow runs in an embedded terminal when the
    /// user clicks "Run login". `None` if there is no canonical login
    /// command for this agent kind.
    pub login_command: Option<String>,
}

/// One auth method, flattened from ACP's tagged enum into a flat shape
/// the UI can iterate uniformly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMethodSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

impl From<&AuthMethod> for AuthMethodSummary {
    fn from(method: &AuthMethod) -> Self {
        Self {
            id: method.id().0.to_string(),
            name: method.name().to_string(),
            description: method.description().map(|s| s.to_string()),
        }
    }
}

impl AuthRequirement {
    /// Build an [`AuthRequirement`] from an `InitializeResponse`. Used
    /// when the agent advertises auth methods upfront (Codex pattern).
    /// Returns `None` if `auth_methods` is empty -- no advertisement
    /// means no card is needed.
    pub fn from_initialize(response: &InitializeResponse, agent: AgentKind) -> Option<Self> {
        if response.auth_methods.is_empty() {
            return None;
        }
        let methods: Vec<AuthMethodSummary> =
            response.auth_methods.iter().map(Into::into).collect();
        Some(Self {
            description: pick_description(&methods, agent),
            methods,
            login_command: canonical_login_command(agent),
            agent,
        })
    }

    /// Build an [`AuthRequirement`] from an ACP error. Returns `None`
    /// if the error is not an `AuthRequired` error so callers can
    /// match `Some(req) => show card, None => surface other error`.
    ///
    /// Only the error's `message` is used as the description -- the
    /// structured method list comes from the agent's earlier
    /// `InitializeResponse` (the UI caches it from
    /// [`Self::from_initialize`] and merges via
    /// [`Self::with_methods`] when an auth error arrives).
    pub fn from_error(error: &Error, agent: AgentKind) -> Option<Self> {
        if !is_auth_required(error) {
            return None;
        }
        let description = if error.message.is_empty() {
            generic_description(agent)
        } else {
            error.message.clone()
        };
        Some(Self {
            description,
            methods: Vec::new(),
            login_command: canonical_login_command(agent),
            agent,
        })
    }

    /// Replace the method list. Used by the UI to merge methods
    /// captured during `initialize` into an [`AuthRequirement`] that
    /// was built from an `AuthRequired` error response.
    #[must_use]
    pub fn with_methods(mut self, methods: Vec<AuthMethodSummary>) -> Self {
        if self.methods.is_empty() && self.description == generic_description(self.agent) {
            if let Some(desc) = methods.iter().find_map(|m| m.description.clone()) {
                // Upgrade a generic description to the agent's own
                // first advertised description so the card reads
                // better.
                self.description = desc;
            }
        }
        self.methods = methods;
        self
    }
}

/// True iff `error` is the ACP `AuthRequired` error (-32000).
pub fn is_auth_required(error: &Error) -> bool {
    matches!(error.code, ErrorCode::AuthRequired)
}

/// Best-effort filesystem check for "is the user already signed in
/// to this agent?". Strictly a presence probe — the file existing
/// does NOT prove the credentials are still valid (token expiry,
/// revocation, scope mismatch). The real authoritative check happens
/// at session start, where the ACP runtime surfaces an `AuthRequired`
/// error if the token is rejected; this function only drives the UI
/// chip on the welcome screen so the user does not see a "Sign in"
/// button when they have obviously already done so.
///
/// Lookups:
/// - `ClaudeCode`: `~/.claude/.credentials.json` exists.
/// - `Codex`: `~/.codex/auth.json` exists, OR `CODEX_API_KEY` /
///   `OPENAI_API_KEY` env var is set (Codex accepts either auth path).
pub fn is_signed_in(agent: AgentKind) -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    match agent {
        AgentKind::ClaudeCode => home.join(".claude/.credentials.json").exists(),
        AgentKind::Codex => {
            if home.join(".codex/auth.json").exists() {
                return true;
            }
            std::env::var_os("CODEX_API_KEY")
                .filter(|v| !v.is_empty())
                .is_some()
                || std::env::var_os("OPENAI_API_KEY")
                    .filter(|v| !v.is_empty())
                    .is_some()
        }
    }
}

/// Canonical login command for an agent kind, or `None` if no
/// terminal-based login flow is known. Used by the UI's "Run login"
/// button to spawn an embedded `TerminalView` running this command.
pub fn canonical_login_command(agent: AgentKind) -> Option<String> {
    match agent {
        AgentKind::ClaudeCode => Some("claude /login".to_string()),
        // Codex's login flow varies (ChatGPT browser flow, API key
        // export, etc.). The card surfaces all advertised methods
        // verbatim per AC #5; no single canonical command exists.
        AgentKind::Codex => None,
    }
}

fn pick_description(methods: &[AuthMethodSummary], agent: AgentKind) -> String {
    methods
        .iter()
        .find_map(|m| m.description.clone())
        .unwrap_or_else(|| generic_description(agent))
}

fn generic_description(agent: AgentKind) -> String {
    format!("Authenticate with {}", agent.display_name())
}

/// Copy shown when discovery returns zero agents. AC #4 mandates at
/// least the Claude Code install command be present. The strings are
/// rendered verbatim in the missing-agents empty state card.
pub struct MissingAgentsGuidance {
    pub title: &'static str,
    pub message: &'static str,
    /// Shell commands the user can copy-paste to install Claude Code.
    /// Listed in order of recommendation -- bun first (matches the
    /// repo's mandatory package manager per CLAUDE.md), npm fallback.
    pub install_commands: &'static [&'static str],
    pub more_url: &'static str,
}

/// Single source of truth for the missing-agents empty state copy
/// (US-005 AC #4).
pub const MISSING_AGENTS_GUIDANCE: MissingAgentsGuidance = MissingAgentsGuidance {
    title: "No AI agents detected",
    message: "Paneflow looks for Claude Code or Codex on your PATH. Install one to get started.",
    install_commands: &[
        "bun install -g @anthropic-ai/claude-code @zed-industries/claude-code-acp",
        "npm install -g @anthropic-ai/claude-code @zed-industries/claude-code-acp",
    ],
    more_url: "https://claude.ai/code",
};

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        AuthMethod, AuthMethodAgent, AuthMethodId, InitializeResponse, ProtocolVersion,
    };

    fn agent_method(id: &str, name: &str, desc: Option<&str>) -> AuthMethod {
        let mut method = AuthMethodAgent::new(AuthMethodId::from(id.to_string()), name);
        if let Some(d) = desc {
            method.description = Some(d.to_string());
        }
        AuthMethod::Agent(method)
    }

    fn auth_required_error(message: &str) -> Error {
        let code_i32: i32 = ErrorCode::AuthRequired.into();
        Error::new(code_i32, message)
    }

    #[test]
    fn from_initialize_with_methods_picks_first_description() {
        let response = InitializeResponse::new(ProtocolVersion::V1).auth_methods(vec![
            agent_method(
                "chatgpt",
                "Sign in with ChatGPT",
                Some("Open browser for ChatGPT login"),
            ),
            agent_method(
                "codex_api_key",
                "CODEX_API_KEY env var",
                Some("Set CODEX_API_KEY before launching"),
            ),
        ]);
        let req = AuthRequirement::from_initialize(&response, AgentKind::Codex)
            .expect("Codex should produce an AuthRequirement");
        assert_eq!(req.agent, AgentKind::Codex);
        assert_eq!(req.methods.len(), 2);
        assert_eq!(req.methods[0].id, "chatgpt");
        assert_eq!(req.methods[0].name, "Sign in with ChatGPT");
        assert_eq!(req.description, "Open browser for ChatGPT login");
        assert_eq!(req.login_command, None, "Codex has no canonical CLI login");
    }

    #[test]
    fn from_initialize_with_no_methods_returns_none() {
        let response = InitializeResponse::new(ProtocolVersion::V1);
        let result = AuthRequirement::from_initialize(&response, AgentKind::ClaudeCode);
        assert!(result.is_none(), "no advertised methods means no card");
    }

    #[test]
    fn from_initialize_falls_back_to_generic_description() {
        let response = InitializeResponse::new(ProtocolVersion::V1)
            .auth_methods(vec![agent_method("plain", "Plain auth", None)]);
        let req = AuthRequirement::from_initialize(&response, AgentKind::ClaudeCode).expect("some");
        assert!(req.description.contains("Claude Code"));
        assert_eq!(req.login_command.as_deref(), Some("claude /login"));
    }

    #[test]
    fn from_error_returns_none_for_non_auth_errors() {
        let code_i32: i32 = ErrorCode::InvalidParams.into();
        let err = Error::new(code_i32, "bad params");
        assert!(AuthRequirement::from_error(&err, AgentKind::ClaudeCode).is_none());
    }

    #[test]
    fn from_error_with_bare_message_synthesizes_card() {
        let err = auth_required_error("Run `claude /login` in the terminal");
        let req = AuthRequirement::from_error(&err, AgentKind::ClaudeCode)
            .expect("AuthRequired must yield a requirement");
        assert_eq!(req.agent, AgentKind::ClaudeCode);
        assert_eq!(req.description, "Run `claude /login` in the terminal");
        assert!(
            req.methods.is_empty(),
            "no data payload -> no methods to surface",
        );
        assert_eq!(req.login_command.as_deref(), Some("claude /login"));
    }

    #[test]
    fn with_methods_merges_initialize_data_into_error_requirement() {
        // Typical UI flow: capture auth methods during initialize, then
        // on AuthRequired error merge them onto the error-built
        // requirement so the card shows both the error message and the
        // full method list.
        let err = auth_required_error("Codex needs authentication");
        let methods = vec![
            AuthMethodSummary {
                id: "chatgpt".to_string(),
                name: "Sign in with ChatGPT".to_string(),
                description: Some("Browser flow".to_string()),
            },
            AuthMethodSummary {
                id: "codex_api_key".to_string(),
                name: "CODEX_API_KEY env var".to_string(),
                description: None,
            },
        ];
        let req = AuthRequirement::from_error(&err, AgentKind::Codex)
            .expect("auth")
            .with_methods(methods.clone());
        assert_eq!(req.methods, methods);
        // The error message wins over the methods' description when
        // the error has a non-empty message.
        assert_eq!(req.description, "Codex needs authentication");
    }

    #[test]
    fn with_methods_upgrades_generic_description() {
        // If the error carries no message, from_error falls back to a
        // generic description. with_methods should upgrade it to the
        // first method's description for a better-reading card.
        let code_i32: i32 = ErrorCode::AuthRequired.into();
        let err = Error::new(code_i32, "");
        let methods = vec![AuthMethodSummary {
            id: "chatgpt".to_string(),
            name: "ChatGPT login".to_string(),
            description: Some("Open browser for ChatGPT login".to_string()),
        }];
        let req = AuthRequirement::from_error(&err, AgentKind::Codex)
            .expect("auth")
            .with_methods(methods);
        assert_eq!(req.description, "Open browser for ChatGPT login");
    }

    #[test]
    fn missing_agents_guidance_lists_install_commands_for_claude() {
        let guidance = &MISSING_AGENTS_GUIDANCE;
        assert!(!guidance.title.is_empty());
        assert!(!guidance.install_commands.is_empty());
        let any_mentions_claude_code = guidance
            .install_commands
            .iter()
            .any(|c| c.contains("@anthropic-ai/claude-code"));
        assert!(
            any_mentions_claude_code,
            "AC #4: must include the Claude Code install command",
        );
        let any_mentions_acp_wrapper = guidance
            .install_commands
            .iter()
            .any(|c| c.contains("@zed-industries/claude-code-acp"));
        assert!(
            any_mentions_acp_wrapper,
            "ACP wrapper install command must be present",
        );
        let bun_first = guidance.install_commands[0].starts_with("bun ");
        assert!(bun_first, "bun command must be the first recommendation");
    }

    #[test]
    fn canonical_login_command_per_agent_kind() {
        assert_eq!(
            canonical_login_command(AgentKind::ClaudeCode).as_deref(),
            Some("claude /login"),
        );
        assert_eq!(canonical_login_command(AgentKind::Codex), None);
    }
}
