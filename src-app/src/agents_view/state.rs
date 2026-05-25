//! Minimal state machine for the US-005 Agents-view shell.
//!
//! Lives outside [`view::AgentsView`] so unit tests can drive the
//! transitions without spinning up GPUI. The full FSM (Thinking,
//! Streaming, WaitingForInput etc.) lands with US-013 + US-015.

use paneflow_acp::{AgentKind, AuthRequirement, DiscoveredAgent};

/// Variants of an agent-load failure surface (US-026, port of Zed's
/// `LoadError`). Paneflow does not have a runtime ACP load-error
/// emitter yet -- this enum exists today as the data shape the
/// rendering path consumes once the wiring lands. The
/// [`super::view::AgentsView::trigger_load_error`] helper seeds the
/// variants from tests and dev tooling so the visual port can be
/// exercised against the running app.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum AgentLoadError {
    /// Install command exited non-zero -- captured stderr.
    FailedToInstall(String),
    /// Server process exited (after a successful spawn) -- exit code
    /// plus optional captured stderr.
    Exited { status: i32, stderr: Option<String> },
    /// Reported version is older than the minimum Paneflow supports.
    Unsupported {
        command: String,
        current_version: String,
        minimum_version: String,
    },
    /// Catch-all (failed to launch / IO error / handshake mismatch).
    Other(String),
}

impl AgentLoadError {
    /// Title shown at the top of the [`crate::widgets::callout::Callout`].
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::FailedToInstall(_) => "Failed to Install",
            Self::Exited { .. } | Self::Other(_) => "Failed to Launch",
            Self::Unsupported { .. } => "Unsupported version",
        }
    }

    /// Body text. Multi-line for `Exited` (status + captured stderr).
    pub(crate) fn body(&self) -> String {
        match self {
            Self::FailedToInstall(msg) | Self::Other(msg) => msg.clone(),
            Self::Exited { status, stderr } => {
                let mut s = format!("Server exited with status {status}");
                if let Some(stderr) = stderr {
                    s.push('\n');
                    s.push_str(stderr);
                }
                s
            }
            Self::Unsupported {
                command,
                current_version,
                minimum_version,
            } => {
                if current_version.is_empty() {
                    format!("Currently using {command}, which does not report a valid --version")
                } else {
                    format!(
                        "Currently using {command}, which is only version {current_version} \
                         (need at least {minimum_version})"
                    )
                }
            }
        }
    }
}

/// What the Agents view is currently displaying.
#[derive(Clone, Debug)]
pub(crate) enum AgentsState {
    /// First open before discovery has run.
    Loading,
    /// Discovery returned at least one agent. We surface the list with
    /// a "Try connect" button per agent (US-008/US-013 will swap this
    /// for the full project + thread sidebar).
    AgentsListed(Vec<DiscoveredAgent>),
    /// Discovery returned no agents -- render the
    /// [`paneflow_acp::MISSING_AGENTS_GUIDANCE`] empty state (AC #4).
    NoAgentsDetected,
    /// One agent reported `AuthRequired` (or advertised auth methods
    /// at init time). Render the auth card with a "Run login" button
    /// (AC #1, AC #5).
    AuthRequired {
        requirement: AuthRequirement,
        agent: AgentKind,
    },
    /// As above, but the user has clicked a specific auth method and
    /// we are waiting on the agent to finish authenticating. Renders
    /// a rotating spinner inside the Callout (US-025 AC #2).
    AuthPending {
        requirement: AuthRequirement,
        agent: AgentKind,
    },
    /// User clicked "Run login"; the embedded TerminalView for the
    /// login command is live. We do not move out of `AuthRequired`
    /// state -- the card stays so the user has context for what they
    /// are doing; the embedded terminal renders as a child of the
    /// card. This is a separate variant so the FSM can capture
    /// "login terminal is on-screen, do not re-arm the agent yet".
    LoginInProgress {
        requirement: AuthRequirement,
        agent: AgentKind,
    },
    /// Agent backend failed to load / install / launch. Rendered as
    /// an Error-severity [`crate::widgets::callout::Callout`] with a
    /// Copy button on long bodies (US-026 AC #1, AC #2).
    LoadFailed { error: AgentLoadError },
}

impl AgentsState {
    /// True iff the state corresponds to "no agents at all". Used by
    /// tests; the runtime view matches the variant directly.
    #[allow(dead_code)]
    pub(crate) fn is_missing(&self) -> bool {
        matches!(self, Self::NoAgentsDetected)
    }

    /// True iff the user is currently being prompted to authenticate.
    /// Used by tests; the runtime view matches the variant directly.
    #[allow(dead_code)]
    pub(crate) fn is_authenticating(&self) -> bool {
        matches!(
            self,
            Self::AuthRequired { .. } | Self::AuthPending { .. } | Self::LoginInProgress { .. }
        )
    }

    /// True iff the agent backend reported a load error. US-026 test
    /// helper.
    #[allow(dead_code)]
    pub(crate) fn is_load_failed(&self) -> bool {
        matches!(self, Self::LoadFailed { .. })
    }

    /// Transition `AuthRequired` -> `LoginInProgress` after the user
    /// clicks "Run login". No-op for other states.
    pub(crate) fn start_login(self) -> Self {
        match self {
            Self::AuthRequired { requirement, agent }
            | Self::AuthPending { requirement, agent } => {
                Self::LoginInProgress { requirement, agent }
            }
            other => other,
        }
    }

    /// Transition `LoginInProgress` -> `AuthRequired` when the user
    /// cancels the embedded login terminal. Used by the close button
    /// on the embedded terminal card.
    pub(crate) fn cancel_login(self) -> Self {
        match self {
            Self::LoginInProgress { requirement, agent } => {
                Self::AuthRequired { requirement, agent }
            }
            other => other,
        }
    }

    /// Transition `AuthRequired` -> `AuthPending` once a specific
    /// auth method has been selected and the request is in flight.
    /// Mirrors Zed's `pending_auth_method` discriminator.
    #[allow(dead_code)]
    pub(crate) fn start_auth_pending(self) -> Self {
        match self {
            Self::AuthRequired { requirement, agent } => Self::AuthPending { requirement, agent },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paneflow_acp::AuthRequirement;
    use std::path::PathBuf;

    fn dummy_requirement(agent: AgentKind) -> AuthRequirement {
        AuthRequirement {
            agent,
            description: "Authenticate".to_string(),
            methods: Vec::new(),
            login_command: Some("claude /login".to_string()),
        }
    }

    fn dummy_agent(agent: AgentKind) -> DiscoveredAgent {
        DiscoveredAgent {
            kind: agent,
            spawn_command: "bunx -y @scope/pkg@latest".to_string(),
            binary_path: PathBuf::from("/usr/bin/claude"),
            runner_path: PathBuf::from("/usr/bin/bunx"),
        }
    }

    #[test]
    fn missing_predicate_only_true_for_no_agents() {
        assert!(AgentsState::NoAgentsDetected.is_missing());
        assert!(!AgentsState::Loading.is_missing());
        assert!(!AgentsState::AgentsListed(vec![dummy_agent(AgentKind::ClaudeCode)]).is_missing());
    }

    #[test]
    fn authenticating_predicate_covers_both_auth_states() {
        let req = dummy_requirement(AgentKind::ClaudeCode);
        assert!(
            AgentsState::AuthRequired {
                requirement: req.clone(),
                agent: AgentKind::ClaudeCode,
            }
            .is_authenticating()
        );
        assert!(
            AgentsState::LoginInProgress {
                requirement: req,
                agent: AgentKind::ClaudeCode,
            }
            .is_authenticating()
        );
        assert!(!AgentsState::Loading.is_authenticating());
    }

    #[test]
    fn start_then_cancel_login_returns_to_auth_required() {
        let req = dummy_requirement(AgentKind::Codex);
        let state = AgentsState::AuthRequired {
            requirement: req,
            agent: AgentKind::Codex,
        };
        let in_progress = state.start_login();
        assert!(matches!(in_progress, AgentsState::LoginInProgress { .. }));
        let back = in_progress.cancel_login();
        assert!(matches!(back, AgentsState::AuthRequired { .. }));
    }

    #[test]
    fn auth_pending_is_authenticating_and_recovers_to_login() {
        let req = dummy_requirement(AgentKind::ClaudeCode);
        let pending = AgentsState::AuthPending {
            requirement: req.clone(),
            agent: AgentKind::ClaudeCode,
        };
        assert!(pending.is_authenticating());
        let in_progress = pending.start_login();
        assert!(matches!(in_progress, AgentsState::LoginInProgress { .. }));
    }

    #[test]
    fn start_auth_pending_only_from_auth_required() {
        let req = dummy_requirement(AgentKind::Codex);
        let armed = AgentsState::AuthRequired {
            requirement: req,
            agent: AgentKind::Codex,
        };
        let pending = armed.start_auth_pending();
        assert!(matches!(pending, AgentsState::AuthPending { .. }));
        // No-op from Loading.
        let loading = AgentsState::Loading.start_auth_pending();
        assert!(matches!(loading, AgentsState::Loading));
    }

    #[test]
    fn load_error_titles_match_zed_variants() {
        assert_eq!(
            AgentLoadError::FailedToInstall("nope".into()).title(),
            "Failed to Install"
        );
        assert_eq!(
            AgentLoadError::Exited {
                status: 1,
                stderr: None,
            }
            .title(),
            "Failed to Launch"
        );
        assert_eq!(
            AgentLoadError::Other("io".into()).title(),
            "Failed to Launch"
        );
        assert_eq!(
            AgentLoadError::Unsupported {
                command: "claude".into(),
                current_version: "0.1".into(),
                minimum_version: "1.0".into(),
            }
            .title(),
            "Unsupported version"
        );
    }

    #[test]
    fn load_error_body_includes_stderr_when_present() {
        let body = AgentLoadError::Exited {
            status: 137,
            stderr: Some("OOM".to_string()),
        }
        .body();
        assert!(body.starts_with("Server exited with status 137"));
        assert!(body.contains("OOM"));

        let body = AgentLoadError::Exited {
            status: 1,
            stderr: None,
        }
        .body();
        assert!(!body.contains('\n'));
    }

    #[test]
    fn load_error_unsupported_body_handles_empty_version() {
        let body = AgentLoadError::Unsupported {
            command: "claude".into(),
            current_version: String::new(),
            minimum_version: "1.0".into(),
        }
        .body();
        assert!(body.contains("does not report a valid"));
    }

    #[test]
    fn load_failed_state_predicate() {
        let state = AgentsState::LoadFailed {
            error: AgentLoadError::Other("boom".into()),
        };
        assert!(state.is_load_failed());
        assert!(!state.is_authenticating());
    }
}
