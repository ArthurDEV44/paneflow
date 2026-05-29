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
//! EP-001 US-003 — MCP bridge extraction.
//!   `extract::ensure_bridge_extracted` materializes the embedded
//!   `paneflow-mcp` bridge to a stable, non-versioned path under
//!   `data_dir()` (`runtime_paths::bridge_binary_path`), distinct from the
//!   version-pinned helper cache above. Called once at launch from
//!   `main()`; the path is what `paneflow mcp install` (EP-002) writes into
//!   agent configs, so it must survive Paneflow updates.
//!
//! Future stories (not in scope for US-008):
//! - US-009 — PATH-prepend in `pty_session` using this extraction path.
//! - US-011 — end-to-end integration tests over the whole pipeline.

pub mod extract;
