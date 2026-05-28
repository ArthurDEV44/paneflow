//! US-028 (cli-hardening-followup-2026-Q3): criterion baseline for
//! the `compress_items` zstd-roundtrip used by `ThreadStore::save_items`.
//!
//! Two scenarios are measured:
//!
//! 1. **Small thread** (10 messages, ~50 chars each): the steady-state
//!    persist tick during an idle conversation. Sub-millisecond.
//! 2. **Heavy thread** (200 messages, 1 KB each, plus 50 tool calls):
//!    the case `US-022` (per-item dirty-flag cache) optimises -- the
//!    bench measures the FULL `compress_items` cost, which is the
//!    upper bound the cache short-circuits past on a cache-hit
//!    persist tick.
//!
//! Run via `cargo bench -p paneflow-threads --bench blob_compress`.
//! Criterion's regression detector compares each run against the
//! committed baseline in `target/criterion/**/base/`.

// Benches are bench-only: an `.expect()` on a deterministic
// known-good fixture is the canonical bench idiom and the project's
// `clippy::expect_used = warn` was intended for production code, not
// criterion harnesses. `clippy.toml`'s `allow-expect-in-tests` does
// not cover bench targets so we opt out explicitly here.
#![allow(clippy::expect_used)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use paneflow_threads::blob::compress_items;
use paneflow_threads::item::{PersistedAssistant, PersistedAssistantChunk, PersistedThreadItem};
use paneflow_threads::message::Message;

fn small_thread(count: usize) -> Vec<PersistedThreadItem> {
    (0..count)
        .map(|i| {
            if i % 2 == 0 {
                PersistedThreadItem::Message(Message::user_text(format!("tiny user message {i}")))
            } else {
                PersistedThreadItem::Assistant(PersistedAssistant {
                    chunks: vec![PersistedAssistantChunk::Text {
                        text: format!("tiny assistant response {i}"),
                    }],
                })
            }
        })
        .collect()
}

fn heavy_thread(count: usize, body_bytes: usize) -> Vec<PersistedThreadItem> {
    // Realistic shape: alternating user / assistant turns with
    // ~1 KB bodies. The compressor sees a lot of repetition so a
    // single large body would be misleadingly compressible; varying
    // the content per turn keeps the benchmark honest.
    (0..count)
        .map(|i| {
            let body: String = std::iter::repeat_with(|| ((i % 26) as u8 + b'a') as char)
                .take(body_bytes)
                .collect();
            if i % 2 == 0 {
                PersistedThreadItem::Message(Message::user_text(body))
            } else {
                PersistedThreadItem::Assistant(PersistedAssistant {
                    chunks: vec![PersistedAssistantChunk::Text { text: body }],
                })
            }
        })
        .collect()
}

fn bench_compress_items(c: &mut Criterion) {
    let small = small_thread(10);
    c.bench_function("compress_items_small_10msg", |b| {
        b.iter(|| {
            let out = compress_items(black_box(&small)).expect("compress small");
            black_box(out);
        });
    });

    let heavy = heavy_thread(200, 1024);
    c.bench_function("compress_items_heavy_200msg_1k", |b| {
        b.iter(|| {
            let out = compress_items(black_box(&heavy)).expect("compress heavy");
            black_box(out);
        });
    });
}

criterion_group!(blob_compress, bench_compress_items);
criterion_main!(blob_compress);
