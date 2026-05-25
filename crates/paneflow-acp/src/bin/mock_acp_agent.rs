//! Mock ACP agent for paneflow-acp integration tests.
//!
//! Talks ACP over stdio, advertises `protocolVersion: 1`, returns a
//! deterministic session id, and on each `session/prompt` streams 3
//! hardcoded `AgentMessageChunk` notifications followed by a
//! `StopReason::EndTurn` response. The behaviour is selectable via the
//! first CLI argument (defaults to `happy` if absent). A CLI flag is
//! used rather than an env var so concurrent integration tests do not
//! step on each other's mode.
//!
//! - `happy` (default): stream three chunks, end turn.
//! - `permission`: stream the chunks, then issue one
//!   `RequestPermissionRequest` back to the client and echo the user's
//!   decision into a fourth chunk so tests can assert the round-trip.
//! - `crash`: exit non-zero before reading anything from stdin (the
//!   "stdin pipe closed early" case in US-003 AC #4).
//!
//! See US-003 of `tasks/prd-agents-view.md`.

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionId, PermissionOptionKind, PromptRequest,
    PromptResponse, ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCallId, ToolCallUpdate,
    ToolCallUpdateFields,
};
use agent_client_protocol::{on_receive_request, Agent, Stdio};

const STREAM_CHUNKS: &[&str] = &["Hello ", "from mock ", "agent."];
const MOCK_SESSION_ID: &str = "mock-session-001";
const PERMISSION_OPTION_ALLOW: &str = "allow-once";
const PERMISSION_OPTION_DENY: &str = "deny-once";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Happy,
    Permission,
    Crash,
}

impl Mode {
    fn from_argv() -> Self {
        match std::env::args().nth(1).as_deref().unwrap_or("happy") {
            "permission" => Self::Permission,
            "crash" => Self::Crash,
            _ => Self::Happy,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mode = Mode::from_argv();

    if mode == Mode::Crash {
        // AC #4: simulate agent crash. Exit non-zero before reading any
        // request, which closes stdin / stdout from the parent's POV.
        std::process::exit(7);
    }

    let build = Agent
        .builder()
        .on_receive_request(
            async |_request: InitializeRequest, responder, _cx| {
                responder.respond(InitializeResponse::new(ProtocolVersion::V1))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async |_request: NewSessionRequest, responder, _cx| {
                responder.respond(NewSessionResponse::new(SessionId::from(
                    MOCK_SESSION_ID.to_string(),
                )))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx| {
                let session_id = request.session_id.clone();
                for chunk in STREAM_CHUNKS {
                    let notif = SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new((*chunk).to_string()),
                        ))),
                    );
                    cx.send_notification(notif)?;
                }

                if mode == Mode::Permission {
                    // Per the SDK docs, calling `block_task().await` from
                    // inside an `on_receive_request` handler deadlocks
                    // the event loop. We use `on_receiving_result` to
                    // continue the turn from a spawned task once the
                    // client's permission outcome arrives.
                    let perm_req = RequestPermissionRequest::new(
                        session_id.clone(),
                        ToolCallUpdate::new(
                            ToolCallId::from("mock-tool".to_string()),
                            ToolCallUpdateFields::new(),
                        ),
                        vec![
                            PermissionOption::new(
                                PermissionOptionId::from(PERMISSION_OPTION_ALLOW.to_string()),
                                "Allow once",
                                PermissionOptionKind::AllowOnce,
                            ),
                            PermissionOption::new(
                                PermissionOptionId::from(PERMISSION_OPTION_DENY.to_string()),
                                "Deny once",
                                PermissionOptionKind::RejectOnce,
                            ),
                        ],
                    );
                    let cx_continue = cx.clone();
                    cx.send_request(perm_req)
                        .on_receiving_result(move |result| async move {
                            let outcome_text = match result {
                                Ok(perm_resp) => match perm_resp.outcome {
                                    RequestPermissionOutcome::Selected(sel) => {
                                        format!("SELECTED:{}", sel.option_id.0)
                                    }
                                    RequestPermissionOutcome::Cancelled => "CANCELLED".to_string(),
                                    _ => "UNKNOWN".to_string(),
                                },
                                Err(err) => format!("ERROR:{err}"),
                            };
                            let notif = SessionNotification::new(
                                session_id.clone(),
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new(outcome_text)),
                                )),
                            );
                            cx_continue.send_notification(notif)?;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })?;
                    return Ok(());
                }

                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            on_receive_request!(),
        );

    build
        .connect_to(Stdio::new())
        .await
        .map_err(|e| anyhow::anyhow!("mock_acp_agent exited with ACP error: {e}"))
}
