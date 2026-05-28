//! US-001 (prd-markdown-append-fix-2026-Q3): synthetic proxy for the
//! Zed Markdown::append O(n^2) hot path used by Paneflow streaming.
//!
//! Run via `cargo bench -p paneflow-threads --bench markdown_append`.
//! Criterion writes the comparable wall-clock numbers under
//! `target/criterion/markdown_append/`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use gpui_shared_string::SharedString;

const CHUNKS: usize = 100;
const CHUNK_BYTES: usize = 600;
const TOTAL_BYTES: usize = CHUNKS * CHUNK_BYTES;

fn make_chunks() -> Vec<String> {
    const FIXTURE_BYTES: &[u8] =
        b"paneflow markdown append streaming fixture with code blocks and prose\n";

    (0..CHUNKS)
        .map(|chunk_ix| {
            (0..CHUNK_BYTES)
                .map(|byte_ix| FIXTURE_BYTES[(chunk_ix + byte_ix) % FIXTURE_BYTES.len()] as char)
                .collect()
        })
        .collect()
}

fn shared_string_concat_proxy(chunks: &[String]) -> SharedString {
    let mut source = SharedString::new("");

    for chunk in chunks {
        source = SharedString::new(source.to_string() + black_box(chunk.as_str()));
    }

    source
}

fn string_push_str_proxy(chunks: &[String]) -> String {
    let mut source = String::with_capacity(TOTAL_BYTES);

    for chunk in chunks {
        source.push_str(black_box(chunk.as_str()));
    }

    source
}

fn bench_markdown_append(c: &mut Criterion) {
    let chunks = make_chunks();
    let mut group = c.benchmark_group("markdown_append");
    group.throughput(Throughput::Bytes(TOTAL_BYTES as u64));

    group.bench_function(
        BenchmarkId::new("shared_string_concat_proxy", "60kb_100x600b"),
        |b| {
            b.iter(|| {
                let source = shared_string_concat_proxy(black_box(&chunks));
                black_box(source);
            });
        },
    );

    group.bench_function(
        BenchmarkId::new("string_push_str_proxy", "60kb_100x600b"),
        |b| {
            b.iter(|| {
                let source = string_push_str_proxy(black_box(&chunks));
                black_box(source);
            });
        },
    );

    group.finish();
}

criterion_group!(markdown_append, bench_markdown_append);
criterion_main!(markdown_append);
