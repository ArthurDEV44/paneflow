//! Typed error surface for [`crate::ThreadStore`].

use crate::store::ThreadId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ThreadStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("thread not found: {0}")]
    NotFound(ThreadId),

    #[error(
        "could not resolve a writable data directory; \
         pass an explicit path to ThreadStore::open_at"
    )]
    NoDataDir,
}
