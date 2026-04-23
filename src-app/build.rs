//! Minimal build script whose sole purpose is to tell Cargo to rebuild
//! the crate when telemetry-related compile-time env vars change.
//!
//! `option_env!("POSTHOG_API_KEY")` and `option_env!("POSTHOG_HOST")`
//! are resolved at compile time (see `src-app/src/app/bootstrap.rs`).
//! Without these `rerun-if-env-changed` directives Cargo has no way to
//! know the macro output depends on those vars, so rotating the key or
//! host in CI would produce a binary that still embeds the previous
//! value until an unrelated source change forces a rebuild.

fn main() {
    println!("cargo:rerun-if-env-changed=POSTHOG_API_KEY");
    println!("cargo:rerun-if-env-changed=POSTHOG_HOST");
}
