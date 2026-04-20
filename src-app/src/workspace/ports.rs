//! Per-workspace TCP listening-port detection.
//!
//! Three platform branches:
//! - **Linux** — parses `/proc/net/tcp[6]` and `/proc/{pid}/fd/` socket inodes.
//! - **macOS** — uses `libproc::listpidinfo::<ListFDs>` + `pidfdinfo::<SocketFDInfo>`
//!   and filters TCP sockets in the `Listen` state.
//! - **Everything else (Windows, BSDs)** — stub returning `vec![]`.
//!
//! Descendant PID enumeration mirrors the platform: Linux walks
//! `/proc/{pid}/task/{pid}/children`; macOS uses `libc::proc_listchildpids`.
//! Both branches cap at 512 PIDs to bound memory on fork-bombs.
//!
//! Extracted from `workspace.rs` per US-030 of the src-app refactor PRD.

#[cfg(target_os = "linux")]
use super::git::read_capped;

/// Collect all descendant PIDs of the given PID by walking `/proc/{pid}/task/{tid}/children`.
/// Requires `CONFIG_PROC_CHILDREN=y` in the kernel; absent on some distributions.
/// Returns the input PID plus all recursive descendants. On non-Linux or on
/// read failure, returns only the input PID. Capped at 512 PIDs to bound
/// memory usage in fork-bomb scenarios.
#[cfg(target_os = "linux")]
fn collect_descendant_pids(root_pid: u32) -> Vec<u32> {
    const MAX_PIDS: usize = 512;
    let mut visited = std::collections::HashSet::new();
    visited.insert(root_pid);
    let mut result = vec![root_pid];
    let mut queue = vec![root_pid];
    while let Some(pid) = queue.pop() {
        if visited.len() >= MAX_PIDS {
            break;
        }
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(content) = read_capped(std::path::Path::new(&children_path), 4096) {
            for token in content.split_whitespace() {
                if let Ok(child_pid) = token.parse::<u32>()
                    && visited.insert(child_pid)
                {
                    result.push(child_pid);
                    queue.push(child_pid);
                }
            }
        }
    }
    result
}

/// Detect TCP listening ports belonging to any of the given PIDs or their descendants.
///
/// Parses `/proc/net/tcp` and `/proc/net/tcp6` directly, filtering for TCP state
/// `0A` (LISTEN) only. Cross-references socket inodes with the file descriptors
/// of descendant PIDs to determine ownership.
///
/// Returns a sorted, deduplicated `Vec<u16>`. On non-Linux or on read failure,
/// returns an empty Vec without panic.
#[cfg(target_os = "linux")]
pub fn detect_ports(pids: &[u32]) -> Vec<u16> {
    if pids.is_empty() {
        return vec![];
    }

    // Expand PIDs to include all descendant processes
    let mut all_pids = std::collections::HashSet::new();
    for &pid in pids {
        for descendant in collect_descendant_pids(pid) {
            all_pids.insert(descendant);
        }
    }

    // Collect all socket inodes owned by our PID set
    let owned_inodes = collect_socket_inodes(&all_pids);
    if owned_inodes.is_empty() {
        return vec![];
    }

    // Parse /proc/net/tcp and /proc/net/tcp6 for LISTEN-state sockets
    let mut ports: Vec<u16> = Vec::new();
    for path in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = read_capped(std::path::Path::new(path), 256 * 1024) {
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
                    && owned_inodes.contains(&inode)
                {
                    ports.push(port);
                }
            }
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Collect all socket inodes from `/proc/{pid}/fd/` for the given PID set.
#[cfg(target_os = "linux")]
fn collect_socket_inodes(pids: &std::collections::HashSet<u32>) -> std::collections::HashSet<u64> {
    let mut inodes = std::collections::HashSet::new();
    for &pid in pids {
        let fd_dir = format!("/proc/{pid}/fd");
        if let Ok(entries) = std::fs::read_dir(&fd_dir) {
            for entry in entries.flatten() {
                if let Ok(link) = std::fs::read_link(entry.path()) {
                    let link_str = link.to_string_lossy();
                    if let Some(rest) = link_str.strip_prefix("socket:[")
                        && let Some(inode_str) = rest.strip_suffix(']')
                        && let Ok(inode) = inode_str.parse::<u64>()
                    {
                        inodes.insert(inode);
                    }
                }
            }
        }
    }
    inodes
}

/// macOS descendant PID walker — kernel equivalent of the Linux
/// `/proc/{pid}/task/{pid}/children` traversal used above. BFS via
/// `libc::proc_listchildpids`, capped at 512 PIDs to bound memory if a
/// workspace ever hosts a fork-bomb (mirrors the Linux branch).
#[cfg(target_os = "macos")]
fn collect_descendant_pids_macos(root_pid: u32) -> Vec<u32> {
    const MAX_PIDS: usize = 512;
    const MAX_CHILDREN_PER_PROC: usize = 256;

    let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
    visited.insert(root_pid);
    let mut result = vec![root_pid];
    let mut queue = vec![root_pid];

    while let Some(pid) = queue.pop() {
        if visited.len() >= MAX_PIDS {
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
            // sandbox / SIP). Either way, skip this PID — AC4 requires no
            // panic and no noise on a routine permission denial.
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
                queue.push(child);
            }
        }
    }

    result
}

/// Detect TCP listening ports owned by any of the given PIDs or their
/// descendants on macOS.
///
/// Walks each PID's file descriptors via `libproc::listpidinfo::<ListFDs>`,
/// queries `pidfdinfo::<SocketFDInfo>` for every Socket FD, and filters to
/// TCP sockets in the `Listen` state.
///
/// `insi_lport` in `TcpSockInfo.tcpsi_ini` is the kernel's inpcb local port
/// cast to `c_int`; the low 16 bits hold the network-byte-order u16, so we
/// mask + `from_be` to get the host-order port.
#[cfg(target_os = "macos")]
pub fn detect_ports(pids: &[u32]) -> Vec<u16> {
    use libproc::libproc::file_info::{ListFDs, ProcFDType};
    use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind, TcpSIState};
    use libproc::libproc::proc_pid::{listpidinfo, pidfdinfo};

    if pids.is_empty() {
        return vec![];
    }

    // Mirror the Linux flow: expand to include descendants so a dev server
    // launched under `npm run dev` (node → vite-child → ...) is captured.
    let mut all_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for &pid in pids {
        for descendant in collect_descendant_pids_macos(pid) {
            all_pids.insert(descendant);
        }
    }

    // Typical ulimit default on macOS is 256–4096 FDs per process. 1024 is
    // a sensible over-provisioning ceiling — the buffer is uninitialised
    // memory so allocation cost is a single malloc, not a zeroing pass.
    const MAX_FDS_PER_PROC: usize = 1024;
    let mut ports: Vec<u16> = Vec::new();

    for pid in all_pids {
        let Ok(fds) = listpidinfo::<ListFDs>(pid as i32, MAX_FDS_PER_PROC) else {
            // EPERM / dead-process races / SIP-restricted targets → skip
            // silently (AC4). `listpidinfo` already wraps the error string
            // which is more noise than signal at warn level during normal
            // port-detection runs triggered by UI refresh.
            continue;
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
            // `soi_proto` union's `pri_tcp` arm is the active one. The
            // union is POD (`SocketInfoProto` holds `#[repr(C)]` structs
            // all the way down) so reading a different arm would only
            // produce garbage port bytes, not UB — but we gate on
            // `soi_kind` to keep the data meaningful.
            let tcp = unsafe { sfi.psi.soi_proto.pri_tcp };

            if TcpSIState::from(tcp.tcpsi_state) as i32 != TcpSIState::Listen as i32 {
                continue;
            }

            // `insi_lport` stores the inpcb's local port as `c_int` with the
            // network-byte-order u16 sitting in the low 16 bits. Mask to u16
            // then `from_be` to recover host-order.
            let net_port = (tcp.tcpsi_ini.insi_lport as u32 & 0xFFFF) as u16;
            let port = u16::from_be(net_port);
            if port != 0 {
                ports.push(port);
            }
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Stub for other non-Linux platforms (BSDs, Windows). Port detection is
/// a platform-specific syscall on each, outside the v0.2.0 scope.
///
/// US-008 (prd-windows-port.md) — verified: this cfg predicate covers
/// `target_os = "windows"` (no Linux, no macOS → stub selected). The
/// services sidebar renders empty on Windows v1 as a deliberate design
/// choice. A real Windows implementation would use
/// `GetExtendedTcpTable` / `GetExtendedUdpTable` with `TCPIP_OWNER_MODULE_BASIC_INFO`
/// to attribute listening ports to owning PIDs; that work is deferred to a
/// post-v1 PRD. US-022 surfaces this limitation in `docs/WINDOWS.md`.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn detect_ports(_pids: &[u32]) -> Vec<u16> {
    vec![]
}
