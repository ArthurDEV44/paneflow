//! PTY backend abstraction — platform-agnostic terminal spawning (US-008).
//!
//! Defines the `PtyBackend` trait and `PortablePtyBackend` implementation.
//! This module has no GPUI imports and is independently testable.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, PtySize};

/// Result of spawning a PTY process.
pub struct PtyProcess {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    pub child_pid: u32,
}

/// Abstraction over PTY creation. Implementations provide platform-specific
/// PTY spawning (Unix openpty, Windows ConPTY) or test mocks.
pub trait PtyBackend: Send + Sync {
    fn spawn(
        &self,
        command: &str,
        args: &[String],
        cwd: &Path,
        env: &HashMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> anyhow::Result<PtyProcess>;
}

/// Production backend using `portable-pty` (openpty on Unix, ConPTY on Windows).
pub struct PortablePtyBackend;

impl PtyBackend for PortablePtyBackend {
    fn spawn(
        &self,
        command: &str,
        args: &[String],
        cwd: &Path,
        env: &HashMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> anyhow::Result<PtyProcess> {
        let pty_system = portable_pty::native_pty_system();
        let pty_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system.openpty(pty_size).map_err(|e| {
            log::error!("Failed to create PTY: {e}");
            anyhow::anyhow!("Failed to create terminal: {e}")
        })?;

        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);
        // Remove SHLVL so the child shell starts fresh at 1 (not inherited+1)
        cmd.env_remove("SHLVL");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            log::error!("Failed to spawn shell '{command}': {e}");
            anyhow::anyhow!("Failed to spawn shell: {e}")
        })?;
        let child_pid = child.process_id().unwrap_or(0);
        // Close our copy of the slave FD — only the child should hold it.
        // Otherwise the master reader won't see EOF when the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("Failed to clone PTY reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("Failed to take PTY writer: {e}"))?;
        let master = Arc::new(Mutex::new(pair.master));

        Ok(PtyProcess {
            reader,
            writer,
            child,
            master,
            child_pid,
        })
    }
}
