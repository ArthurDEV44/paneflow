// Build scripts idiomatically `panic!` on fatal errors — that is how
// Cargo surfaces build-time failures to the user. The workspace-wide
// `clippy::panic = "deny"` policy targets production runtime code, not
// build tooling; a `?`-returning `main() -> Result<…>` here would only
// produce worse error messages via `Termination`. Allow-listed at file
// level with this justification.
#![allow(clippy::panic)]

//! Build script for `paneflow-app`.
//!
//! Responsibilities:
//! 1. Invalidate the build when telemetry-related compile-time env vars
//!    change. `option_env!("POSTHOG_API_KEY")` and
//!    `option_env!("POSTHOG_HOST")` are resolved at compile time (see
//!    `src-app/src/app/bootstrap.rs`); without these `rerun-if-env-changed`
//!    directives Cargo has no way to know the macro output depends on those
//!    vars, so rotating the key or host in CI would produce a binary that
//!    still embeds the previous value until an unrelated source change
//!    forces a rebuild.
//!
//! 2. **US-008 — AI-hook binary staging.** Build the
//!    `paneflow-shim` and `paneflow-ai-hook` workspace binaries for the
//!    current target triple and stage them into
//!    `src-app/target/embed/bin/<target>/` so the `Bins` `RustEmbed` struct
//!    in `src-app/src/assets.rs` picks them up at compile time. A nested
//!    `cargo build` is used rather than relying on workspace build ordering
//!    because `paneflow-app` does not directly depend on either of those
//!    crates — without this step they would not be guaranteed to exist when
//!    `rust-embed` expands.
//!
//!    The nested build uses a **separate `--target-dir`**
//!    (`<workspace>/target/embed-build`) so it does not fight the outer
//!    cargo for the same target-dir lock. The cost is duplicated
//!    compilation of the shim + hook dependency closure; both closures are
//!    tiny (serde_json, tempfile, interprocess) so the overhead is
//!    acceptable and far cheaper than designing a shared build graph.
//!
//!    Size budget: total embedded bytes per target triple must stay
//!    ≤ 1 MB. The check fails the outer build when exceeded rather than
//!    silently shipping a bloated `paneflow` binary.
//!
//!    Escape hatch: setting `PANEFLOW_SKIP_EMBED_BUILD=1` skips the nested
//!    build — useful in CI pre-stages that build the nested crates
//!    separately and pre-populate `target/embed/bin/<target>/`, and for
//!    fast iteration on the main crate when the nested binaries have not
//!    changed. The staging dir must still be populated when the `Bins`
//!    `RustEmbed` macro expands — rust-embed 8.x panics on missing folders.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Hard cap on the total bytes staged under `target/embed/bin/<target>/`.
/// Enforced to keep the main PaneFlow binary slim; the PRD sets the
/// per-OS/per-arch budget at ≤ 1 MB with a month-3 target of ≤ 1.0 MB.
const EMBED_SIZE_LIMIT_BYTES: u64 = 1_048_576;

fn main() {
    // 1. Telemetry env vars (unchanged behavior — preserved so a key
    //    rotation forces the downstream `option_env!` to be re-resolved).
    println!("cargo:rerun-if-env-changed=POSTHOG_API_KEY");
    println!("cargo:rerun-if-env-changed=POSTHOG_HOST");
    println!("cargo:rerun-if-env-changed=PANEFLOW_SKIP_EMBED_BUILD");

    // 2. US-008 — stage the AI-hook binaries into a dir that
    //    `assets::Bins` (rust-embed) will ingest.
    let target = std::env::var("TARGET").expect("cargo always sets TARGET for build scripts");
    // Expose the triple to source code via `env!("PANEFLOW_TARGET_TRIPLE")`
    // so `ai_hooks::extract` can locate the correct sub-folder under
    // `bin/<triple>/` at runtime without re-deriving it from `std::env::consts`.
    println!("cargo:rustc-env=PANEFLOW_TARGET_TRIPLE={target}");

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .expect("cargo always sets CARGO_MANIFEST_DIR for build scripts"),
    );
    let workspace_root = manifest_dir
        .parent()
        .expect("src-app manifest dir has a parent (the workspace root)")
        .to_path_buf();

    // The folder `RustEmbed` points at, relative to CARGO_MANIFEST_DIR.
    // Keep the in-memory/on-disk folder layout aligned with the macro.
    let embed_root = manifest_dir.join("target").join("embed").join("bin");
    let embed_dir = embed_root.join(&target);
    fs::create_dir_all(&embed_dir).unwrap_or_else(|e| {
        panic!(
            "US-008: cannot create embed staging dir {}: {e}",
            embed_dir.display()
        )
    });

    // Rerun when the shim / hook crate sources change. Cargo watches
    // directories recursively when a directory path is emitted.
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("crates/paneflow-shim").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("crates/paneflow-ai-hook").display()
    );
    // Also rerun if the root manifest changes (workspace-wide lint policy,
    // dep version bumps, etc., affect the staged binaries).
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("Cargo.toml").display()
    );

    let skip_nested_build = std::env::var_os("PANEFLOW_SKIP_EMBED_BUILD").is_some();
    if !skip_nested_build {
        stage_ai_hook_binaries(&workspace_root, &target, &embed_dir);
    } else {
        println!(
            "cargo:warning=PANEFLOW_SKIP_EMBED_BUILD is set — assuming {} is already populated",
            embed_dir.display()
        );
    }

    // Whether the nested build ran or not, enforce the size budget so a
    // pre-populated staging dir also honors the PRD cap.
    enforce_embed_size_budget(&embed_dir);
}

/// Invoke a child `cargo build` against the workspace to produce the
/// `paneflow-shim` and `paneflow-ai-hook` binaries for `target`, then
/// copy them into `embed_dir`. Panics (fails the outer build) on any
/// non-success exit, non-existent artifact, or IO error.
fn stage_ai_hook_binaries(workspace_root: &Path, target: &str, embed_dir: &Path) {
    // Use a dedicated `--target-dir` so we do not fight the outer cargo
    // for `target/debug/.cargo-lock` or `target/release/.cargo-lock`.
    // `embed-build` is a sibling of the outer `target/<profile>/` tree.
    let nested_target_dir = workspace_root.join("target").join("embed-build");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let profile = "release-min";

    // Run the nested cargo from the workspace root so `-p <crate>` is
    // resolved unambiguously and the workspace's `[patch.crates-io]`
    // block is honored.
    let mut cmd = Command::new(&cargo);
    cmd.current_dir(workspace_root)
        .arg("build")
        .arg("--profile")
        .arg(profile)
        .arg("--target")
        .arg(target)
        .arg("--target-dir")
        .arg(&nested_target_dir)
        .arg("-p")
        .arg("paneflow-shim")
        .arg("-p")
        .arg("paneflow-ai-hook")
        // Prevent the nested cargo from inheriting the outer cargo's
        // target-dir via `CARGO_TARGET_DIR` — the explicit `--target-dir`
        // above already pins it, but removing the env avoids confusion if
        // the parent environment sets it.
        .env_remove("CARGO_TARGET_DIR")
        // `RUSTFLAGS` changes (e.g. `-C link-arg=...` from sccache setups)
        // would invalidate the nested cache on every outer build. Leave
        // them alone; Cargo deals with that via its own fingerprinting.
        ;

    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("US-008: failed to spawn nested cargo build: {e}"));
    if !status.success() {
        panic!(
            "US-008: nested `cargo build --profile {profile} -p paneflow-shim -p paneflow-ai-hook --target {target}` \
             failed with {status}. Re-run the outer build with verbose logging to see the child cargo output."
        );
    }

    // Cargo lays artifacts out at
    // `<target-dir>/<triple>/<profile-dir>/<binary>[.exe]`.
    // For custom profiles the `<profile-dir>` equals the profile name
    // (release-min → release-min).
    let artifact_dir = nested_target_dir.join(target).join(profile);

    let bin_exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };

    // Copy only the two binaries we need; anything else in
    // `artifact_dir` is a transitive build product we don't want to embed.
    for bin in ["paneflow-shim", "paneflow-ai-hook"] {
        let src = artifact_dir.join(format!("{bin}{bin_exe}"));
        let dst = embed_dir.join(format!("{bin}{bin_exe}"));

        if !src.exists() {
            panic!(
                "US-008: expected nested build artifact {} is missing — \
                 did the child cargo build silently skip this binary?",
                src.display()
            );
        }
        // `fs::copy` preserves mode on Unix; embedded bytes don't need
        // the executable bit (the extractor sets it), but a 0o755 here
        // keeps `ls -l target/embed/bin/<triple>/` self-documenting.
        fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!(
                "US-008: copy {} → {} failed: {e}",
                src.display(),
                dst.display()
            )
        });
    }
}

/// Enforce the ≤ 1 MB total embedded-bytes cap defined by the PRD.
/// Inspects only top-level files in `embed_dir` — there are no subdirs
/// in the per-target staging layout so a recursive walk is not warranted.
fn enforce_embed_size_budget(embed_dir: &Path) {
    let mut total: u64 = 0;
    let mut per_file: BTreeMap<String, u64> = BTreeMap::new();
    let iter = match fs::read_dir(embed_dir) {
        Ok(iter) => iter,
        Err(e) => panic!(
            "US-008: cannot read embed staging dir {}: {e}",
            embed_dir.display()
        ),
    };
    for entry in iter {
        let entry = entry
            .unwrap_or_else(|e| panic!("US-008: broken embed dir entry in {embed_dir:?}: {e}"));
        let metadata = entry
            .metadata()
            .unwrap_or_else(|e| panic!("US-008: cannot stat {}: {e}", entry.path().display()));
        if metadata.is_file() {
            let size = metadata.len();
            total = total.saturating_add(size);
            per_file.insert(entry.file_name().to_string_lossy().into_owned(), size);
        }
    }

    if total > EMBED_SIZE_LIMIT_BYTES {
        let mut details = String::new();
        for (name, size) in &per_file {
            details.push_str(&format!("  {name}: {size} bytes\n"));
        }
        panic!(
            "US-008: embedded AI-hook binaries exceed the 1 MB cap ({total} > {EMBED_SIZE_LIMIT_BYTES} bytes).\n\
             Staging dir: {}\n\
             Per-file:\n{details}\
             Shrink the shim/ai-hook via smaller deps or tighter release-min profile.",
            embed_dir.display()
        );
    }
}
