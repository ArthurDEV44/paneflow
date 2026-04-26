//! Cross-platform AI-hook wiring.
//!
//! US-008 — binary extraction & cache-dir layout.
//!   `extract::ensure_binaries_extracted` materializes the embedded
//!   `paneflow-shim` and `paneflow-ai-hook` binaries into
//!   `<cache_dir>/paneflow/bin/<version>/` with atomic rename + `chmod
//!   0o755` on Unix. The shim is written twice (as `claude` and `codex`)
//!   so the PTY's `$PATH`-prepend in US-009 resolves both tool names to
//!   the same underlying shim.
//!
//! Future stories (not in scope for US-008):
//! - US-009 — PATH-prepend in `pty_session` using this extraction path.
//! - US-011 — end-to-end integration tests over the whole pipeline.

pub mod extract;
