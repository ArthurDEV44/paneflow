//! Polymorphic agent connection abstraction (US-101).
//!
//! [`AgentConnection`] is the seam between the runtime (which owns the
//! session lifecycle) and the underlying agent backend. The single v1
//! implementation is [`AcpConnection`], a thin wrapper around the
//! [`ConnectionTo<Agent>`] handle returned by
//! [`crate::connect_with_handlers`]. Future backends -- a native LLM
//! driver, a Gemini CLI bridge, an OpenCode TCP client -- implement the
//! same trait without forcing every caller in `src-app/src/agents` to
//! branch on [`crate::AgentKind`]. See `docs/ZED_AGENT_REFERENCE.md` §3
//! for the upstream Zed pattern this mirrors.
//!
//! The trait deliberately reuses the schema types from
//! `agent_client_protocol::schema` (e.g. [`NewSessionResponse`],
//! [`PromptResponse`]) as the lingua franca. A non-ACP backend that
//! wants to play has to translate its native shape into these structs
//! once -- the runtime, the streaming pipeline, and the persistence
//! layer stay untouched.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use agent_client_protocol::schema::{
    AuthenticateRequest, AvailableCommand, CancelNotification, ContentBlock, InitializeRequest,
    InitializeResponse, LoadSessionRequest, LoadSessionResponse, ModelId, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, SessionId, SessionMode, SessionModeId,
    SetSessionModeRequest, SetSessionModelRequest,
};
use agent_client_protocol::{schema::AuthMethodId, Agent, ConnectionTo};

/// Boxed future alias used by [`AgentConnection`] to stay object-safe
/// without pulling in `async_trait`. Mirrors the alias already used by
/// [`crate::PermissionCallback`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Polymorphic abstraction over an established agent connection.
///
/// The v1 implementation is [`AcpConnection`]; future implementations
/// can drive non-ACP backends. The trait surface is intentionally
/// narrow -- only the ACP methods Paneflow uses today, plus the
/// capability flag named in `docs/ZED_AGENT_REFERENCE.md` §3 that the
/// runtime checks before deciding between native replay and remote
/// `session/load` (US-118).
///
/// Object-safe: `Send + Sync + 'static` so it can be shared via
/// `Arc<dyn AgentConnection>` across the runtime thread and any
/// future thread the caller wants to spawn (e.g. a notification
/// pump).
pub trait AgentConnection: Send + Sync + 'static {
    /// Send the ACP `initialize` handshake. Must be called once before
    /// any [`Self::new_session`] call.
    fn initialize(&self) -> BoxFuture<'_, anyhow::Result<InitializeResponse>>;

    /// Open a fresh agent session rooted at `cwd`. The returned
    /// [`NewSessionResponse`] carries the session id, available modes,
    /// and (when the `unstable_session_model` feature is enabled
    /// upstream) the model picker state.
    fn new_session(&self, cwd: PathBuf) -> BoxFuture<'_, anyhow::Result<NewSessionResponse>>;

    /// US-118: replay an existing agent session by id. Backends that
    /// do not support `session/load` (today's Claude Code @0.16 +
    /// Codex @0.14 wrappers) MUST keep [`Self::supports_load_session`]
    /// returning `false` AND return an error here -- the runtime
    /// branches on the flag and falls back to local replay-from-blob
    /// when it is `false`, so a default error implementation is the
    /// safe contract for trait consumers.
    fn load_session(
        &self,
        session_id: SessionId,
        cwd: PathBuf,
    ) -> BoxFuture<'_, anyhow::Result<LoadSessionResponse>> {
        let _ = session_id;
        let _ = cwd;
        Box::pin(async {
            Err(anyhow::anyhow!(
                "load_session not supported by this AgentConnection backend"
            ))
        })
    }

    /// Send a `session/prompt` with a fully-built block list. The
    /// returned [`PromptResponse`] carries the `stop_reason` the
    /// composer needs to morph Stop back into Send.
    fn prompt(
        &self,
        session_id: SessionId,
        blocks: Vec<ContentBlock>,
    ) -> BoxFuture<'_, anyhow::Result<PromptResponse>>;

    /// Fire-and-forget cancel of the in-flight turn for `session_id`.
    /// The agent emits a `Cancelled` stop reason shortly after; the
    /// runtime surfaces that via [`crate::client::NotificationSink`].
    fn cancel(&self, session_id: SessionId);

    /// Send the ACP `authenticate` request for the chosen method id.
    /// Today this is only exercised by [`crate::auth`] flows; the
    /// trait method exists so non-ACP backends can hook auth at the
    /// same point in the lifecycle.
    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'_, anyhow::Result<()>>;

    /// Switch the agent's session mode. Backends that do not support
    /// mid-session mode switching may return an error -- the runtime
    /// logs it and keeps the old mode.
    fn set_session_mode(
        &self,
        session_id: SessionId,
        mode_id: SessionModeId,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    /// Switch the agent's session model. Same semantics as
    /// [`Self::set_session_mode`] for backends without mid-session
    /// model switching.
    fn set_session_model(
        &self,
        session_id: SessionId,
        model_id: ModelId,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    /// Whether the backend supports the ACP `session/load` flow. When
    /// `false` (today's default for the Claude Code and Codex
    /// wrappers), the host replays the persisted blob locally; when
    /// `true`, US-118 wires a real `session/load` call instead. Default
    /// `false` so new backends opt in.
    fn supports_load_session(&self) -> bool {
        false
    }

    /// Cached modes from the most recent [`Self::new_session`] call.
    /// Empty until `new_session` resolves; used by the composer's mode
    /// picker pill.
    fn available_modes(&self) -> Vec<SessionMode>;

    /// Cached slash commands surfaced by the agent via the ACP
    /// `available_commands_update` notification. Empty for backends
    /// that do not stream commands or until the first notification
    /// arrives. US-112 plumbs the notification side; for now the
    /// [`AcpConnection`] implementation returns an empty vec.
    fn available_commands(&self) -> Vec<AvailableCommand>;
}

/// v1 implementation of [`AgentConnection`] backed by a live
/// [`ConnectionTo<Agent>`] handed back from
/// [`crate::connect_with_handlers`].
///
/// Construct it inside the `main_fn` closure of
/// `connect_with_handlers`, immediately after the connection
/// resolves; share it as `Arc<dyn AgentConnection>` if more than one
/// task needs to dispatch ACP calls. [`ConnectionTo`] is `Clone`, so
/// the wrapper imposes no extra locking on the dispatch path.
pub struct AcpConnection {
    connection: ConnectionTo<Agent>,
    state: RwLock<ConnectionState>,
}

#[derive(Default)]
struct ConnectionState {
    available_modes: Vec<SessionMode>,
    available_commands: Vec<AvailableCommand>,
}

impl AcpConnection {
    /// Wrap an established [`ConnectionTo<Agent>`] (typically obtained
    /// from the `main_fn` callback of [`crate::connect_with_handlers`]).
    pub fn new(connection: ConnectionTo<Agent>) -> Self {
        Self {
            connection,
            state: RwLock::new(ConnectionState::default()),
        }
    }

    /// Replace the cached slash commands. Called by US-112 when the
    /// ACP `available_commands_update` notification fires. Today,
    /// nothing calls this -- the trait's `available_commands()` thus
    /// returns an empty vec until US-112 ships.
    pub fn set_available_commands(&self, commands: Vec<AvailableCommand>) {
        let mut guard = self.write_state();
        guard.available_commands = commands;
    }

    fn read_state(&self) -> RwLockReadGuard<'_, ConnectionState> {
        match self.state.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn write_state(&self) -> RwLockWriteGuard<'_, ConnectionState> {
        match self.state.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }
}

impl AgentConnection for AcpConnection {
    fn initialize(&self) -> BoxFuture<'_, anyhow::Result<InitializeResponse>> {
        Box::pin(async move {
            self.connection
                .send_request(InitializeRequest::new(
                    agent_client_protocol::schema::ProtocolVersion::V1,
                ))
                .block_task()
                .await
                .map_err(|err| anyhow::anyhow!("initialize failed: {err}"))
        })
    }

    fn new_session(&self, cwd: PathBuf) -> BoxFuture<'_, anyhow::Result<NewSessionResponse>> {
        Box::pin(async move {
            let response = self
                .connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await
                .map_err(|err| anyhow::anyhow!("new_session failed: {err}"))?;
            if let Some(modes) = response.modes.as_ref() {
                let mut guard = self.write_state();
                guard.available_modes = modes.available_modes.clone();
            }
            Ok(response)
        })
    }

    fn load_session(
        &self,
        session_id: SessionId,
        cwd: PathBuf,
    ) -> BoxFuture<'_, anyhow::Result<LoadSessionResponse>> {
        // US-118: per the trait contract, [`Self::supports_load_session`]
        // gates the runtime's decision -- today it returns `false` for
        // every shipping ACP wrapper, so this branch is dead code under
        // production wiring. We still implement the dispatch so the
        // moment a Claude Code / Codex wrapper flips its capability
        // flag, no plumbing change is required on the Paneflow side.
        Box::pin(async move {
            let response = self
                .connection
                .send_request(LoadSessionRequest::new(session_id, cwd))
                .block_task()
                .await
                .map_err(|err| anyhow::anyhow!("load_session failed: {err}"))?;
            if let Some(modes) = response.modes.as_ref() {
                let mut guard = self.write_state();
                guard.available_modes = modes.available_modes.clone();
            }
            Ok(response)
        })
    }

    fn prompt(
        &self,
        session_id: SessionId,
        blocks: Vec<ContentBlock>,
    ) -> BoxFuture<'_, anyhow::Result<PromptResponse>> {
        Box::pin(async move {
            self.connection
                .send_request(PromptRequest::new(session_id, blocks))
                .block_task()
                .await
                .map_err(|err| anyhow::anyhow!("prompt failed: {err}"))
        })
    }

    fn cancel(&self, session_id: SessionId) {
        if let Err(err) = self
            .connection
            .send_notification(CancelNotification::new(session_id))
        {
            tracing::warn!(
                target: "paneflow_acp::connection",
                "cancel notification dispatch failed: {err}",
            );
        }
    }

    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.connection
                .send_request(AuthenticateRequest::new(method))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("authenticate failed: {err}"))
        })
    }

    fn set_session_mode(
        &self,
        session_id: SessionId,
        mode_id: SessionModeId,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.connection
                .send_request(SetSessionModeRequest::new(session_id, mode_id))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("set_session_mode failed: {err}"))
        })
    }

    fn set_session_model(
        &self,
        session_id: SessionId,
        model_id: ModelId,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.connection
                .send_request(SetSessionModelRequest::new(session_id, model_id))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("set_session_model failed: {err}"))
        })
    }

    fn available_modes(&self) -> Vec<SessionMode> {
        self.read_state().available_modes.clone()
    }

    fn available_commands(&self) -> Vec<AvailableCommand> {
        self.read_state().available_commands.clone()
    }
}
