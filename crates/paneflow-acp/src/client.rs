//! Pre-wired ACP client factory.
//!
//! [`ClientConfig`] bundles the four pieces a Paneflow agent thread needs:
//! a permission callback (Allow/Deny policy), a session registry (cwd
//! sandbox), a terminal spawner (delegated to `src-app` in US-018), and a
//! terminal registry (live PTY sessions). [`connect_with_handlers`] takes
//! that config plus a spawned [`AcpAgent`] and a caller-provided `main_fn`
//! and runs the conversation through a `Client::builder()` chain that has
//! every relevant ACP request type registered.
//!
//! The "Client builder factory" wording in the PRD (EP-001 DoD + US-002)
//! is intentionally encapsulated: the agent-client-protocol `Builder`'s
//! type signature changes with every `on_receive_request` call, so
//! exposing the partially-wired builder publicly would force every caller
//! to know the full handler stack. We hand back a `Result<R>` instead.

use crate::file_ops::{handle_read, handle_write, FileOpError};
use crate::permission::{map_decision, PermissionCallback};
use crate::session::SessionRegistry;
use crate::terminal::{
    handle_create as handle_create_terminal, handle_kill as handle_kill_terminal,
    handle_output as handle_output_terminal, handle_release as handle_release_terminal,
    handle_wait_for_exit as handle_wait_for_exit_terminal, TerminalRegistry, TerminalSpawner,
};
use agent_client_protocol::schema::{
    CreateTerminalRequest, ErrorCode, KillTerminalRequest, ReadTextFileRequest,
    ReleaseTerminalRequest, RequestPermissionRequest, SessionNotification, TerminalOutputRequest,
    WaitForTerminalExitRequest, WriteTextFileRequest,
};
use agent_client_protocol::{
    on_receive_notification, on_receive_request, AcpAgent, Agent, Client, ConnectionTo, Error,
};
use std::sync::Arc;

/// Caller-supplied dependencies for a paneflow-acp client.
#[derive(Clone)]
pub struct ClientConfig {
    pub permission_callback: Arc<dyn PermissionCallback>,
    pub sessions: SessionRegistry,
    pub terminal_spawner: Arc<dyn TerminalSpawner>,
    pub terminals: TerminalRegistry,
    pub notification_sink: Arc<dyn NotificationSink>,
}

/// Sink that receives every `SessionNotification` (the agent's stream:
/// `AgentMessageChunk`, `ToolCall`, `Plan`, etc.). The default no-op sink
/// is fine for headless tests; the UI provides a real sink that forwards
/// to the active `ThreadView` (US-013/US-015).
pub trait NotificationSink: Send + Sync + 'static {
    fn handle(&self, notification: SessionNotification);
}

/// No-op notification sink. Useful as a default in tests and during the
/// missing-agent empty state.
pub struct NoopNotificationSink;

impl NotificationSink for NoopNotificationSink {
    fn handle(&self, _notification: SessionNotification) {}
}

impl ClientConfig {
    /// Build a [`ClientConfig`] with a custom permission callback and
    /// terminal spawner. The session + terminal registries are created
    /// empty; clone the returned config to share them.
    pub fn new(
        permission_callback: Arc<dyn PermissionCallback>,
        terminal_spawner: Arc<dyn TerminalSpawner>,
    ) -> Self {
        Self {
            permission_callback,
            sessions: SessionRegistry::new(),
            terminal_spawner,
            terminals: TerminalRegistry::new(),
            notification_sink: Arc::new(NoopNotificationSink),
        }
    }

    pub fn with_notification_sink(mut self, sink: Arc<dyn NotificationSink>) -> Self {
        self.notification_sink = sink;
        self
    }
}

/// Connect `agent` through a fully-wired ACP client and drive the
/// conversation via `main_fn`. All inbound request types (permission,
/// file ops, terminal ops) are pre-handled; the notification stream is
/// forwarded to `config.notification_sink`.
///
/// The `main_fn` runs on the client side and gets a [`ConnectionTo<Agent>`]
/// it can use to send `InitializeRequest`, `NewSessionRequest`,
/// `PromptRequest`, etc.
pub async fn connect_with_handlers<R, F>(
    agent: AcpAgent,
    config: ClientConfig,
    main_fn: F,
) -> anyhow::Result<R>
where
    R: Send + 'static,
    F: AsyncFnOnce(ConnectionTo<Agent>) -> anyhow::Result<R> + Send + 'static,
{
    let cfg_permission = config.clone();
    let cfg_write = config.clone();
    let cfg_read = config.clone();
    let cfg_create_term = config.clone();
    let cfg_kill_term = config.clone();
    let cfg_output_term = config.clone();
    let cfg_wait_term = config.clone();
    let cfg_release_term = config.clone();
    let cfg_notification = config.clone();

    let result = Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                cfg_notification.notification_sink.handle(notification);
                Ok(())
            },
            on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let decision = cfg_permission.permission_callback.decide(&request).await;
                let response = map_decision(&request, decision);
                responder.respond(response)
            },
            on_receive_request!(),
        )
        // File I/O off the JSON-RPC loop. The ACP framework's
        // own guidance says "Handlers Must Not Block" (see
        // `jsonrpc.rs:162` in `agent_client_protocol`) -- and the
        // runtime backing this client is a current-thread tokio
        // (see `paneflow_app::agents::runtime::run_blocking`), so
        // a `std::fs::read_to_string` inside a handler stalls
        // the entire JSON-RPC event loop. With Claude Code's
        // typical pattern of rapid sequential Read requests
        // (5-10 per turn), the accumulated stalls block the
        // agent's own follow-up tokens / responses, and the
        // turn observably interrupts mid-stream. `spawn_blocking`
        // moves the sync I/O to tokio's dedicated blocking thread
        // pool so the JSON-RPC reader keeps spinning.
        //
        // Zed avoids this same trap by routing reads through its
        // async worktree subsystem (`project.open_buffer(...)`)
        // -- we don't have one, so we hand the work to a
        // blocking thread instead.
        .on_receive_request(
            async move |request: WriteTextFileRequest, responder, _cx| {
                let sessions = cfg_write.sessions.clone();
                let outcome = tokio::task::spawn_blocking(move || handle_write(request, &sessions))
                    .await
                    .unwrap_or_else(|join_err| {
                        Err(crate::file_ops::FileOpError::Io(format!(
                            "write task panicked: {join_err}"
                        )))
                    });
                match outcome {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(file_op_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ReadTextFileRequest, responder, _cx| {
                let sessions = cfg_read.sessions.clone();
                let outcome = tokio::task::spawn_blocking(move || handle_read(request, &sessions))
                    .await
                    .unwrap_or_else(|join_err| {
                        Err(crate::file_ops::FileOpError::Io(format!(
                            "read task panicked: {join_err}"
                        )))
                    });
                match outcome {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(file_op_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CreateTerminalRequest, responder, _cx| {
                match handle_create_terminal(
                    request,
                    &cfg_create_term.terminal_spawner,
                    &cfg_create_term.terminals,
                )
                .await
                {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(terminal_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: KillTerminalRequest, responder, _cx| match handle_kill_terminal(
                request,
                &cfg_kill_term.terminals,
            )
            .await
            {
                Ok(response) => responder.respond(response),
                Err(err) => responder.respond_with_error(terminal_error_to_acp(err)),
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: TerminalOutputRequest, responder, _cx| {
                match handle_output_terminal(request, &cfg_output_term.terminals).await {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(terminal_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: WaitForTerminalExitRequest, responder, _cx| {
                match handle_wait_for_exit_terminal(request, &cfg_wait_term.terminals).await {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(terminal_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ReleaseTerminalRequest, responder, _cx| {
                match handle_release_terminal(request, &cfg_release_term.terminals).await {
                    Ok(response) => responder.respond(response),
                    Err(err) => responder.respond_with_error(terminal_error_to_acp(err)),
                }
            },
            on_receive_request!(),
        )
        .connect_with(agent, async move |cx| {
            main_fn(cx).await.map_err(|e| acp_internal_error(&e))
        })
        .await;

    result.map_err(|e| anyhow::anyhow!("ACP client error: {e}"))
}

fn acp_error(code: ErrorCode, message: impl Into<String>) -> Error {
    let code_i32: i32 = code.into();
    Error::new(code_i32, message)
}

fn acp_internal_error(err: &anyhow::Error) -> Error {
    acp_error(ErrorCode::InternalError, format!("{err:#}"))
}

fn file_op_error_to_acp(err: FileOpError) -> Error {
    let message = err.to_string();
    match err {
        FileOpError::NotInsideCwd { .. } | FileOpError::UnknownSession => {
            acp_error(ErrorCode::InvalidParams, message)
        }
        FileOpError::Io(_) => acp_error(ErrorCode::InternalError, message),
    }
}

fn terminal_error_to_acp(err: crate::terminal::TerminalError) -> Error {
    acp_error(ErrorCode::InternalError, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{always_allow, FnPermissionCallback, PermissionDecision};
    use crate::terminal::{TerminalError, TerminalSession};

    struct NullSpawner;
    impl TerminalSpawner for NullSpawner {
        fn create(
            &self,
            _request: &CreateTerminalRequest,
        ) -> Result<Arc<dyn TerminalSession>, TerminalError> {
            Err(TerminalError::Other(anyhow::anyhow!(
                "NullSpawner refuses all terminals (test-only)"
            )))
        }
    }

    #[test]
    fn config_clones_share_inner_state() {
        let cfg = ClientConfig::new(always_allow(), Arc::new(NullSpawner));
        let clone = cfg.clone();
        // The same Arc<SessionRegistry inner> is shared across clones.
        cfg.sessions.register(
            agent_client_protocol::schema::SessionId::from("a".to_string()),
            std::path::PathBuf::from("/tmp"),
        );
        assert_eq!(clone.sessions.len(), 1, "clones share inner state");
    }

    #[test]
    fn fn_callback_decides_allow_and_reject() {
        let allow = FnPermissionCallback::new(|_| PermissionDecision::AllowOnce);
        let always = FnPermissionCallback::new(|_| PermissionDecision::AllowAlways);
        let reject = FnPermissionCallback::new(|_| PermissionDecision::Reject);
        let _: Arc<dyn PermissionCallback> = allow;
        let _: Arc<dyn PermissionCallback> = always;
        let _: Arc<dyn PermissionCallback> = reject;
    }
}
