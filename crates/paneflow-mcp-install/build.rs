//! Build script: embed an `asInvoker` application manifest into this crate's
//! Windows (MSVC) test binary.
//!
//! Windows' installer-detection heuristic auto-elevates (UAC) any unmanifested
//! `.exe` whose name contains "install"/"setup"/"update". The crate name makes
//! the unit-test harness `paneflow_mcp_install-<hash>.exe`, so `cargo test`
//! cannot even launch it (os error 740, "the requested operation requires
//! elevation"). An embedded manifest that declares an explicit
//! `requestedExecutionLevel` disables installer detection, so the test binary
//! runs un-elevated like any other.
//!
//! Scope: only the MSVC-target link step of THIS crate's own binaries (its test
//! harness). The crate has no `links` key, so the flag never propagates to
//! `paneflow-app`, which links the lib as an rlib (no link step affected) and
//! supplies its own packaging manifest. Non-Windows / non-MSVC targets no-op.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let manifest = format!("{dir}\\paneflow-mcp-install.manifest");
    println!("cargo:rerun-if-changed={manifest}");
    // MSVC linker: embed the manifest as a resource in the linked binary. Both
    // flags are MSVC link.exe options; `rustc-link-arg` (unscoped) applies to
    // the linkable targets cargo builds for this crate (its test harness here),
    // never to the non-linking rlib it ships as a dependency.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest}");
}
