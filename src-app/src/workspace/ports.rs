//! Per-pane TCP listening-port + agent-process detection (EP-005 US-012).
//!
//! One entry point, [`scan_panes`]: given `(terminal_key, root_pid)` pairs,
//! it returns a per-terminal [`PaneScan`] attributing LISTEN ports and
//! recognised agent binaries to each terminal's PTY process subtree.
//!
//! Cost contract (US-012): the process table is traversed ONCE per tick —
//! a shared `visited` set spans all roots so no pid is walked twice, each
//! pid's `comm` is read at most once, and `/proc/net/tcp[6]` is parsed a
//! single time for the whole scan (the pre-refactor code re-walked the
//! descendants once for ports and once for agents, so this is strictly
//! cheaper per tick at any pane count).
//!
//! Three platform branches:
//! - **Linux** — `/proc/{pid}/task/{pid}/children` BFS, `/proc/{pid}/comm`,
//!   `/proc/{pid}/fd` socket inodes cross-referenced with `/proc/net/tcp[6]`.
//! - **macOS** — `libc::proc_listchildpids` BFS, `libproc` name +
//!   `listpidinfo::<ListFDs>`/`pidfdinfo::<SocketFDInfo>` (naturally
//!   per-pid, so per-subtree attribution needs no global socket table).
//! - **Everything else (Windows, BSDs)** — stub returning an empty map; the
//!   sidebar chips and tab badges degrade to absent without error (US-012
//!   AC, parity with the historical `detect_ports` stub).
//!
//! BFS (not DFS) ordering is load-bearing: US-013 picks the agent binary
//! NEAREST the subtree root ("the agent you launched, not its children"),
//! which is exactly breadth-first visit order. Both walkers cap at 512 PIDs
//! per root to bound memory on fork-bombs.

#[cfg(target_os = "linux")]
use super::git::read_capped;

/// Per-terminal scan result (EP-005 US-012).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneScan {
    /// Sorted, deduplicated LISTEN ports owned by the terminal's subtree.
    pub ports: Vec<u16>,
    /// Recognised agent binary names found in the subtree, in BFS
    /// (root-proximity) order, deduplicated. `first()` is the pane's
    /// identity-pill agent (US-013); the union across panes feeds the
    /// workspace-level `detected_agents` aggregate.
    pub agents: Vec<String>,
}

/// Soft cap on PIDs walked per root subtree (fork-bomb bound, both
/// platforms). Checked at dequeue time, so one last fanout batch can
/// overshoot it by up to one process's child count — the bound is
/// "≈512", which is all the memory guarantee needs.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_PIDS_PER_ROOT: usize = 512;

// ---------------------------------------------------------------------------
// Platform-neutral pure helpers (unit-tested on every host)
// ---------------------------------------------------------------------------

/// Filter a BFS-ordered stream of process names down to recognised agent
/// binaries, preserving first-seen (nearest-root) order and deduplicating.
/// Exact basename match only — `claude-code-cli` or a wrapper script must
/// not trigger (parity with the historical `AI_PROCESS_NAMES` contract).
fn agents_in_bfs_order<'a>(
    comms_in_bfs_order: impl Iterator<Item = &'a str>,
    agent_binaries: &[&str],
) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for comm in comms_in_bfs_order {
        if agent_binaries.contains(&comm) && !found.iter().any(|f| f == comm) {
            found.push(comm.to_string());
            if found.len() == agent_binaries.len() {
                break;
            }
        }
    }
    found
}

/// Parse `/proc/net/tcp`-format content into `(port, socket_inode)` pairs
/// for LISTEN-state (0A) sockets. Pure string parsing, platform-neutral so
/// the fixture test runs on every host; malformed lines are skipped.
fn parse_listen_entries(content: &str) -> Vec<(u16, u64)> {
    let mut out = Vec::new();
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }
        // Field 3 is TCP state; 0A = LISTEN
        if fields[3] != "0A" {
            continue;
        }
        // Field 1 is local_address (hex_ip:hex_port)
        if let Some(port_hex) = fields[1].split(':').next_back()
            && let Ok(port) = u16::from_str_radix(port_hex, 16)
            && let Ok(inode) = fields[9].parse::<u64>()
        {
            out.push((port, inode));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

/// BFS the descendants of `root_pid` via `/proc/{pid}/task/{pid}/children`
/// (requires `CONFIG_PROC_CHILDREN=y`; absent kernels yield just the root).
/// `visited` is SHARED across the tick's roots so a pid reparented between
/// subtrees is only ever attributed once. Returns pids in breadth-first
/// order, root first.
#[cfg(target_os = "linux")]
fn bfs_descendants_linux(root_pid: u32, visited: &mut std::collections::HashSet<u32>) -> Vec<u32> {
    let mut result = Vec::new();
    if !visited.insert(root_pid) {
        return result;
    }
    result.push(root_pid);
    let mut queue = std::collections::VecDeque::from([root_pid]);
    while let Some(pid) = queue.pop_front() {
        if result.len() >= MAX_PIDS_PER_ROOT {
            break;
        }
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(content) = read_capped(std::path::Path::new(&children_path), 4096) {
            for token in content.split_whitespace() {
                if let Ok(child_pid) = token.parse::<u32>()
                    && visited.insert(child_pid)
                {
                    result.push(child_pid);
                    queue.push_back(child_pid);
                }
            }
        }
    }
    result
}

/// Collect socket inodes from `/proc/{pid}/fd/` for one PID.
#[cfg(target_os = "linux")]
fn socket_inodes_of(pid: u32, inodes: &mut Vec<u64>) {
    let fd_dir = format!("/proc/{pid}/fd");
    if let Ok(entries) = std::fs::read_dir(&fd_dir) {
        for entry in entries.flatten() {
            if let Ok(link) = std::fs::read_link(entry.path()) {
                let link_str = link.to_string_lossy();
                if let Some(rest) = link_str.strip_prefix("socket:[")
                    && let Some(inode_str) = rest.strip_suffix(']')
                    && let Ok(inode) = inode_str.parse::<u64>()
                {
                    inodes.push(inode);
                }
            }
        }
    }
}

/// Scan every terminal's PTY subtree in one pass (see module docs for the
/// cost contract). `roots` pairs an opaque caller key (the terminal entity
/// id) with the PTY child pid; `agent_binaries` is the recognition set —
/// derived by the caller from `TerminalAgent::ALL` (US-012 vocabulary
/// unification; matching is exact against `/proc/<pid>/comm`, which the
/// kernel truncates to 15 chars — every current binary name fits).
#[cfg(target_os = "linux")]
pub fn scan_panes(
    roots: &[(u64, u32)],
    agent_binaries: &[&str],
) -> std::collections::HashMap<u64, PaneScan> {
    let mut results: std::collections::HashMap<u64, PaneScan> = std::collections::HashMap::new();
    if roots.is_empty() {
        return results;
    }

    // 1. One shared subtree walk (each pid visited once per tick).
    let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut subtrees: Vec<(u64, Vec<u32>)> = Vec::with_capacity(roots.len());
    for &(key, root_pid) in roots {
        let pids = bfs_descendants_linux(root_pid, &mut visited);
        subtrees.push((key, pids));
    }

    // 2. Agents per subtree: read each pid's comm once, match in BFS order.
    //    3. Socket inodes per subtree → inode → subtree-index map.
    let mut inode_owner: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    for (idx, (key, pids)) in subtrees.iter().enumerate() {
        let comms: Vec<String> = if agent_binaries.is_empty() {
            Vec::new()
        } else {
            pids.iter()
                .filter_map(|pid| {
                    std::fs::read_to_string(format!("/proc/{pid}/comm"))
                        .ok()
                        .map(|s| s.trim().to_string())
                })
                .collect()
        };
        let agents = agents_in_bfs_order(comms.iter().map(String::as_str), agent_binaries);

        let mut inodes: Vec<u64> = Vec::new();
        for &pid in pids {
            socket_inodes_of(pid, &mut inodes);
        }
        for inode in inodes {
            // First owner wins; subtrees are disjoint (shared `visited`) so
            // a duplicate inode here means a shared/inherited socket — keep
            // the earlier (older pane) attribution deterministically.
            inode_owner.entry(inode).or_insert(idx);
        }

        results.insert(
            *key,
            PaneScan {
                ports: Vec::new(),
                agents,
            },
        );
    }

    // 4. /proc/net/tcp[6] parsed ONCE for the whole tick. The 256 KiB read
    //    cap (~1700 socket lines) truncates the tail on hosts with very
    //    many sockets; the failure mode is "some ports missing badges this
    //    tick" (inode-keyed attribution skips the cut lines), never a
    //    failed scan — `parse_listen_entries` drops the partial last line.
    let mut per_idx_ports: Vec<Vec<u16>> = vec![Vec::new(); subtrees.len()];
    for path in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = read_capped(std::path::Path::new(path), 256 * 1024) {
            for (port, inode) in parse_listen_entries(&content) {
                if let Some(&idx) = inode_owner.get(&inode) {
                    per_idx_ports[idx].push(port);
                }
            }
        }
    }
    for (idx, (key, _)) in subtrees.iter().enumerate() {
        let mut ports = std::mem::take(&mut per_idx_ports[idx]);
        ports.sort_unstable();
        ports.dedup();
        if let Some(scan) = results.get_mut(key) {
            scan.ports = ports;
        }
    }

    results
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

/// macOS descendant walker — kernel equivalent of the Linux
/// `/proc/{pid}/task/{pid}/children` traversal. BFS via
/// `libc::proc_listchildpids`; `visited` shared across roots (same
/// single-walk contract as Linux). Returns pids in breadth-first order.
#[cfg(target_os = "macos")]
fn bfs_descendants_macos(root_pid: u32, visited: &mut std::collections::HashSet<u32>) -> Vec<u32> {
    const MAX_CHILDREN_PER_PROC: usize = 256;

    let mut result = Vec::new();
    if !visited.insert(root_pid) {
        return result;
    }
    result.push(root_pid);
    let mut queue = std::collections::VecDeque::from([root_pid]);

    while let Some(pid) = queue.pop_front() {
        if result.len() >= MAX_PIDS_PER_ROOT {
            break;
        }

        let mut children_buf = vec![0i32; MAX_CHILDREN_PER_PROC];
        let buf_size = (children_buf.len() * std::mem::size_of::<i32>()) as libc::c_int;

        // SAFETY: `children_buf` is a mutable Vec<i32> with its full capacity
        // written (len == MAX_CHILDREN_PER_PROC). The kernel writes at most
        // `buf_size` bytes of `pid_t` (== i32) values; any tail beyond the
        // return value is ignored and truncated below.
        let written = unsafe {
            libc::proc_listchildpids(
                pid as libc::pid_t,
                children_buf.as_mut_ptr() as *mut libc::c_void,
                buf_size,
            )
        };

        if written <= 0 {
            // Either no children or the kernel denied the call (EPERM under
            // sandbox / SIP). Either way, skip this PID — no panic and no
            // noise on a routine permission denial.
            continue;
        }

        let count = (written as usize) / std::mem::size_of::<i32>();
        for &child_i32 in &children_buf[..count.min(MAX_CHILDREN_PER_PROC)] {
            if child_i32 <= 0 {
                continue;
            }
            let child = child_i32 as u32;
            if visited.insert(child) {
                result.push(child);
                queue.push_back(child);
            }
        }
    }

    result
}

/// macOS LISTEN ports for one PID, appended to `ports`.
///
/// Walks the PID's file descriptors via `libproc::listpidinfo::<ListFDs>`,
/// queries `pidfdinfo::<SocketFDInfo>` for every Socket FD, and filters to
/// TCP sockets in the `Listen` state. `insi_lport` in `TcpSockInfo.tcpsi_ini`
/// is the kernel's inpcb local port cast to `c_int`; the low 16 bits hold
/// the network-byte-order u16, so we mask + `from_be` to get host order.
#[cfg(target_os = "macos")]
fn listen_ports_of(pid: u32, ports: &mut Vec<u16>) {
    use libproc::libproc::file_info::{ListFDs, ProcFDType, pidfdinfo};
    use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind, TcpSIState};
    use libproc::libproc::proc_pid::listpidinfo;

    // Typical ulimit default on macOS is 256–4096 FDs per process. 1024 is
    // a sensible over-provisioning ceiling — the buffer is uninitialised
    // memory so allocation cost is a single malloc, not a zeroing pass.
    const MAX_FDS_PER_PROC: usize = 1024;

    let Ok(fds) = listpidinfo::<ListFDs>(pid as i32, MAX_FDS_PER_PROC) else {
        // EPERM / dead-process races / SIP-restricted targets → skip
        // silently. `listpidinfo` already wraps the error string, which is
        // more noise than signal during routine UI-triggered scans.
        return;
    };

    for fd in fds {
        if !matches!(ProcFDType::from(fd.proc_fdtype), ProcFDType::Socket) {
            continue;
        }

        let Ok(sfi) = pidfdinfo::<SocketFDInfo>(pid as i32, fd.proc_fd) else {
            continue;
        };

        if sfi.psi.soi_kind != SocketInfoKind::Tcp as libc::c_int {
            continue;
        }

        // SAFETY: when `soi_kind == Tcp`, the kernel guarantees the
        // `soi_proto` union's `pri_tcp` arm is the active one. The union is
        // POD (`SocketInfoProto` holds `#[repr(C)]` structs all the way
        // down) so reading a different arm would only produce garbage port
        // bytes, not UB — but we gate on `soi_kind` to keep the data
        // meaningful.
        let tcp = unsafe { sfi.psi.soi_proto.pri_tcp };

        if TcpSIState::from(tcp.tcpsi_state) as i32 != TcpSIState::Listen as i32 {
            continue;
        }

        let net_port = (tcp.tcpsi_ini.insi_lport as u32 & 0xFFFF) as u16;
        let port = u16::from_be(net_port);
        if port != 0 {
            ports.push(port);
        }
    }
}

/// Scan every terminal's PTY subtree in one pass (macOS). libproc's socket
/// queries are naturally per-pid, so per-subtree attribution falls out of
/// the BFS partition without a global socket table. Same shared-`visited` /
/// single-walk contract as the Linux branch.
#[cfg(target_os = "macos")]
pub fn scan_panes(
    roots: &[(u64, u32)],
    agent_binaries: &[&str],
) -> std::collections::HashMap<u64, PaneScan> {
    use libproc::libproc::proc_pid::name;

    let mut results: std::collections::HashMap<u64, PaneScan> = std::collections::HashMap::new();
    if roots.is_empty() {
        return results;
    }

    let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for &(key, root_pid) in roots {
        let pids = bfs_descendants_macos(root_pid, &mut visited);

        // `libproc::name` returns the kernel's `p_comm` — same semantics
        // and 16-char limit as Linux `/proc/<pid>/comm`. EPERM (sandbox /
        // SIP) skips silently.
        let comms: Vec<String> = if agent_binaries.is_empty() {
            Vec::new()
        } else {
            pids.iter()
                .filter_map(|&pid| name(pid as i32).ok().map(|n| n.trim().to_string()))
                .collect()
        };
        let agents = agents_in_bfs_order(comms.iter().map(String::as_str), agent_binaries);

        let mut ports: Vec<u16> = Vec::new();
        for &pid in &pids {
            listen_ports_of(pid, &mut ports);
        }
        ports.sort_unstable();
        ports.dedup();

        results.insert(key, PaneScan { ports, agents });
    }

    results
}

// ---------------------------------------------------------------------------
// Stub (Windows, BSDs)
// ---------------------------------------------------------------------------

/// Stub for other platforms. Port detection needs `GetExtendedTcpTable` +
/// owner-module attribution on Windows and is deferred to a post-v1 PRD
/// (US-022 surfaces the limitation in `docs/WINDOWS.md`). An empty map
/// means every tab renders without badges or pills and the workspace
/// aggregates stay empty — degradation without error (US-012/US-014 AC).
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn scan_panes(
    _roots: &[(u64, u32)],
    _agent_binaries: &[&str],
) -> std::collections::HashMap<u64, PaneScan> {
    std::collections::HashMap::new()
}

// ---------------------------------------------------------------------------
// Tests (platform-neutral helpers)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // US-013 AC: "plusieurs binaires agents dans le même sous-arbre → le
    // plus proche de la racine" — first-seen BFS order wins, duplicates
    // collapse to the first occurrence.
    #[test]
    fn agents_in_bfs_order_picks_nearest_root_first_and_dedups() {
        let comms = ["zsh", "claude", "node", "codex", "claude"];
        let agents = agents_in_bfs_order(comms.into_iter(), &["claude", "codex", "opencode"]);
        assert_eq!(agents, vec!["claude".to_string(), "codex".to_string()]);
    }

    #[test]
    fn agents_in_bfs_order_exact_match_only() {
        // A wrapper named `claude-code-cli` must not trigger (exact
        // basename contract, parity with the old AI_PROCESS_NAMES match).
        let comms = ["claude-code-cli", "Claude", "claudex"];
        assert!(agents_in_bfs_order(comms.into_iter(), &["claude"]).is_empty());
    }

    #[test]
    fn agents_in_bfs_order_empty_inputs() {
        assert!(agents_in_bfs_order(std::iter::empty(), &["claude"]).is_empty());
        assert!(agents_in_bfs_order(["claude"].into_iter(), &[]).is_empty());
    }

    #[test]
    fn parse_listen_entries_filters_listen_state_and_malformed_lines() {
        // Header + one LISTEN (port 0x1F90 = 8080, inode 4242) + one
        // ESTABLISHED (01) + one garbage line.
        let content = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
             0: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0000000000000000 100 0 0 10 0\n\
             1: 0100007F:0050 0100007F:1234 01 00000000:00000000 00:00000000 00000000  1000        0 9999 1 0000000000000000 100 0 0 10 0\n\
             garbage line\n";
        assert_eq!(parse_listen_entries(content), vec![(8080, 4242)]);
    }

    #[test]
    fn parse_listen_entries_empty_input() {
        assert!(parse_listen_entries("").is_empty());
        assert!(parse_listen_entries("header only\n").is_empty());
    }
}
