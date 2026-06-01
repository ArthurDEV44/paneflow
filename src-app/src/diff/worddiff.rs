//! Word-level intra-line diff (US-010, prd-multi-worktree-diff-2026-Q3.md).
//!
//! A from-scratch port of Zed's `language::word_diff_ranges`
//! (`crates/language/src/text_diff.rs:185`), reduced to use only `imara-diff`.
//! Zed's tokenizer is language-aware (`CharClassifier` from the `language`
//! crate); this uses a simpler word/non-word run tokenizer so the diff module
//! keeps its zero-Zed-crate-dependency property. Returns the byte ranges of the
//! changed words in each side, which the renderer paints as stronger
//! highlights over the line background.

use std::ops::Range;

use imara_diff::intern::InternedInput;
use imara_diff::{Algorithm, Sink, diff};

/// Max line count of a modified hunk that still gets word-diff highlighting.
/// Zed caps at 5; raised to 20 here because multi-line refactors (function
/// renames, parameter changes spanning 10-20 lines) are exactly the hunks worth
/// diffing precisely, and the `imara-diff` Histogram pass is microseconds/line
/// off-thread, so there is no perceptible cost. Larger hunks fall back to
/// line-level highlighting only — no per-line word-diff cliff.
pub const MAX_WORD_DIFF_LINE_COUNT: u32 = 20;

/// Split `text` into maximal runs of word chars (`alphanumeric` / `_`) vs
/// non-word chars. Contiguous slices of the original, so prefix byte offsets
/// recover positions.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut prev: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let is_word = c.is_alphanumeric() || c == '_';
        if let Some(p) = prev
            && p != is_word
        {
            tokens.push(&text[start..i]);
            start = i;
        }
        prev = Some(is_word);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Prefix byte offsets of `tokens` (length `tokens.len() + 1`); entry `i` is the
/// byte offset where token `i` begins, the last entry is the total length.
fn prefix_offsets(tokens: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(tokens.len() + 1);
    let mut acc = 0;
    offsets.push(0);
    for t in tokens {
        acc += t.len();
        offsets.push(acc);
    }
    offsets
}

fn push_merged(ranges: &mut Vec<Range<usize>>, r: Range<usize>) {
    if let Some(last) = ranges.last_mut()
        && last.end >= r.start
    {
        last.end = r.end;
    } else {
        ranges.push(r);
    }
}

struct WordSink<'a> {
    old_off: &'a [usize],
    new_off: &'a [usize],
    old_ranges: Vec<Range<usize>>,
    new_ranges: Vec<Range<usize>>,
}

impl Sink for WordSink<'_> {
    type Out = (Vec<Range<usize>>, Vec<Range<usize>>);

    fn process_change(&mut self, before: Range<u32>, after: Range<u32>) {
        if before.start != before.end {
            let r = self.old_off[before.start as usize]..self.old_off[before.end as usize];
            push_merged(&mut self.old_ranges, r);
        }
        if after.start != after.end {
            let r = self.new_off[after.start as usize]..self.new_off[after.end as usize];
            push_merged(&mut self.new_ranges, r);
        }
    }

    fn finish(self) -> Self::Out {
        (self.old_ranges, self.new_ranges)
    }
}

/// Compute `(old_ranges, new_ranges)` of changed words between two lines, as
/// byte ranges into `old_text` / `new_text` respectively. Adjacent changed
/// tokens are merged (mirroring Zed).
pub fn word_diff_ranges(old_text: &str, new_text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let old_tokens = tokenize(old_text);
    let new_tokens = tokenize(new_text);
    let old_off = prefix_offsets(&old_tokens);
    let new_off = prefix_offsets(&new_tokens);
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(old_tokens.iter().copied());
    input.update_after(new_tokens.iter().copied());
    diff(
        Algorithm::Histogram,
        &input,
        WordSink {
            old_off: &old_off,
            new_off: &new_off,
            old_ranges: Vec::new(),
            new_ranges: Vec::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_word_change() {
        let (old_r, new_r) = word_diff_ranges("let x = 1", "let y = 1");
        // Only the identifier changed.
        assert_eq!(old_r.len(), 1);
        assert_eq!(new_r.len(), 1);
        assert_eq!(&"let x = 1"[old_r[0].clone()], "x");
        assert_eq!(&"let y = 1"[new_r[0].clone()], "y");
    }

    #[test]
    fn multi_word_change() {
        let (old_r, new_r) = word_diff_ranges("foo bar baz", "foo QUX baz");
        assert_eq!(old_r.len(), 1);
        assert_eq!(&"foo bar baz"[old_r[0].clone()], "bar");
        assert_eq!(&"foo QUX baz"[new_r[0].clone()], "QUX");
    }

    #[test]
    fn identical_lines_no_ranges() {
        let (old_r, new_r) = word_diff_ranges("same line", "same line");
        assert!(old_r.is_empty());
        assert!(new_r.is_empty());
    }

    #[test]
    fn pure_insertion_only_new_side() {
        let (old_r, new_r) = word_diff_ranges("a c", "a b c");
        assert!(old_r.is_empty());
        assert_eq!(new_r.len(), 1);
    }
}
