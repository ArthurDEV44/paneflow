//! Terminal session abstraction for the ACP `CreateTerminal` /
//! `KillTerminal` / `TerminalOutput` / `WaitForTerminalExit` /
//! `ReleaseTerminal` requests.
//!
//! paneflow-acp does NOT depend on a PTY crate -- that wiring lives in
//! `src-app` and lands in US-018 via [`TerminalSpawner`]. This module
//! provides:
//!
//! - [`TerminalSpawner`] / [`TerminalSession`]: the seams that consumers
//!   implement on top of their PTY layer.
//! - [`TerminalRegistry`]: tracks live sessions by id so the four follow-up
//!   requests (kill/output/wait/release) can look up the right session.
//! - The five `handle_*` request-side helpers that dispatch the inbound
//!   request to the registered session.

use agent_client_protocol::schema::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, KillTerminalResponse,
    ReleaseTerminalRequest, ReleaseTerminalResponse, TerminalExitStatus, TerminalId,
    TerminalOutputRequest, TerminalOutputResponse, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse,
};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Boxed future alias used in the terminal session trait so it stays
/// object-safe without depending on `async_trait`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result of a `terminal/output` snapshot, mapped onto
/// [`TerminalOutputResponse`] by [`handle_output`].
#[derive(Debug, Clone, Default)]
pub struct TerminalOutputSnapshot {
    pub output: String,
    pub truncated: bool,
    pub exit_status: Option<TerminalExitStatus>,
}

/// Errors that can surface from a [`TerminalSpawner`] / [`TerminalSession`]
/// implementation. Variants are intentionally coarse -- the consumer's
/// error type is wrapped via `Other(anyhow::Error)`.
#[derive(Debug)]
pub enum TerminalError {
    UnknownTerminalId(TerminalId),
    Other(anyhow::Error),
}

impl fmt::Display for TerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTerminalId(id) => write!(f, "unknown terminal_id `{id}`"),
            Self::Other(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for TerminalError {}

impl From<anyhow::Error> for TerminalError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value)
    }
}

/// Caller-provided factory that spawns a new terminal session per ACP
/// `CreateTerminalRequest`. The implementation lives in `src-app`
/// (US-018), wired on top of `pty_session::PtySession`.
pub trait TerminalSpawner: Send + Sync + 'static {
    fn create(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<Arc<dyn TerminalSession>, TerminalError>;
}

/// One live terminal session. Drops should release any owned resources;
/// `release()` is the explicit ACP-side hook for the same.
pub trait TerminalSession: Send + Sync + 'static {
    fn id(&self) -> TerminalId;
    fn snapshot(&self) -> BoxFuture<'_, Result<TerminalOutputSnapshot, TerminalError>>;
    fn wait_for_exit(&self) -> BoxFuture<'_, Result<TerminalExitStatus, TerminalError>>;
    fn kill(&self) -> BoxFuture<'_, Result<(), TerminalError>>;
    fn release(&self) -> BoxFuture<'_, Result<(), TerminalError>>;
}

/// Registry of live terminal sessions, keyed by their ACP `TerminalId`.
#[derive(Clone, Default)]
pub struct TerminalRegistry {
    inner: Arc<RwLock<HashMap<TerminalId, Arc<dyn TerminalSession>>>>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: Arc<dyn TerminalSession>) {
        let id = session.id();
        self.write().insert(id, session);
    }

    pub fn get(&self, id: &TerminalId) -> Option<Arc<dyn TerminalSession>> {
        self.read().get(id).cloned()
    }

    pub fn remove(&self, id: &TerminalId) -> Option<Arc<dyn TerminalSession>> {
        self.write().remove(id)
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<TerminalId, Arc<dyn TerminalSession>>> {
        match self.inner.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<TerminalId, Arc<dyn TerminalSession>>> {
        match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }
}

/// Spawn a new terminal session via `spawner` and register it under its
/// advertised [`TerminalId`].
pub async fn handle_create(
    request: CreateTerminalRequest,
    spawner: &Arc<dyn TerminalSpawner>,
    registry: &TerminalRegistry,
) -> Result<CreateTerminalResponse, TerminalError> {
    let session = spawner.create(&request)?;
    let id = session.id();
    registry.insert(session);
    Ok(CreateTerminalResponse::new(id))
}

/// Send a kill signal to the named terminal. The session is left in the
/// registry (caller releases it via [`handle_release`] when truly done).
pub async fn handle_kill(
    request: KillTerminalRequest,
    registry: &TerminalRegistry,
) -> Result<KillTerminalResponse, TerminalError> {
    let session = registry
        .get(&request.terminal_id)
        .ok_or_else(|| TerminalError::UnknownTerminalId(request.terminal_id.clone()))?;
    session.kill().await?;
    Ok(KillTerminalResponse::new())
}

/// Return an output snapshot of the named terminal.
pub async fn handle_output(
    request: TerminalOutputRequest,
    registry: &TerminalRegistry,
) -> Result<TerminalOutputResponse, TerminalError> {
    let session = registry
        .get(&request.terminal_id)
        .ok_or_else(|| TerminalError::UnknownTerminalId(request.terminal_id.clone()))?;
    let snap = session.snapshot().await?;
    let response = TerminalOutputResponse::new(snap.output, snap.truncated);
    Ok(if let Some(status) = snap.exit_status {
        response.exit_status(status)
    } else {
        response
    })
}

/// Block (asynchronously) until the named terminal exits.
pub async fn handle_wait_for_exit(
    request: WaitForTerminalExitRequest,
    registry: &TerminalRegistry,
) -> Result<WaitForTerminalExitResponse, TerminalError> {
    let session = registry
        .get(&request.terminal_id)
        .ok_or_else(|| TerminalError::UnknownTerminalId(request.terminal_id.clone()))?;
    let exit_status = session.wait_for_exit().await?;
    Ok(WaitForTerminalExitResponse::new(exit_status))
}

/// Release a terminal session: drop it from the registry and let the
/// session implementation tear down its resources.
pub async fn handle_release(
    request: ReleaseTerminalRequest,
    registry: &TerminalRegistry,
) -> Result<ReleaseTerminalResponse, TerminalError> {
    let session = registry
        .remove(&request.terminal_id)
        .ok_or_else(|| TerminalError::UnknownTerminalId(request.terminal_id.clone()))?;
    session.release().await?;
    Ok(ReleaseTerminalResponse::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::SessionId;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// Stateless mock session that just records which lifecycle methods
    /// have been called. Output snapshots return a canned string.
    struct MockSession {
        id: TerminalId,
        killed: AtomicBool,
        released: AtomicBool,
        exited: AtomicBool,
    }

    impl MockSession {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: TerminalId::from(id.to_string()),
                killed: AtomicBool::new(false),
                released: AtomicBool::new(false),
                exited: AtomicBool::new(false),
            })
        }
    }

    impl TerminalSession for MockSession {
        fn id(&self) -> TerminalId {
            self.id.clone()
        }
        fn snapshot(&self) -> BoxFuture<'_, Result<TerminalOutputSnapshot, TerminalError>> {
            Box::pin(async move {
                Ok(TerminalOutputSnapshot {
                    output: "hello".into(),
                    truncated: false,
                    exit_status: None,
                })
            })
        }
        fn wait_for_exit(&self) -> BoxFuture<'_, Result<TerminalExitStatus, TerminalError>> {
            self.exited.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(TerminalExitStatus::new().exit_code(0u32)) })
        }
        fn kill(&self) -> BoxFuture<'_, Result<(), TerminalError>> {
            self.killed.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(()) })
        }
        fn release(&self) -> BoxFuture<'_, Result<(), TerminalError>> {
            self.released.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(()) })
        }
    }

    struct MockSpawner {
        next_id: AtomicU64,
    }

    impl MockSpawner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                next_id: AtomicU64::new(1),
            })
        }
    }

    impl TerminalSpawner for MockSpawner {
        fn create(
            &self,
            _request: &CreateTerminalRequest,
        ) -> Result<Arc<dyn TerminalSession>, TerminalError> {
            let n = self.next_id.fetch_add(1, Ordering::SeqCst);
            let session = MockSession::new(&format!("term-{n}"));
            Ok(session)
        }
    }

    fn make_create_request() -> CreateTerminalRequest {
        CreateTerminalRequest::new(SessionId::from("sess".to_string()), "echo")
            .args(vec!["hi".to_string()])
    }

    #[tokio::test]
    async fn create_then_kill_output_wait_release_roundtrip() {
        let spawner: Arc<dyn TerminalSpawner> = MockSpawner::new();
        let registry = TerminalRegistry::new();

        let create_resp = handle_create(make_create_request(), &spawner, &registry)
            .await
            .expect("create");
        let id = create_resp.terminal_id.clone();
        assert_eq!(registry.len(), 1);

        let sess = SessionId::from("sess".to_string());
        let out = handle_output(
            TerminalOutputRequest::new(sess.clone(), id.clone()),
            &registry,
        )
        .await
        .expect("output");
        assert_eq!(out.output, "hello");
        assert!(!out.truncated);

        let kill = handle_kill(
            KillTerminalRequest::new(sess.clone(), id.clone()),
            &registry,
        )
        .await;
        assert!(kill.is_ok());

        let wait = handle_wait_for_exit(
            WaitForTerminalExitRequest::new(sess.clone(), id.clone()),
            &registry,
        )
        .await
        .expect("wait");
        assert_eq!(wait.exit_status.exit_code, Some(0));

        let rel = handle_release(
            ReleaseTerminalRequest::new(sess.clone(), id.clone()),
            &registry,
        )
        .await;
        assert!(rel.is_ok());
        assert!(registry.is_empty(), "release should drop the session");
    }

    #[tokio::test]
    async fn unknown_terminal_id_returns_error() {
        let registry = TerminalRegistry::new();
        let err = handle_kill(
            KillTerminalRequest::new(
                SessionId::from("sess".to_string()),
                TerminalId::from("ghost".to_string()),
            ),
            &registry,
        )
        .await
        .expect_err("ghost id must fail");
        assert!(
            matches!(err, TerminalError::UnknownTerminalId(_)),
            "got {err:?}"
        );
    }
}
