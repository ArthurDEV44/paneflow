//! US-101 (prd-agent-ui-refactor-2026-Q3): mock [`AgentConnection`]
//! impl driving `new_session → prompt → cancel`, on both the happy
//! and error paths. This is the regression suite that locks the
//! trait's contract: any future implementation must behave the same
//! way, and the runtime in `src-app/src/agents/runtime.rs` must still
//! observe the same outcomes through the trait surface.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Poison-recovery `.lock()` helper. Mirrors the pattern used in
/// `src-app/src/agents/runtime.rs::PermissionBroker` so the test
/// stays clippy-clean (the workspace policy warns on `.unwrap()`
/// outside `#[test]` functions, and these helpers live in `impl`
/// blocks).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

use agent_client_protocol::schema::{
    AuthMethodId, AvailableCommand, ContentBlock, InitializeResponse, LoadSessionResponse, ModelId,
    NewSessionResponse, PromptResponse, ProtocolVersion, SessionId, SessionMode, SessionModeId,
    StopReason, TextContent,
};
use paneflow_acp::{AgentConnection, BoxFuture, PermissionDecision};

/// Mock that records every dispatch and returns caller-configured
/// results. Each `Mutex<Option<...>>` plays the role of a one-shot
/// stub: the test seeds it before calling the trait method, and the
/// method drains it. Anything left untouched stays `None`.
struct MockConnection {
    initialize_response: Mutex<Option<anyhow::Result<InitializeResponse>>>,
    new_session_response: Mutex<Option<anyhow::Result<NewSessionResponse>>>,
    prompt_response: Mutex<Option<anyhow::Result<PromptResponse>>>,
    available_modes_state: Mutex<Vec<SessionMode>>,
    available_commands_state: Mutex<Vec<AvailableCommand>>,
    calls: Mutex<Vec<TraitCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TraitCall {
    Initialize,
    NewSession {
        cwd: PathBuf,
    },
    Prompt {
        session_id: String,
        block_count: usize,
    },
    Cancel {
        session_id: String,
    },
    Authenticate {
        method: String,
    },
    SetSessionMode {
        session_id: String,
        mode_id: String,
    },
    SetSessionModel {
        session_id: String,
        model_id: String,
    },
}

impl MockConnection {
    fn new() -> Self {
        Self {
            initialize_response: Mutex::new(Some(Ok(InitializeResponse::new(ProtocolVersion::V1)))),
            new_session_response: Mutex::new(None),
            prompt_response: Mutex::new(None),
            available_modes_state: Mutex::new(Vec::new()),
            available_commands_state: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn seed_new_session(&self, result: anyhow::Result<NewSessionResponse>) {
        *lock(&self.new_session_response) = Some(result);
    }

    fn seed_prompt(&self, result: anyhow::Result<PromptResponse>) {
        *lock(&self.prompt_response) = Some(result);
    }

    fn seed_available_modes(&self, modes: Vec<SessionMode>) {
        *lock(&self.available_modes_state) = modes;
    }

    fn seed_available_commands(&self, commands: Vec<AvailableCommand>) {
        *lock(&self.available_commands_state) = commands;
    }

    fn record(&self, call: TraitCall) {
        lock(&self.calls).push(call);
    }

    fn calls(&self) -> Vec<TraitCall> {
        lock(&self.calls).clone()
    }
}

impl AgentConnection for MockConnection {
    fn initialize(&self) -> BoxFuture<'_, anyhow::Result<InitializeResponse>> {
        self.record(TraitCall::Initialize);
        Box::pin(async move {
            lock(&self.initialize_response)
                .take()
                .unwrap_or_else(|| Ok(InitializeResponse::new(ProtocolVersion::V1)))
        })
    }

    fn new_session(&self, cwd: PathBuf) -> BoxFuture<'_, anyhow::Result<NewSessionResponse>> {
        self.record(TraitCall::NewSession { cwd: cwd.clone() });
        Box::pin(async move {
            lock(&self.new_session_response).take().unwrap_or_else(|| {
                Ok(NewSessionResponse::new(SessionId::from(
                    "sess-1".to_string(),
                )))
            })
        })
    }

    fn prompt(
        &self,
        session_id: SessionId,
        blocks: Vec<ContentBlock>,
    ) -> BoxFuture<'_, anyhow::Result<PromptResponse>> {
        self.record(TraitCall::Prompt {
            session_id: session_id.0.to_string(),
            block_count: blocks.len(),
        });
        Box::pin(async move {
            lock(&self.prompt_response)
                .take()
                .unwrap_or_else(|| Ok(PromptResponse::new(StopReason::EndTurn)))
        })
    }

    fn cancel(&self, session_id: SessionId) {
        self.record(TraitCall::Cancel {
            session_id: session_id.0.to_string(),
        });
    }

    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'_, anyhow::Result<()>> {
        self.record(TraitCall::Authenticate {
            method: method.0.to_string(),
        });
        Box::pin(async move { Ok(()) })
    }

    fn set_session_mode(
        &self,
        session_id: SessionId,
        mode_id: SessionModeId,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        self.record(TraitCall::SetSessionMode {
            session_id: session_id.0.to_string(),
            mode_id: mode_id.0.to_string(),
        });
        Box::pin(async move { Ok(()) })
    }

    fn set_session_model(
        &self,
        session_id: SessionId,
        model_id: ModelId,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        self.record(TraitCall::SetSessionModel {
            session_id: session_id.0.to_string(),
            model_id: model_id.to_string(),
        });
        Box::pin(async move { Ok(()) })
    }

    fn available_modes(&self) -> Vec<SessionMode> {
        lock(&self.available_modes_state).clone()
    }

    fn available_commands(&self) -> Vec<AvailableCommand> {
        lock(&self.available_commands_state).clone()
    }
}

/// AC #1 / #4 — happy path: new_session resolves, prompt resolves
/// with EndTurn, cancel records the dispatch. Trait is object-safe
/// (we drive it through `&dyn AgentConnection`).
#[tokio::test]
async fn happy_path_new_session_prompt_cancel() {
    let mock = MockConnection::new();
    let session = SessionId::from("happy-session".to_string());
    mock.seed_new_session(Ok(NewSessionResponse::new(session.clone())));
    mock.seed_prompt(Ok(PromptResponse::new(StopReason::EndTurn)));

    let conn: &dyn AgentConnection = &mock;
    let cwd = PathBuf::from("/tmp/happy");

    // initialize() first -- mirrors runtime.rs's call order.
    let init = conn.initialize().await.expect("initialize Ok");
    assert_eq!(init.protocol_version, ProtocolVersion::V1);

    // new_session returns the seeded session id.
    let resp = conn.new_session(cwd.clone()).await.expect("new_session Ok");
    assert_eq!(resp.session_id, session);

    // prompt returns the seeded EndTurn.
    let prompt_resp = conn
        .prompt(
            session.clone(),
            vec![ContentBlock::Text(TextContent::new("hi"))],
        )
        .await
        .expect("prompt Ok");
    assert_eq!(prompt_resp.stop_reason, StopReason::EndTurn);

    // cancel is fire-and-forget -- no return value to check, just
    // confirm it doesn't panic and recorded the dispatch.
    conn.cancel(session.clone());

    let calls = mock.calls();
    assert_eq!(
        calls,
        vec![
            TraitCall::Initialize,
            TraitCall::NewSession { cwd },
            TraitCall::Prompt {
                session_id: "happy-session".to_string(),
                block_count: 1,
            },
            TraitCall::Cancel {
                session_id: "happy-session".to_string(),
            },
        ],
    );
}

/// AC #4 unhappy path: when `prompt` returns an error, the caller
/// observes that error verbatim. This is the contract the runtime
/// loop relies on to surface a `RuntimeEvent::Fatal` and exit cleanly.
#[tokio::test]
async fn prompt_error_propagates_to_caller() {
    let mock = MockConnection::new();
    mock.seed_prompt(Err(anyhow::anyhow!("simulated prompt failure")));

    let conn: &dyn AgentConnection = &mock;
    let err = conn
        .prompt(
            SessionId::from("sess-err".to_string()),
            vec![ContentBlock::Text(TextContent::new("anything"))],
        )
        .await
        .expect_err("prompt must surface the seeded error");

    assert!(
        err.to_string().contains("simulated prompt failure"),
        "expected wrapped error message, got: {err}",
    );
}

/// AC #1: the trait surface compiles with `Arc<dyn AgentConnection>`
/// — i.e. it is object-safe — and the default `supports_load_session`
/// returns `false` for any new backend that doesn't override it.
#[tokio::test]
async fn trait_is_object_safe_and_defaults_to_no_load_session() {
    let conn: std::sync::Arc<dyn AgentConnection> = std::sync::Arc::new(MockConnection::new());
    assert!(
        !conn.supports_load_session(),
        "default supports_load_session must be false (US-118 plumbing)",
    );
    // available_modes / available_commands start empty on a fresh mock.
    assert!(conn.available_modes().is_empty());
    assert!(conn.available_commands().is_empty());
}

/// Trait getters round-trip the cached state mutated outside the
/// trait surface (mirrors how the v1 `AcpConnection` populates its
/// own cache from the new_session response and -- eventually --
/// US-112's `available_commands_update` notification).
#[tokio::test]
async fn cached_getters_reflect_seeded_state() {
    let mock = MockConnection::new();
    mock.seed_available_modes(vec![SessionMode::new(
        SessionModeId::from("default".to_string()),
        "Default".to_string(),
    )]);
    mock.seed_available_commands(vec![AvailableCommand::new("init", "Initialize project")]);

    let conn: &dyn AgentConnection = &mock;
    assert_eq!(conn.available_modes().len(), 1);
    assert_eq!(conn.available_modes()[0].name, "Default");
    assert_eq!(conn.available_commands().len(), 1);
    assert_eq!(conn.available_commands()[0].name, "init");
}

// PermissionDecision is unrelated to the AgentConnection trait surface
// but the test module relies on the same import chain that callers in
// `runtime.rs` use. Touching the type here keeps the import surface
// honest (if PermissionDecision is renamed, this test fails before
// the prod loop does).
#[test]
fn permission_decision_alive() {
    let _ = PermissionDecision::AllowOnce;
    let _ = PermissionDecision::AllowAlways;
    let _ = PermissionDecision::Reject;
}

/// US-118 AC #1 / #2: a backend that does NOT override
/// `supports_load_session` falls into the trait's default
/// implementation of `load_session`, which returns an error so the
/// runtime falls back to the local replay path.
#[tokio::test]
async fn default_load_session_returns_unsupported_error() {
    let mock = MockConnection::new();
    let conn: &dyn AgentConnection = &mock;
    assert!(!conn.supports_load_session());
    let err = conn
        .load_session(
            SessionId::from("any-session".to_string()),
            PathBuf::from("/tmp"),
        )
        .await
        .expect_err("default load_session must surface an error");
    assert!(
        err.to_string().contains("not supported"),
        "expected 'not supported' message, got: {err}",
    );
}

/// US-118 AC #2: a backend that opts in to `supports_load_session`
/// can override `load_session` to return a real response. The runtime
/// branches on the flag so this is the path that lights up the day a
/// shipping wrapper flips its capability.
#[tokio::test]
async fn opt_in_backend_returns_load_session_response() {
    struct LoadCapableMock;
    impl AgentConnection for LoadCapableMock {
        fn initialize(&self) -> BoxFuture<'_, anyhow::Result<InitializeResponse>> {
            Box::pin(async { Ok(InitializeResponse::new(ProtocolVersion::V1)) })
        }
        fn new_session(&self, _cwd: PathBuf) -> BoxFuture<'_, anyhow::Result<NewSessionResponse>> {
            Box::pin(async {
                Ok(NewSessionResponse::new(SessionId::from(
                    "should-not-be-used".to_string(),
                )))
            })
        }
        fn prompt(
            &self,
            _session_id: SessionId,
            _blocks: Vec<ContentBlock>,
        ) -> BoxFuture<'_, anyhow::Result<PromptResponse>> {
            Box::pin(async { Ok(PromptResponse::new(StopReason::EndTurn)) })
        }
        fn cancel(&self, _session_id: SessionId) {}
        fn authenticate(&self, _method: AuthMethodId) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn set_session_mode(
            &self,
            _session_id: SessionId,
            _mode_id: SessionModeId,
        ) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn set_session_model(
            &self,
            _session_id: SessionId,
            _model_id: ModelId,
        ) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn supports_load_session(&self) -> bool {
            true
        }
        fn load_session(
            &self,
            _session_id: SessionId,
            _cwd: PathBuf,
        ) -> BoxFuture<'_, anyhow::Result<LoadSessionResponse>> {
            Box::pin(async { Ok(LoadSessionResponse::default()) })
        }
        fn available_modes(&self) -> Vec<SessionMode> {
            Vec::new()
        }
        fn available_commands(&self) -> Vec<AvailableCommand> {
            Vec::new()
        }
    }

    let conn: &dyn AgentConnection = &LoadCapableMock;
    assert!(
        conn.supports_load_session(),
        "opt-in backend must report the capability",
    );
    let resp = conn
        .load_session(
            SessionId::from("resume-sess-42".to_string()),
            PathBuf::from("/tmp/resume"),
        )
        .await
        .expect("opt-in backend returns Ok");
    // The default LoadSessionResponse has no modes / models -- the
    // assertion is structural: the call resolved instead of erroring.
    assert!(resp.modes.is_none());
}
