//! Per-session metadata (cwd) tracked for sandboxing.
//!
//! ACP requests like `WriteTextFileRequest` carry only the `session_id` and
//! a path. To enforce the "writes must be inside the session's cwd" rule
//! (US-002 AC + FR-10), the file-ops handler needs to look up the cwd that
//! was passed to `session/new`. This registry is that map.
//!
//! Callers register a session after the `NewSessionRequest` resolves; the
//! registry is shared (`Arc`) with all handlers wired by
//! [`crate::client::ClientConfig`].

use agent_client_protocol::schema::SessionId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub cwd: PathBuf,
}

#[derive(Clone, Default, Debug)]
pub struct SessionRegistry {
    inner: Arc<RwLock<HashMap<SessionId, SessionMeta>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, id: SessionId, cwd: PathBuf) {
        self.write().insert(id, SessionMeta { cwd });
    }

    pub fn unregister(&self, id: &SessionId) {
        self.write().remove(id);
    }

    pub fn cwd(&self, id: &SessionId) -> Option<PathBuf> {
        self.read().get(id).map(|m| m.cwd.clone())
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<SessionId, SessionMeta>> {
        match self.inner.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<SessionId, SessionMeta>> {
        match self.inner.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_id(s: &str) -> SessionId {
        SessionId::from(s.to_string())
    }

    #[test]
    fn register_and_lookup() {
        let reg = SessionRegistry::new();
        let id = session_id("sess-1");
        reg.register(id.clone(), PathBuf::from("/tmp/work"));
        assert_eq!(reg.cwd(&id), Some(PathBuf::from("/tmp/work")));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unregister_drops_entry() {
        let reg = SessionRegistry::new();
        let id = session_id("sess-2");
        reg.register(id.clone(), PathBuf::from("/tmp/work"));
        reg.unregister(&id);
        assert_eq!(reg.cwd(&id), None);
        assert!(reg.is_empty());
    }

    #[test]
    fn unknown_session_returns_none() {
        let reg = SessionRegistry::new();
        assert_eq!(reg.cwd(&session_id("nope")), None);
    }
}
