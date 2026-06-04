//! Detached minisign (Ed25519) signature verification — the independent
//! root of trust for the self-updater (US-001, prd-audit-remediation-2026-Q3).
//!
//! ## Why this exists
//!
//! Before US-001 every installer (`linux/targz.rs`, `macos/dmg.rs`,
//! `windows/msi.rs`) downloaded a `.sha256` sibling *from the same host* as
//! the artifact and trusted it. That is not a trust anchor: a compromised
//! mirror or a MITM serves a malicious binary together with a matching
//! `.sha256` and the check passes. The downloaded code then runs as the
//! user — an RCE-class hole.
//!
//! minisign fixes this by binding the artifact to a key the running binary
//! already carries. The public key is baked in at **build time** (CI sets
//! `PANEFLOW_MINISIGN_PUBKEY`, see US-002); the matching secret key lives
//! only in a GitHub Encrypted Secret and never touches a release host. An
//! attacker who controls the download channel cannot forge a signature
//! without the secret key, so a tampered artifact fails verification and is
//! never extracted, mounted, or executed.
//!
//! ## Fail-closed contract
//!
//! Every exit path that is not "a signature made by an embedded key
//! verifies the exact bytes on disk" returns an [`UpdateError`] and aborts:
//!
//! - No public key embedded in this build (dev / unsigned build).
//! - No `.minisig` published next to the asset (404 or network error).
//! - Signature malformed, key-id unknown, or the bytes don't verify.
//! - Legacy (non-prehashed) signature — we only accept the modern format.
//!
//! There is no silent skip and no `.sha256` fallback.
//!
//! ## Dual-key rotation
//!
//! Two slots are embedded — `PANEFLOW_MINISIGN_PUBKEY` (current) and
//! `PANEFLOW_MINISIGN_PUBKEY_NEXT` (next). Verification accepts a signature
//! made by *either* key. That lets a key be rotated without an online
//! revocation step: ship the next key in a release, switch CI signing to
//! it, then retire the old key after a few releases (see
//! `docs/self-update-signing.md`, US-002).
//!
//! ## Cross-platform
//!
//! Pure Rust (`minisign-verify` has no C deps) and no platform-specific
//! syscalls — the module compiles and runs identically on Linux, macOS and
//! Windows. The OS-native belt (`codesign`/`spctl`, `WinVerifyTrust`) is a
//! second, independent layer added per-platform in US-004 / US-005.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use minisign_verify::{PublicKey, Signature};

use super::error::IntegrityMismatch;

/// Upper bound on the `.minisig` fetch. The signature file is a few hundred
/// bytes; cap well above that so a hostile mirror can't stream an unbounded
/// body into memory while we wait for a "signature".
const MAX_SIG_BYTES: u64 = 64 * 1024;

/// Upper bound on the signature HTTP fetch. Mirrors the 30 s per-call budget
/// the rest of the update flow uses (US-001).
const SIG_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Current minisign public key, embedded at build time by CI (US-002). This
/// is the **base64 line** of a `minisign.pub` file (the second line, not the
/// untrusted-comment header). `None` in any build that did not set the env
/// var — dev builds, and any release cut before US-002 wired CI signing.
const EMBEDDED_PUBKEY_CURRENT: Option<&str> = option_env!("PANEFLOW_MINISIGN_PUBKEY");

/// Next minisign public key for rotation (US-002). A signature made by this
/// key also verifies, so a release can switch the signing key without
/// bricking clients that still carry only the previous key. Usually `None`
/// outside a rotation window.
const EMBEDDED_PUBKEY_NEXT: Option<&str> = option_env!("PANEFLOW_MINISIGN_PUBKEY_NEXT");

/// Parse the embedded base64 public keys into verifier keys.
///
/// A malformed slot is logged and skipped rather than aborting — one bad
/// env var must not disable a second, good key. The returned set is empty
/// only when this build embeds no usable key, in which case every caller
/// fails closed.
fn embedded_public_keys() -> Vec<PublicKey> {
    [
        ("PANEFLOW_MINISIGN_PUBKEY", EMBEDDED_PUBKEY_CURRENT),
        ("PANEFLOW_MINISIGN_PUBKEY_NEXT", EMBEDDED_PUBKEY_NEXT),
    ]
    .into_iter()
    .filter_map(|(name, slot)| {
        let b64 = slot?.trim();
        if b64.is_empty() {
            return None;
        }
        match PublicKey::from_base64(b64) {
            Ok(pk) => Some(pk),
            Err(e) => {
                log::error!(
                    "self-update/signature: embedded {name} is not a valid minisign key: {e}"
                );
                None
            }
        }
    })
    .collect()
}

/// True when this build carries at least one usable signing key — i.e. the
/// in-app updater has a trust anchor and can verify downloads. Callers can
/// use this to short-circuit the whole update flow on unsigned builds with
/// a clear message instead of downloading an artifact they can never trust.
pub(crate) fn has_embedded_key() -> bool {
    !embedded_public_keys().is_empty()
}

/// Build the fail-closed tagged error for a verification failure. Uses the
/// [`IntegrityMismatch`] tag so the main-thread classifier routes it to the
/// frozen "corrupt or tampered" toast (US-013) — exactly the right message
/// whether the bytes were tampered, the signature is missing, or this build
/// has no key.
fn reject(reason: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(IntegrityMismatch {
        expected: "valid minisign signature".to_string(),
        got: reason.into(),
    })
}

/// Verify `artifact` against the detached signature text `sig_text` using an
/// explicit key set. Streaming: the artifact is hashed incrementally so a
/// 500 MB download is never buffered in memory.
///
/// Returns `Ok(())` iff some key in `keys` produced a valid, prehashed
/// signature over the exact bytes on disk. Every other outcome is a
/// fail-closed [`IntegrityMismatch`] error.
///
/// Split out from [`verify_detached_file`] so unit tests can inject an
/// ephemeral key generated by the `minisign` dev-dependency.
fn verify_with_keys(artifact: &Path, sig_text: &str, keys: &[PublicKey]) -> Result<()> {
    if keys.is_empty() {
        return Err(reject(
            "no verification key embedded in this build — refusing to install an unverifiable update",
        ));
    }

    let signature =
        Signature::decode(sig_text).map_err(|e| reject(format!("signature is malformed: {e}")))?;

    // At most one embedded key shares the signature's key-id; the rest fail
    // `verify_stream`'s key-id check *before* any file I/O, so we hash the
    // artifact at most once. `verify_stream` also rejects legacy
    // (non-prehashed) signatures up front, which keeps us on the modern
    // format only.
    let mut key_id_matched = false;
    for key in keys {
        let mut verifier = match key.verify_stream(&signature) {
            Ok(v) => v,
            // Wrong key-id or legacy format for this candidate — try the next
            // embedded key without touching the file.
            Err(_) => continue,
        };
        key_id_matched = true;

        let mut file = std::fs::File::open(artifact)
            .with_context(|| format!("open {} for signature check", artifact.display()))?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .context("read artifact chunk for signature check")?;
            if n == 0 {
                break;
            }
            verifier.update(&buf[..n]);
        }
        if verifier.finalize().is_ok() {
            return Ok(());
        }
        // key-id matched but the bytes don't verify → tampered. Keep trying
        // remaining keys (defensive; normally there is only one match) and
        // fall through to the fail-closed error below.
    }

    if key_id_matched {
        Err(reject(
            "artifact does not match its signature — corrupt or tampered",
        ))
    } else {
        Err(reject(
            "signature was not made by any key trusted by this build",
        ))
    }
}

/// Verify `artifact` against the detached `sig_text` using the keys embedded
/// in this build. Fail-closed: see the module docs.
pub(crate) fn verify_detached_file(artifact: &Path, sig_text: &str) -> Result<()> {
    verify_with_keys(artifact, sig_text, &embedded_public_keys())
}

/// Download the detached `.minisig` for `asset_url` (the sibling
/// `<asset_url>.minisig`) and verify `artifact` against it. This is the
/// single entry point each installer calls **before** any
/// extraction / mount / exec.
///
/// A 404 (no signature published) or any network failure is a hard abort —
/// the unhappy path mandated by US-001: we never install an unsigned
/// artifact.
pub(crate) fn fetch_and_verify(artifact: &Path, asset_url: &str) -> Result<()> {
    // Fail fast on unsigned builds before spending bandwidth on a `.minisig`
    // we could never check.
    if !has_embedded_key() {
        return Err(reject(
            "no verification key embedded in this build — refusing to install an unverifiable update",
        ));
    }

    let sig_url = format!("{asset_url}.minisig");
    let sig_text = fetch_signature_text(&sig_url)?;
    verify_detached_file(artifact, &sig_text)
        .with_context(|| format!("verify minisign signature of {}", artifact.display()))
}

/// Fetch the `.minisig` body as a string, bounded and timeout-capped. A 404
/// surfaces a distinct, actionable message (the release predates signing, or
/// the signature was not uploaded) rather than a generic network error.
fn fetch_signature_text(sig_url: &str) -> Result<String> {
    let mut response = ureq::get(sig_url)
        .config()
        .timeout_global(Some(SIG_HTTP_TIMEOUT))
        .build()
        .header(
            "User-Agent",
            &format!("paneflow/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| "Could not fetch update signature. Try again when online.".to_string())?;

    let status = response.status();
    if !status.is_success() {
        if status.as_u16() == 404 {
            return Err(reject(
                "this release is not signed (no .minisig published) — download the latest version from the releases page",
            ));
        }
        return Err(reject(format!(
            "could not fetch update signature (HTTP {status})"
        )));
    }

    let reader = response.body_mut().as_reader();
    let mut bounded = Read::take(reader, MAX_SIG_BYTES + 1);
    let mut text = String::new();
    let read = bounded
        .read_to_string(&mut text)
        .context("read .minisig body")?;
    if read as u64 > MAX_SIG_BYTES {
        return Err(reject("update signature is implausibly large — aborting"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Generate an ephemeral minisign keypair with the `minisign`
    /// dev-dependency and return `(verify_public_key, base64_line)`.
    fn gen_keypair() -> (PublicKey, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        // `to_box().into_string()` is the 2-line `.pub` file; the base64 the
        // verifier wants is the second line.
        let pub_text = kp.pk.to_box().unwrap().into_string();
        let b64 = pub_text.lines().nth(1).unwrap().to_string();
        let vk = PublicKey::from_base64(&b64).unwrap();
        (vk, b64)
    }

    /// Sign `data` with a freshly generated keypair; return
    /// `(verify_key, detached_sig_text)`.
    fn sign(data: &[u8]) -> (PublicKey, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pub_text = kp.pk.to_box().unwrap().into_string();
        let b64 = pub_text.lines().nth(1).unwrap().to_string();
        let vk = PublicKey::from_base64(&b64).unwrap();
        let sig_box = minisign::sign(Some(&kp.pk), &kp.sk, Cursor::new(data), None, None).unwrap();
        (vk, sig_box.into_string())
    }

    fn write_tmp(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("artifact.bin");
        std::fs::write(&p, bytes).unwrap();
        (dir, p)
    }

    #[test]
    fn verifies_a_correctly_signed_artifact() {
        let data = b"paneflow-0.3.9-x86_64.tar.gz payload";
        let (vk, sig) = sign(data);
        let (_d, path) = write_tmp(data);
        assert!(verify_with_keys(&path, &sig, &[vk]).is_ok());
    }

    #[test]
    fn rejects_a_tampered_artifact() {
        // Sign the real bytes, then verify DIFFERENT bytes on disk — the
        // exact MITM scenario US-001 closes. Must fail before any caller
        // would extract/exec.
        let data = b"the genuine release payload";
        let (vk, sig) = sign(data);
        let (_d, path) = write_tmp(b"a malicious replacement payload");
        let err = verify_with_keys(&path, &sig, &[vk]).unwrap_err();
        assert!(
            err.downcast_ref::<IntegrityMismatch>().is_some(),
            "tampered artifact must produce an IntegrityMismatch tag, got: {err:#}"
        );
        // And it classifies into the "corrupt or tampered" toast.
        assert!(matches!(
            super::super::error::UpdateError::classify(&err),
            super::super::error::UpdateError::IntegrityMismatch { .. }
        ));
    }

    #[test]
    fn rejects_signature_from_an_untrusted_key() {
        // Signed by key A, but the build only trusts key B → fail closed,
        // even though the signature is internally valid.
        let data = b"payload signed by a key we do not trust";
        let (_vk_a, sig) = sign(data);
        let (vk_b, _b64) = gen_keypair();
        let (_d, path) = write_tmp(data);
        let err = verify_with_keys(&path, &sig, &[vk_b]).unwrap_err();
        assert!(err.downcast_ref::<IntegrityMismatch>().is_some());
    }

    #[test]
    fn fails_closed_when_no_key_is_embedded() {
        let data = b"payload";
        let (_vk, sig) = sign(data);
        let (_d, path) = write_tmp(data);
        let err = verify_with_keys(&path, &sig, &[]).unwrap_err();
        assert!(
            err.to_string().contains("no verification key"),
            "got: {err:#}"
        );
    }

    #[test]
    fn rejects_a_malformed_signature() {
        let (_d, path) = write_tmp(b"payload");
        let (vk, _b64) = gen_keypair();
        let err = verify_with_keys(&path, "not a minisig at all", &[vk]).unwrap_err();
        assert!(err.downcast_ref::<IntegrityMismatch>().is_some());
    }

    #[test]
    fn dual_key_accepts_either_slot() {
        // Rotation window: the artifact is signed by the NEXT key while the
        // build still carries CURRENT + NEXT. Verification must accept it.
        let data = b"signed by the rotation key";
        let kp_current = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let b64_current = kp_current
            .pk
            .to_box()
            .unwrap()
            .into_string()
            .lines()
            .nth(1)
            .unwrap()
            .to_string();
        let vk_current = PublicKey::from_base64(&b64_current).unwrap();

        let (vk_next, sig) = sign(data);
        let (_d, path) = write_tmp(data);

        // Order shouldn't matter: current first, next second.
        assert!(verify_with_keys(&path, &sig, &[vk_current, vk_next]).is_ok());
    }

    #[test]
    fn empty_embedded_slots_yield_no_keys() {
        // The const slots are unset in a normal `cargo test` build, so the
        // real embedded set is empty and `has_embedded_key()` is false —
        // proving dev/test builds fail closed by construction.
        assert!(!has_embedded_key());
        assert!(embedded_public_keys().is_empty());
    }
}
