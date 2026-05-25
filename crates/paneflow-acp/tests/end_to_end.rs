//! End-to-end integration tests for paneflow-acp.
//!
//! Each test spawns the `mock_acp_agent` binary through
//! [`paneflow_acp::spawn_acp_agent`], drives a complete ACP conversation
//! (initialize -> new session -> prompt -> stream), and asserts the
//! observable behaviour. Tests intentionally avoid touching the real
//! Claude Code or `bunx` toolchains -- the goal is to exercise the
//! paneflow-acp wiring in isolation.
//!
//! Covers US-003 AC #2 (happy-path streaming), #3 (permission round-trip
//! with allow + deny), and #4 (agent crash propagates to the client).

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, InitializeRequest, NewSessionRequest, PromptRequest,
    ProtocolVersion, RequestPermissionRequest, SessionNotification, SessionUpdate, TextContent,
};
use paneflow_acp::permission::{always_allow, always_deny, BoxFuture, PermissionDecision};
use paneflow_acp::{
    connect_with_handlers, spawn_acp_agent, ClientConfig, NotificationSink, PermissionCallback,
    TerminalError, TerminalSession, TerminalSpawner,
};
use std::sync::{Arc, Mutex};

const MOCK_SESSION_ID: &str = "mock-session-001";
const STREAM_EXPECTED: &str = "Hello from mock agent.";

/// Collects the concatenated text from every `AgentMessageChunk`
/// notification the agent emits, so tests can assert the streamed body.
#[derive(Default)]
struct TextCollector {
    chunks: Mutex<Vec<String>>,
}

impl TextCollector {
    fn joined(&self) -> String {
        let chunks = match self.chunks.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        chunks.concat()
    }
}

impl NotificationSink for TextCollector {
    fn handle(&self, notification: SessionNotification) {
        if let SessionUpdate::AgentMessageChunk(ContentChunk {
            content: ContentBlock::Text(TextContent { text, .. }),
            ..
        }) = notification.update
        {
            let mut chunks = match self.chunks.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            chunks.push(text);
        }
    }
}

/// Permission callback that records every request it sees (so the test
/// can introspect the agent's payload) and returns the configured
/// decision.
struct RecordingCallback {
    decision: PermissionDecision,
    seen: Mutex<Vec<RequestPermissionRequest>>,
}

impl RecordingCallback {
    fn new(decision: PermissionDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn count(&self) -> usize {
        let g = match self.seen.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.len()
    }

    fn first(&self) -> Option<RequestPermissionRequest> {
        let g = match self.seen.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.first().cloned()
    }
}

impl PermissionCallback for RecordingCallback {
    fn decide(&self, request: &RequestPermissionRequest) -> BoxFuture<'_, PermissionDecision> {
        let mut g = match self.seen.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.push(request.clone());
        let decision = self.decision;
        Box::pin(async move { decision })
    }
}

/// Terminal spawner stub. The mock agent never issues terminal requests,
/// so this never fires; if it did fire it would surface as an error.
struct UnusedSpawner;
impl TerminalSpawner for UnusedSpawner {
    fn create(
        &self,
        _request: &agent_client_protocol::schema::CreateTerminalRequest,
    ) -> Result<Arc<dyn TerminalSession>, TerminalError> {
        Err(TerminalError::Other(anyhow::anyhow!(
            "mock agent must not issue terminal requests in these tests"
        )))
    }
}

fn mock_agent_command(mode: &str) -> String {
    format!("{} {mode}", env!("CARGO_BIN_EXE_mock_acp_agent"))
}

fn make_config(
    callback: Arc<dyn PermissionCallback>,
    sink: Arc<dyn NotificationSink>,
) -> ClientConfig {
    ClientConfig::new(callback, Arc::new(UnusedSpawner)).with_notification_sink(sink)
}

/// Drive one full init -> new_session -> prompt round-trip against the
/// running mock agent.
async fn drive_one_prompt(
    agent: agent_client_protocol::AcpAgent,
    config: ClientConfig,
) -> anyhow::Result<()> {
    connect_with_handlers(agent, config, async |cx| {
        cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await
            .map_err(|e| anyhow::anyhow!("initialize failed: {e}"))?;
        let session = cx
            .send_request(NewSessionRequest::new(
                std::env::current_dir().unwrap_or_else(|_| "/".into()),
            ))
            .block_task()
            .await
            .map_err(|e| anyhow::anyhow!("new_session failed: {e}"))?;
        assert_eq!(session.session_id.0.as_ref(), MOCK_SESSION_ID);
        let prompt = PromptRequest::new(
            session.session_id.clone(),
            vec![ContentBlock::Text(TextContent::new("ping".to_string()))],
        );
        let response = cx
            .send_request(prompt)
            .block_task()
            .await
            .map_err(|e| anyhow::anyhow!("prompt failed: {e}"))?;
        assert!(matches!(
            response.stop_reason,
            agent_client_protocol::schema::StopReason::EndTurn
        ));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn happy_path_streams_three_chunks_then_ends() {
    // AC #2: init -> new session -> prompt -> stream collect -> done,
    // with the streamed text matching the agent's hardcoded concatenation.
    let sink = Arc::new(TextCollector::default());
    let agent = spawn_acp_agent(&mock_agent_command("happy"))
        .await
        .expect("spawn mock");
    let config = make_config(always_allow(), sink.clone());
    drive_one_prompt(agent, config)
        .await
        .expect("happy-path conversation");
    assert_eq!(sink.joined(), STREAM_EXPECTED);
}

#[tokio::test]
async fn permission_round_trip_allow_picks_first_option() {
    // AC #3 (happy half): the callback fires with the expected payload,
    // and returning `Allow` selects the first advertised option.
    let sink = Arc::new(TextCollector::default());
    let callback = RecordingCallback::new(PermissionDecision::AllowOnce);
    let agent = spawn_acp_agent(&mock_agent_command("permission"))
        .await
        .expect("spawn mock");
    let config = make_config(callback.clone(), sink.clone());
    drive_one_prompt(agent, config)
        .await
        .expect("permission round-trip");
    assert_eq!(callback.count(), 1, "callback must fire exactly once");
    let req = callback.first().expect("at least one request");
    assert_eq!(req.options.len(), 2, "mock advertises two options");
    assert_eq!(req.options[0].option_id.0.as_ref(), "allow-once");
    // The mock echoes the outcome into a fourth chunk; the joined text
    // contains the streamed body + the echoed decision.
    let joined = sink.joined();
    assert!(
        joined.starts_with(STREAM_EXPECTED),
        "joined text must start with the streamed body, got: {joined:?}",
    );
    assert!(
        joined.ends_with("SELECTED:allow-once"),
        "Allow decision must produce SELECTED:allow-once, got: {joined:?}",
    );
}

#[tokio::test]
async fn permission_round_trip_deny_yields_cancelled_outcome() {
    // AC #3 (unhappy half): Deny -> the agent receives `Cancelled`.
    let sink = Arc::new(TextCollector::default());
    let callback = RecordingCallback::new(PermissionDecision::Reject);
    let agent = spawn_acp_agent(&mock_agent_command("permission"))
        .await
        .expect("spawn mock");
    let config = make_config(callback.clone(), sink.clone());
    drive_one_prompt(agent, config)
        .await
        .expect("permission deny round-trip");
    assert_eq!(callback.count(), 1);
    let joined = sink.joined();
    assert!(
        joined.ends_with("CANCELLED"),
        "Deny decision must produce CANCELLED outcome, got: {joined:?}",
    );
}

#[tokio::test]
async fn agent_crash_surfaces_to_client_as_error() {
    // AC #4: the mock exits non-zero before reading any request; the
    // client must surface that as an error rather than hanging or
    // returning Ok.
    let sink = Arc::new(TextCollector::default());
    let agent = spawn_acp_agent(&mock_agent_command("crash"))
        .await
        .expect("spawn mock");
    let config = make_config(always_deny(), sink);
    let result = drive_one_prompt(agent, config).await;
    assert!(result.is_err(), "client must surface mock crash as Err");
}
