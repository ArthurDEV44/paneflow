//! JSON + zstd codec for the per-thread timeline blob.
//!
//! Format on disk (the `data` column):
//!
//! ```text
//! zstd_decompress(blob)
//!   == serde_json::to_vec(&BlobV1 {
//!         schema_version: 1,
//!         items: [...],     // primary timeline (since 2026-05)
//!         messages: [...],  // legacy field, ignored when `items` is non-empty
//!     })
//! ```
//!
//! Both `items` and `messages` are `#[serde(default)]` so blobs written
//! by any historical Paneflow build (pre-`items`, pre-`messages`) still
//! round-trip cleanly. On read, when `items` is empty AND `messages` is
//! non-empty, the loader migrates each message into
//! `PersistedThreadItem::Message`. New writes always populate `items`
//! and leave `messages` empty.

use crate::item::PersistedThreadItem;
use crate::message::Message;
use serde::{Deserialize, Serialize};

/// Current on-disk blob schema version. Bump when adding a non-additive
/// field to [`Message`] / [`PersistedThreadItem`] or rewriting the
/// wrapper shape.
pub const CURRENT_BLOB_VERSION: u32 = 1;

/// zstd compression level used for `compress_*`. Level 3 is the
/// zstd default and the PRD-specified value (US-006 AC). It typically
/// achieves >5x on chat text while keeping per-message write latency
/// well under the 50 ms budget (~ a few hundred microseconds for blobs
/// under 100 KB on a 2024-class CPU).
pub const COMPRESSION_LEVEL: i32 = 3;

/// Defence-in-depth cap on the decompressed JSON size. The PRD soft-caps
/// a thread at 10 MB compressed (~50 MB plaintext at the typical 5x
/// ratio); 256 MB leaves generous headroom while preventing a corrupted
/// or adversarial row from triggering unbounded allocation via a
/// zstd "decompression bomb" frame header.
pub const MAX_DECOMPRESSED_BYTES: usize = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct BlobV1 {
    #[serde(default = "default_blob_version")]
    schema_version: u32,
    /// Primary timeline. Empty in blobs written by Paneflow builds that
    /// predate the persisted-tool-calls refactor (2026-05).
    #[serde(default)]
    items: Vec<PersistedThreadItem>,
    /// Legacy timeline (messages only). Populated by old writes, kept
    /// for read-only backward compat. Always empty in new writes.
    #[serde(default)]
    messages: Vec<Message>,
}

fn default_blob_version() -> u32 {
    CURRENT_BLOB_VERSION
}

/// Serialise `messages` to JSON and compress with zstd. Wraps each
/// message into `PersistedThreadItem::Message` under the hood so the
/// on-disk shape is always the modern `items` field.
pub fn compress_messages(messages: &[Message]) -> Result<Vec<u8>, std::io::Error> {
    let items: Vec<PersistedThreadItem> = messages
        .iter()
        .cloned()
        .map(PersistedThreadItem::Message)
        .collect();
    compress_items(&items)
}

/// Serialise a full `Vec<PersistedThreadItem>` (messages + tool calls +
/// reasoning cards) to JSON and compress with zstd. This is the
/// primary persistence entry point as of 2026-05; `compress_messages`
/// remains as a convenience for callers that only need to write
/// messages.
pub fn compress_items(items: &[PersistedThreadItem]) -> Result<Vec<u8>, std::io::Error> {
    let blob = BlobV1 {
        schema_version: CURRENT_BLOB_VERSION,
        items: items.to_vec(),
        messages: Vec::new(),
    };
    let json = serde_json::to_vec(&blob).map_err(std::io::Error::other)?;
    zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)
}

/// Decompress a blob and return ONLY the `Message` entries. Used by
/// callers (sidebar summary derivation, export-markdown) that don't
/// care about tool calls or reasoning. Internally routes through
/// [`decompress_items`] so old `messages`-only blobs migrate
/// transparently.
pub fn decompress_messages(blob: &[u8]) -> Result<Vec<Message>, std::io::Error> {
    let items = decompress_items(blob)?;
    Ok(items
        .into_iter()
        .filter_map(|item| match item {
            PersistedThreadItem::Message(m) => Some(m),
            _ => None,
        })
        .collect())
}

/// Decompress a blob and return the full timeline. Handles all three
/// historical shapes:
/// - new (>=2026-05) blob: returns `items` directly.
/// - legacy (`messages` only) blob: wraps each message into
///   `PersistedThreadItem::Message`.
/// - empty bytes: returns an empty Vec.
///
/// The decompressed JSON is capped at [`MAX_DECOMPRESSED_BYTES`] to
/// prevent a corrupted or adversarially-crafted zstd frame from
/// triggering unbounded allocation. Beyond the cap, decompression
/// aborts with an `InvalidData` error rather than OOM-killing the
/// process.
pub fn decompress_items(blob: &[u8]) -> Result<Vec<PersistedThreadItem>, std::io::Error> {
    use std::io::{Error, ErrorKind, Read};
    if blob.is_empty() {
        return Ok(Vec::new());
    }
    let mut decoder = zstd::Decoder::new(blob)?;
    let mut json = Vec::with_capacity(blob.len().min(MAX_DECOMPRESSED_BYTES));
    let limit = (MAX_DECOMPRESSED_BYTES as u64).saturating_add(1);
    let read = std::io::copy(&mut decoder.by_ref().take(limit), &mut json)?;
    if read as usize > MAX_DECOMPRESSED_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "decompressed payload exceeds cap ({} > {} bytes)",
                read, MAX_DECOMPRESSED_BYTES
            ),
        ));
    }
    let parsed: BlobV1 = serde_json::from_slice(&json).map_err(Error::other)?;
    if !parsed.items.is_empty() {
        return Ok(parsed.items);
    }
    Ok(parsed
        .messages
        .into_iter()
        .map(PersistedThreadItem::Message)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_round_trip() {
        let payload = compress_messages(&[]).unwrap();
        assert!(!payload.is_empty(), "compressed empty blob is still framed");
        let back = decompress_messages(&payload).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn empty_bytes_decode_to_empty_vec() {
        let back = decompress_messages(&[]).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn small_blob_round_trip() {
        let messages = vec![
            Message::user_text("hello"),
            Message::assistant_text("hi there"),
            Message::user_text("how are you"),
        ];
        let payload = compress_messages(&messages).unwrap();
        let back = decompress_messages(&payload).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].role, messages[0].role);
        assert_eq!(back[2].role, messages[2].role);
    }

    #[test]
    fn large_blob_compresses_more_than_half() {
        // The PRD's >50% target is a regression guard: zstd level 3 on
        // repetitive English chat text should hit ~75-95% reduction.
        // We use lorem-ipsum style filler to mimic prose entropy.
        let phrase = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let big = phrase.repeat(20_000); // ~1.1 MB plaintext
        let messages = vec![Message::assistant_text(big.clone())];
        let raw_len = big.len();
        let payload = compress_messages(&messages).unwrap();
        let ratio = payload.len() as f64 / raw_len as f64;
        assert!(
            ratio < 0.5,
            "expected >50% compression, got ratio {ratio:.3} \
             (raw {raw_len}, compressed {})",
            payload.len()
        );
        let back = decompress_messages(&payload).unwrap();
        assert_eq!(back.len(), 1);
    }

    #[test]
    fn corrupt_payload_is_an_error_not_a_panic() {
        let mut payload = compress_messages(&[Message::user_text("hi")]).unwrap();
        // Flip a byte mid-payload to break the zstd frame.
        let mid = payload.len() / 2;
        payload[mid] = payload[mid].wrapping_add(1);
        let res = decompress_messages(&payload);
        assert!(res.is_err(), "must error, not panic, on corrupted bytes");
    }
}
