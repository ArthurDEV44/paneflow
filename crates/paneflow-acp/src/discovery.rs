//! PATH-based ACP agent discovery.
//!
//! Probes the user's `PATH` for the known ACP-capable CLIs (Claude Code,
//! Codex) plus the `bunx` runner used to launch their ACP wrapper
//! packages. Only agents whose underlying binary AND `bunx` are present
//! end up in the picker. We never bundle Node, npm, or any agent binary
//! ourselves -- install is the user's job (PRD FR-14, Non-Goals).
//!
//! Results are cached for the active window-focus session so repeated
//! UI reads do not re-hit `which::which`. The GPUI layer calls
//! [`AgentDiscovery::refresh`] when the window regains focus to pick up
//! agents installed in another terminal without requiring a Paneflow
//! restart.
//!
//! See US-004 of `tasks/prd-agents-view.md`.

use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The set of ACP agents Paneflow knows how to launch in v1. OpenCode is
/// deliberately deferred to v1.1 (the PRD's Non-Goals call this out --
/// OpenCode uses TCP transport, not stdio).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    /// Human-readable label used in the agent picker UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    /// Name of the underlying CLI binary that must be on PATH for this
    /// agent to work (the ACP wrapper shells out to it).
    pub fn binary_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    /// npm package spec for the ACP wrapper, suitable for `bunx -y`.
    pub fn acp_wrapper_pkg(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "@zed-industries/claude-code-acp@latest",
            Self::Codex => "@zed-industries/codex-acp@latest",
        }
    }

    /// All v1 agent kinds in display order.
    pub fn all() -> [AgentKind; 2] {
        [Self::ClaudeCode, Self::Codex]
    }
}

/// One discovered agent, ready to be handed to
/// [`crate::spawn_acp_agent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredAgent {
    pub kind: AgentKind,
    /// Full command string for [`crate::spawn_acp_agent`]. On POSIX this
    /// is `env -u CLAUDECODE bunx -y <pkg>` per the PRD AC; on Windows
    /// the `env` prefix is dropped because Windows has no `env`
    /// executable (the `spawn_acp_agent` helper still scrubs
    /// `CLAUDECODE` from the parent process).
    pub spawn_command: String,
    /// Resolved absolute path of the underlying agent CLI.
    pub binary_path: PathBuf,
    /// Resolved absolute path of `bunx` (the npm wrapper runner).
    pub runner_path: PathBuf,
}

/// PATH-lookup seam so tests can swap in a deterministic fake without
/// mutating the test process's real environment.
pub trait PathProbe: Send + Sync + 'static {
    fn find(&self, binary: &str) -> Option<PathBuf>;
}

/// Default [`PathProbe`] backed by [`which::which`].
pub struct SystemPathProbe;

impl PathProbe for SystemPathProbe {
    fn find(&self, binary: &str) -> Option<PathBuf> {
        which::which(binary).ok()
    }
}

/// Cached PATH probe for ACP agents.
pub struct AgentDiscovery {
    probe: Arc<dyn PathProbe>,
    cache: RwLock<Option<Vec<DiscoveredAgent>>>,
}

impl Default for AgentDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentDiscovery {
    /// Build a discovery service backed by the real system PATH.
    pub fn new() -> Self {
        Self::with_probe(Arc::new(SystemPathProbe))
    }

    /// Build a discovery service backed by a custom [`PathProbe`].
    /// Primarily used by tests; production code should call
    /// [`AgentDiscovery::new`].
    pub fn with_probe(probe: Arc<dyn PathProbe>) -> Self {
        Self {
            probe,
            cache: RwLock::new(None),
        }
    }

    /// Return the currently available agents, populating the cache on
    /// the first call. Subsequent calls within the same focus session
    /// reuse the cache (no `which::which` calls).
    pub fn list(&self) -> Vec<DiscoveredAgent> {
        if let Some(cached) = self.read_cache().clone() {
            return cached;
        }
        let fresh = self.probe_now();
        self.write_cache(Some(fresh.clone()));
        fresh
    }

    /// Invalidate the cache and re-probe. Call this when the Paneflow
    /// window regains focus so users who install an agent in another
    /// terminal see it without restarting Paneflow.
    pub fn refresh(&self) -> Vec<DiscoveredAgent> {
        self.write_cache(None);
        self.list()
    }

    /// True iff the cache currently holds a probe result.
    pub fn is_cached(&self) -> bool {
        self.read_cache().is_some()
    }

    fn probe_now(&self) -> Vec<DiscoveredAgent> {
        let runner_path = match self.probe.find("bunx") {
            Some(path) => path,
            None => {
                tracing::debug!(
                    target: "paneflow_acp::discovery",
                    "`bunx` not on PATH -- no ACP agents will be advertised",
                );
                return Vec::new();
            }
        };

        let mut found = Vec::new();
        for kind in AgentKind::all() {
            match self.probe.find(kind.binary_name()) {
                Some(binary_path) => {
                    let spawn_command = build_spawn_command(kind);
                    tracing::debug!(
                        target: "paneflow_acp::discovery",
                        agent = kind.display_name(),
                        binary = %binary_path.display(),
                        "discovered ACP agent",
                    );
                    found.push(DiscoveredAgent {
                        kind,
                        spawn_command,
                        binary_path,
                        runner_path: runner_path.clone(),
                    });
                }
                None => {
                    tracing::debug!(
                        target: "paneflow_acp::discovery",
                        agent = kind.display_name(),
                        binary = kind.binary_name(),
                        "agent binary not on PATH",
                    );
                }
            }
        }
        found
    }

    fn read_cache(&self) -> RwLockReadGuard<'_, Option<Vec<DiscoveredAgent>>> {
        match self.cache.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn write_cache(&self, value: Option<Vec<DiscoveredAgent>>) {
        let mut guard: RwLockWriteGuard<'_, _> = match self.cache.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *guard = value;
    }
}

#[cfg(unix)]
fn build_spawn_command(kind: AgentKind) -> String {
    // POSIX: prefix with `env -u CLAUDECODE` per US-004 AC. Redundant
    // with the parent-process scrub in `spawn_acp_agent` but matches
    // the spike pattern and reads correctly from a runbook copy-paste.
    format!("env -u CLAUDECODE bunx -y {}", kind.acp_wrapper_pkg())
}

#[cfg(not(unix))]
fn build_spawn_command(kind: AgentKind) -> String {
    // Windows has no `env` executable on the default PATH. The parent
    // process scrub in `spawn_acp_agent` is the cross-platform source
    // of truth for `CLAUDECODE` removal, so we just emit the plain
    // `bunx` invocation here.
    format!("bunx -y {}", kind.acp_wrapper_pkg())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Deterministic [`PathProbe`] backed by a swappable map of binary
    /// name -> resolved path. Tests can mutate the map to simulate
    /// installing or removing an agent without touching the real PATH.
    struct MockProbe {
        map: Mutex<HashMap<String, PathBuf>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockProbe {
        fn new(entries: &[(&str, &str)]) -> Arc<Self> {
            let map = entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), PathBuf::from(*v)))
                .collect();
            Arc::new(Self {
                map: Mutex::new(map),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn insert(&self, binary: &str, path: &str) {
            let mut g = match self.map.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.insert(binary.to_string(), PathBuf::from(path));
        }

        fn call_count(&self) -> usize {
            let g = match self.calls.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.len()
        }
    }

    impl PathProbe for MockProbe {
        fn find(&self, binary: &str) -> Option<PathBuf> {
            let mut calls = match self.calls.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            calls.push(binary.to_string());
            let map = match self.map.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            map.get(binary).cloned()
        }
    }

    fn expected_spawn(kind: AgentKind) -> String {
        build_spawn_command(kind)
    }

    #[test]
    fn both_agents_present_lists_claude_and_codex() {
        let probe = MockProbe::new(&[
            ("bunx", "/usr/bin/bunx"),
            ("claude", "/usr/bin/claude"),
            ("codex", "/usr/bin/codex"),
        ]);
        let discovery = AgentDiscovery::with_probe(probe);
        let agents = discovery.list();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].kind, AgentKind::ClaudeCode);
        assert_eq!(
            agents[0].spawn_command,
            expected_spawn(AgentKind::ClaudeCode)
        );
        assert_eq!(agents[0].binary_path, PathBuf::from("/usr/bin/claude"));
        assert_eq!(agents[0].runner_path, PathBuf::from("/usr/bin/bunx"));
        assert_eq!(agents[1].kind, AgentKind::Codex);
        assert_eq!(agents[1].spawn_command, expected_spawn(AgentKind::Codex));
    }

    #[test]
    fn only_claude_present_skips_codex() {
        let probe = MockProbe::new(&[("bunx", "/usr/bin/bunx"), ("claude", "/usr/bin/claude")]);
        let discovery = AgentDiscovery::with_probe(probe);
        let agents = discovery.list();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].kind, AgentKind::ClaudeCode);
    }

    #[test]
    fn missing_bunx_or_agents_returns_empty() {
        // bunx missing -> no agents even though claude is present.
        let probe = MockProbe::new(&[("claude", "/usr/bin/claude")]);
        let discovery = AgentDiscovery::with_probe(probe);
        assert!(discovery.list().is_empty());

        // bunx present but no agents -> empty.
        let probe = MockProbe::new(&[("bunx", "/usr/bin/bunx")]);
        let discovery = AgentDiscovery::with_probe(probe);
        assert!(discovery.list().is_empty());

        // Nothing at all -> empty.
        let probe = MockProbe::new(&[]);
        let discovery = AgentDiscovery::with_probe(probe);
        assert!(discovery.list().is_empty());
    }

    #[test]
    fn refresh_picks_up_newly_installed_agent_without_restart() {
        let probe = MockProbe::new(&[("bunx", "/usr/bin/bunx")]);
        let discovery = AgentDiscovery::with_probe(probe.clone());
        assert!(discovery.list().is_empty(), "initial probe sees no agents");
        // Simulate the user `bun install -g @anthropic-ai/claude-code`
        // in another terminal between focus events.
        probe.insert("claude", "/home/u/.bun/bin/claude");
        // Without refresh, the cache hides the new install.
        assert!(
            discovery.list().is_empty(),
            "cache must not re-probe by itself"
        );
        // refresh() drops the cache and re-probes -> Claude appears.
        let agents = discovery.refresh();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].kind, AgentKind::ClaudeCode);
    }

    #[test]
    fn cache_short_circuits_repeated_list_calls() {
        let probe = MockProbe::new(&[("bunx", "/usr/bin/bunx"), ("claude", "/usr/bin/claude")]);
        let discovery = AgentDiscovery::with_probe(probe.clone());
        let _ = discovery.list();
        let baseline = probe.call_count();
        // Subsequent calls hit the cache only.
        let _ = discovery.list();
        let _ = discovery.list();
        let _ = discovery.list();
        assert_eq!(
            probe.call_count(),
            baseline,
            "cached calls must not re-invoke the probe",
        );
        assert!(discovery.is_cached());
    }

    #[test]
    fn refresh_clears_cache_then_repopulates() {
        let probe = MockProbe::new(&[("bunx", "/usr/bin/bunx")]);
        let discovery = AgentDiscovery::with_probe(probe.clone());
        let _ = discovery.list();
        let after_first = probe.call_count();
        let _ = discovery.refresh();
        assert!(
            probe.call_count() > after_first,
            "refresh must trigger a re-probe",
        );
        assert!(discovery.is_cached(), "refresh leaves the cache populated");
    }
}
