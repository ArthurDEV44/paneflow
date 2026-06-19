//! Pure diff-to-text serialization (US-001, prd-ai-in-diff-2026-Q3.md).
//!
//! Turns the read-only diff value types (`FileDiff` / `DiffHunk` from `git.rs` /
//! `engine.rs`) back into a byte-correct unified diff string, with no GPUI, no
//! I/O, no `self`. This is the load-bearing primitive under every "copy hunk",
//! "send to agent", and "review" gesture (EP-001..003): by generating the diff
//! deterministically here, the agent never sees a hallucinated `@@` header -
//! the #1 failure mode of LLM-*generated* patches is designed out because the
//! model only ever *consumes* a diff we produced.
//!
//! Row indices in `DiffHunk` are 0-based half-open ranges into the line
//! sequence produced by `imara_diff::sources::lines_with_terminator` (see
//! `engine::compute_hunks`); [`lines_inclusive`] reproduces exactly that
//! segmentation so the `@@` math stays aligned with the rendered rows.

use std::fmt::Write as _;
use std::ops::Range;

use super::engine::DiffHunk;
use super::git::{FileChange, FileDiff};

/// Context lines emitted on each side of a changed region, matching the
/// `diff -U3` / git default. Hunks whose context windows touch are merged into a
/// single `@@` block so the output is always a valid unified diff.
const CONTEXT: u32 = 3;

/// Serialize a whole [`FileDiff`] into a git-style unified diff (raw, no fence).
/// Suitable for "copy file diff" and for an agent review payload.
pub(crate) fn file_to_unified(file: &FileDiff) -> String {
    let mut out = String::new();
    let old_disp = if file.change == FileChange::Renamed {
        file.old_path.as_deref().unwrap_or(&file.path)
    } else {
        &file.path
    };
    let _ = writeln!(out, "diff --git a/{old_disp} b/{}", file.path);

    if file.change == FileChange::Renamed
        && let Some(old) = &file.old_path
    {
        let _ = writeln!(out, "rename from {old}");
        let _ = writeln!(out, "rename to {}", file.path);
    }

    if file.is_binary {
        let _ = writeln!(out, "Binary files a/{old_disp} b/{} differ", file.path);
        return out;
    }

    let (a_label, b_label) = dev_null_labels(file);
    let _ = writeln!(out, "--- {a_label}");
    let _ = writeln!(out, "+++ {b_label}");

    let base_lines = lines_inclusive(&file.base_text);
    let new_lines = lines_inclusive(&file.new_text);
    for group in group_hunks(&file.hunks, base_lines.len() as u32, new_lines.len() as u32) {
        emit_group(&mut out, &group, &base_lines, &new_lines);
    }
    out
}

/// Serialize a single [`DiffHunk`] into a fenced ```` ```diff ```` block,
/// prefixed by a `path:Lstart-Lend` tag so an agent knows exactly which lines
/// the change touches. Suitable for "copy hunk" and `@diff`-style handoff.
pub(crate) fn hunk_to_unified(file: &FileDiff, hunk: &DiffHunk) -> String {
    let tag = hunk_tag(file, hunk);
    if file.is_binary {
        return format!(
            "{tag}\n```diff\nBinary files a/{p} b/{p} differ\n```\n",
            p = file.path
        );
    }
    let base_lines = lines_inclusive(&file.base_text);
    let new_lines = lines_inclusive(&file.new_text);
    let mut body = String::new();
    for group in group_hunks(
        std::slice::from_ref(hunk),
        base_lines.len() as u32,
        new_lines.len() as u32,
    ) {
        emit_group(&mut body, &group, &base_lines, &new_lines);
    }
    format!("{tag}\n```diff\n{body}```\n")
}

/// `--- a/x` / `+++ b/x` labels, substituting `/dev/null` for the absent side of
/// an Added or Deleted file (git's convention).
fn dev_null_labels(file: &FileDiff) -> (String, String) {
    match file.change {
        FileChange::Added => ("/dev/null".to_string(), format!("b/{}", file.path)),
        FileChange::Deleted => (format!("a/{}", file.path), "/dev/null".to_string()),
        FileChange::Renamed => (
            format!("a/{}", file.old_path.as_deref().unwrap_or(&file.path)),
            format!("b/{}", file.path),
        ),
        FileChange::Modified => (format!("a/{}", file.path), format!("b/{}", file.path)),
    }
}

/// `path:Lstart-Lend` tag (1-based, inclusive), anchored on the new side when it
/// has content, else on the base side (a pure deletion). `pub(crate)` so the
/// "copy hunk" confirmation toast (US-003) can label exactly which lines landed.
pub(crate) fn hunk_tag(file: &FileDiff, hunk: &DiffHunk) -> String {
    let (start, end) = if hunk.new_row_range.start != hunk.new_row_range.end {
        (hunk.new_row_range.start + 1, hunk.new_row_range.end)
    } else {
        (hunk.base_row_range.start + 1, hunk.base_row_range.end)
    };
    format!("{}:L{start}-L{end}", file.path)
}

/// One merged hunk group: the union row span (already context-expanded) on each
/// side plus the changed hunks it covers, in order.
struct HunkGroup {
    base: Range<u32>,
    new: Range<u32>,
    hunks: Vec<DiffHunk>,
}

/// Expand each hunk by [`CONTEXT`] lines (clamped to file bounds) and merge hunks
/// whose windows touch, so every emitted `@@` block is a valid, non-overlapping
/// unified-diff hunk. Hunks arrive in row order from `compute_hunks`.
fn group_hunks(hunks: &[DiffHunk], base_lines: u32, new_lines: u32) -> Vec<HunkGroup> {
    let mut groups: Vec<HunkGroup> = Vec::new();
    for h in hunks {
        let bs = h.base_row_range.start.saturating_sub(CONTEXT);
        let be = (h.base_row_range.end + CONTEXT).min(base_lines);
        let ns = h.new_row_range.start.saturating_sub(CONTEXT);
        let ne = (h.new_row_range.end + CONTEXT).min(new_lines);
        if let Some(last) = groups.last_mut()
            && bs <= last.base.end
            && ns <= last.new.end
        {
            last.base.end = last.base.end.max(be);
            last.new.end = last.new.end.max(ne);
            last.hunks.push(h.clone());
            continue;
        }
        groups.push(HunkGroup {
            base: bs..be,
            new: ns..ne,
            hunks: vec![h.clone()],
        });
    }
    groups
}

/// Emit one `@@` header + interleaved context/removed/added body for a group.
fn emit_group(out: &mut String, group: &HunkGroup, base_lines: &[&str], new_lines: &[&str]) {
    let bc = group.base.end - group.base.start;
    let nc = group.new.end - group.new.start;
    let _ = writeln!(
        out,
        "@@ -{} +{} @@",
        fmt_range(group.base.start, bc),
        fmt_range(group.new.start, nc),
    );

    let mut bcur = group.base.start;
    for h in &group.hunks {
        // Leading context: unchanged lines (identical + equal-length on both
        // sides) between the cursor and this hunk; emit from the base side.
        for r in bcur..h.base_row_range.start {
            push_line(out, ' ', base_lines[r as usize]);
        }
        for r in h.base_row_range.clone() {
            push_line(out, '-', base_lines[r as usize]);
        }
        for r in h.new_row_range.clone() {
            push_line(out, '+', new_lines[r as usize]);
        }
        bcur = h.base_row_range.end;
    }
    // Trailing context.
    for r in bcur..group.base.end {
        push_line(out, ' ', base_lines[r as usize]);
    }
}

/// Unified-diff range field: `start+1,count`, or `start,0` for an empty side
/// (git anchors an empty hunk on the line *before* which content is inserted).
fn fmt_range(start: u32, count: u32) -> String {
    if count == 0 {
        format!("{start},0")
    } else {
        format!("{},{count}", start + 1)
    }
}

/// Push one diff line `<prefix><content>`, appending a `\ No newline at end of
/// file` marker when the source line lacks its terminator (only the final line
/// of a side can).
fn push_line(out: &mut String, prefix: char, line: &str) {
    out.push(prefix);
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push('\n');
        out.push_str("\\ No newline at end of file\n");
    }
}

/// Split `text` into lines *including* their `\n` terminator, reproducing
/// `imara_diff::sources::lines_with_terminator` so `DiffHunk` row indices slice
/// correctly: the empty string yields zero lines, and a missing final terminator
/// yields a final line without `\n`.
fn lines_inclusive(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(&text[start..=i]);
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::engine::compute_hunks;

    fn modified(path: &str, base: &str, new: &str) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            change: FileChange::Modified,
            old_path: None,
            base_text: base.to_string(),
            new_text: new.to_string(),
            hunks: compute_hunks(base, new),
            is_binary: false,
        }
    }

    #[test]
    fn lines_inclusive_matches_terminator_semantics() {
        assert_eq!(lines_inclusive("a\nb\n"), vec!["a\n", "b\n"]);
        assert_eq!(lines_inclusive("a\nb"), vec!["a\n", "b"]);
        assert_eq!(lines_inclusive(""), Vec::<&str>::new());
        assert_eq!(lines_inclusive("a\n"), vec!["a\n"]);
    }

    #[test]
    fn new_file_uses_dev_null_and_zero_base_count() {
        let mut f = modified("src/new.rs", "", "a\nb\n");
        f.change = FileChange::Added;
        let out = file_to_unified(&f);
        assert!(out.contains("diff --git a/src/new.rs b/src/new.rs\n"));
        assert!(out.contains("--- /dev/null\n"));
        assert!(out.contains("+++ b/src/new.rs\n"));
        // Empty base side -> `0,0`; two added lines -> `1,2`.
        assert!(out.contains("@@ -0,0 +1,2 @@\n"), "got:\n{out}");
        assert!(out.contains("+a\n"));
        assert!(out.contains("+b\n"));
        assert!(!out.contains("-a"));
    }

    #[test]
    fn pure_deletion_keeps_context_and_counts() {
        let f = modified("x.rs", "a\nb\nc\n", "a\nc\n");
        let out = file_to_unified(&f);
        // 3 base lines shown, 2 new lines shown.
        assert!(out.contains("@@ -1,3 +1,2 @@\n"), "got:\n{out}");
        assert!(out.contains(" a\n"));
        assert!(out.contains("-b\n"));
        assert!(out.contains(" c\n"));
        assert!(!out.contains("+b"));
    }

    #[test]
    fn modification_emits_minus_then_plus() {
        let f = modified("x.rs", "a\nb\nc\n", "a\nB\nc\n");
        let out = file_to_unified(&f);
        assert!(out.contains("@@ -1,3 +1,3 @@\n"), "got:\n{out}");
        let minus = out.find("-b\n").expect("minus line");
        let plus = out.find("+B\n").expect("plus line");
        assert!(minus < plus, "removed line must precede added line:\n{out}");
    }

    #[test]
    fn rename_emits_rename_headers() {
        let mut f = modified("new.rs", "a\nb\n", "a\nB\n");
        f.change = FileChange::Renamed;
        f.old_path = Some("old.rs".to_string());
        let out = file_to_unified(&f);
        assert!(
            out.contains("diff --git a/old.rs b/new.rs\n"),
            "got:\n{out}"
        );
        assert!(out.contains("rename from old.rs\n"));
        assert!(out.contains("rename to new.rs\n"));
        assert!(out.contains("--- a/old.rs\n"));
        assert!(out.contains("+++ b/new.rs\n"));
    }

    #[test]
    fn binary_emits_stub_no_body() {
        let mut f = modified("logo.png", "", "");
        f.is_binary = true;
        f.change = FileChange::Modified;
        let out = file_to_unified(&f);
        assert!(
            out.contains("Binary files a/logo.png b/logo.png differ\n"),
            "got:\n{out}"
        );
        assert!(!out.contains("@@"));
    }

    #[test]
    fn missing_final_newline_marker() {
        // new_text's last line lacks a terminator.
        let f = modified("x.rs", "a\nb\n", "a\nb");
        let out = file_to_unified(&f);
        assert!(
            out.contains("\\ No newline at end of file\n"),
            "got:\n{out}"
        );
    }

    #[test]
    fn far_hunks_split_near_hunks_merge() {
        // Two changes far apart -> two @@ blocks.
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n";
        let new = "X\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\nY\n";
        let f = modified("x.rs", base, new);
        let out = file_to_unified(&f);
        assert_eq!(
            out.matches("@@ ").count(),
            2,
            "far hunks must not merge:\n{out}"
        );

        // Two changes one line apart -> a single merged @@ block.
        let base2 = "a\nb\nc\nd\ne\n";
        let new2 = "A\nb\nc\nd\nE\n";
        let f2 = modified("y.rs", base2, new2);
        let out2 = file_to_unified(&f2);
        assert_eq!(
            out2.matches("@@ ").count(),
            1,
            "near hunks must merge:\n{out2}"
        );
    }

    #[test]
    fn hunk_to_unified_has_tag_and_fence() {
        let f = modified("src/foo.rs", "a\nb\nc\n", "a\nB\nc\n");
        let hunk = &f.hunks[0];
        let out = hunk_to_unified(&f, hunk);
        assert!(out.starts_with("src/foo.rs:L2-L2\n"), "got:\n{out}");
        assert!(out.contains("```diff\n"));
        assert!(out.trim_end().ends_with("```"));
        assert!(out.contains("-b\n"));
        assert!(out.contains("+B\n"));
    }
}
