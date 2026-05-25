//! Standalone edit tool block.
//!
//! Renders a single [`ToolCallSnapshot`] that has been classified as
//! "standalone" (Edit / Delete / Move, or any tool that surfaced a
//! diff). Unlike the in-card compact row, this block takes the full
//! chat width, shows the file path in the header, and paints a
//! Claude-Code-style +/- diff body with line numbers in a left
//! gutter and full-width row backgrounds for added / removed lines.
//!
//! Layout:
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │ ● Edit  src/main.rs                                +5 -2 ▾ │
//! ├────────────────────────────────────────────────────────────┤
//! │   1 │ fn main() {                                          │
//! │   2 │-    println!("hi");                                  │
//! │   2 │+    println!("hello, world");                        │
//! │   3 │ }                                                    │
//! └────────────────────────────────────────────────────────────┘
//! ```

#![allow(dead_code)]

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, ClickEvent, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px, svg,
};

use super::continuous_spinner::continuous_spinner;
use super::runtime::{DiffSnapshot, ToolCallSnapshot, ToolCallStatusKind, ToolKindKind};
use crate::theme::UiColors;

/// Render the standalone block. `on_toggle_expanded` flips the diff
/// body open/closed (mirrors `tool_call_view::render_tool_call`'s
/// click contract so the parent ThreadView can route both card and
/// block toggles through `toggle_tool_call_expanded`).
///
/// The Keep All / Reject All review buttons live in the activity bar
/// above the composer (see `render_activity_bar`) -- not in the card
/// itself. The card stays focused on the file + diff preview.
pub(crate) fn render_edit_tool_block(
    snap: ToolCallSnapshot,
    ui: UiColors,
    on_toggle_expanded: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    diff_scroll: gpui::ScrollHandle,
) -> AnyElement {
    let (added, removed) = diff_stats(&snap.diffs);
    let primary_path = snap
        .diffs
        .first()
        .map(|d| d.path.display().to_string())
        .unwrap_or_default();
    let is_failed = matches!(
        snap.status,
        ToolCallStatusKind::Failed | ToolCallStatusKind::Rejected
    );
    let needs_confirmation = matches!(snap.status, ToolCallStatusKind::WaitingForConfirmation);

    let _ = needs_confirmation;
    // Single clickable header: id + on_click live on the same flex row
    // that paints the chrome so the entire visible area registers
    // pointer events. The previous nested-div layout left the wrapper
    // sized at 0x0 in some cases, making the chevron click silently
    // dead. Mirrors Zed's `render_tool_call_label` which puts the
    // disclosure + label + bg + cursor on one h_flex (no wrapping div).
    // Monochrome DA: `ui.subtle` instead of `ui.tool_card_header_bg`
    // (which is bluish) keeps the card neutral.
    let header_row = render_header(&snap, &primary_path, added, removed, ui, on_toggle_expanded);

    // US-110 AC #1: Failed/Rejected tool cards swap their solid
    // border for a dashed one. The pattern reads as "this didn't
    // land" at a glance, even when the user is scanning a long
    // thread without reading individual labels.
    //
    // US-012 (visual-parity): `tool_card_border_color` in Zed is
    // `border.opacity(0.8)` and the outer container clips its body via
    // `overflow_hidden()` so the diff doesn't bleed past the rounded
    // corners.
    let mut col = div()
        .flex()
        .flex_col()
        .mx(px(16.))
        .my(px(8.))
        .rounded(px(6.))
        .border_1()
        .when(is_failed, |d| d.border_dashed())
        .border_color(ui.border.opacity(0.8))
        .bg(ui.surface)
        .overflow_hidden()
        .child(header_row);

    if snap.expanded && !snap.diffs.is_empty() {
        // US-012 AC #5: cap the diff body at 400 px and let the user
        // scroll inside. Mirrors Zed's diff scrollability (Zed renders
        // the diff inside an `Editor` which has its own scroll; we
        // approximate by capping the height and routing the scroll to
        // GPUI's overflow_y_scroll on our LCS-rendered diff column).
        col = col.child(
            div()
                .id(SharedString::from(format!("edit-tool-diff-{}", snap.id)))
                .border_t_1()
                .when(is_failed, |d| d.border_dashed())
                .border_color(ui.border.opacity(0.8))
                .max_h(px(400.))
                .overflow_y_scroll()
                // Explicit `track_scroll` claims wheel events for this
                // handle. Without it, GPUI's `list()` widget that
                // wraps the thread also reacts to the wheel and the
                // two scroll in lockstep. The handle is owned by
                // ThreadView (`diff_scroll_handles`) so position
                // persists across renders. The trailing
                // `stop_propagation` is belt-and-braces for any
                // wheel events that aren't fully absorbed (e.g. when
                // the diff is shorter than 400 px there is no scroll
                // distance to consume, but the list shouldn't scroll
                // either).
                .track_scroll(&diff_scroll)
                .on_scroll_wheel(|_ev, _w, cx| cx.stop_propagation())
                .children(snap.diffs.iter().map(|d| render_diff(d, ui))),
        );
    }

    col.into_any_element()
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn render_header(
    snap: &ToolCallSnapshot,
    path: &str,
    added: usize,
    removed: usize,
    ui: UiColors,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let verb = verb_for(snap);
    let chevron_path = if snap.expanded {
        "icons/chevron-down.svg"
    } else {
        "icons/chevron-right.svg"
    };

    let decorate_failed = matches!(
        snap.status,
        ToolCallStatusKind::Failed | ToolCallStatusKind::Rejected
    ) && snap.expanded
        && !snap.diffs.is_empty();

    // No `.rounded_t()` here -- the parent `col` already clips its
    // contents via `overflow_hidden() + rounded(px(6.))`, so painting
    // our own corner radius would leave a visible bg gap inside the
    // clipped corner. Let the parent's rounding be the source of truth.
    div()
        .id(SharedString::from(format!("edit-tool-header-{}", snap.id)))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(8.))
        .px(px(12.))
        .py(px(6.))
        .bg(ui.subtle)
        .cursor_pointer()
        .on_click(on_click)
        .child(status_marker(snap.status, ui, &snap.id, decorate_failed))
        .child(
            div()
                .flex_none()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui.text)
                .child(verb),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(ui.text)
                .font_family("Lilex")
                .child(path.to_string()),
        )
        .when(added > 0, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(added_fg())
                    .child(format!("+{added}")),
            )
        })
        .when(removed > 0, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(removed_fg())
                    .child(format!("-{removed}")),
            )
        })
        .child(
            svg()
                .size(px(10.))
                .flex_none()
                .path(chevron_path)
                .text_color(ui.muted),
        )
        .into_any_element()
}

fn verb_for(snap: &ToolCallSnapshot) -> &'static str {
    if !snap.title.is_empty() {
        // Title from the agent (e.g. "Update src/main.rs") often
        // already encodes the verb; we still pick a stable short
        // label for the chip so users scan tool kinds at a glance.
    }
    match snap.kind {
        ToolKindKind::Delete => "Delete",
        ToolKindKind::Move => "Move",
        ToolKindKind::Edit => {
            // New file -> "Create"; existing file -> "Update".
            if snap
                .diffs
                .first()
                .is_some_and(|d| d.old_text.is_none() || d.old_text.as_deref() == Some(""))
            {
                "Create"
            } else {
                "Update"
            }
        }
        _ => "Edit",
    }
}

fn status_marker(
    status: ToolCallStatusKind,
    ui: UiColors,
    id: &str,
    decorate_failed: bool,
) -> AnyElement {
    match status {
        ToolCallStatusKind::Pending | ToolCallStatusKind::InProgress => continuous_spinner(
            SharedString::from(format!("edit-tool-spinner-{id}")),
            px(12.),
            ui.text,
        )
        .into_any_element(),
        ToolCallStatusKind::Completed => svg()
            .size(px(12.))
            .flex_none()
            .path("icons/checks.svg")
            .text_color(added_fg())
            .into_any_element(),
        ToolCallStatusKind::Failed | ToolCallStatusKind::Rejected => {
            let path = if decorate_failed {
                "icons/triangle-alert.svg"
            } else {
                "icons/generic_close.svg"
            };
            svg()
                .size(px(12.))
                .flex_none()
                .path(path)
                .text_color(removed_fg())
                .into_any_element()
        }
        ToolCallStatusKind::Canceled => svg()
            .size(px(12.))
            .flex_none()
            .path("icons/generic_close.svg")
            .text_color(ui.muted)
            .into_any_element(),
        ToolCallStatusKind::WaitingForConfirmation => svg()
            .size(px(12.))
            .flex_none()
            .path("icons/loader-circle.svg")
            .text_color(ui.muted)
            .into_any_element(),
    }
}

// ---------------------------------------------------------------------------
// Diff body
// ---------------------------------------------------------------------------

/// Hue + saturation tuned to read as "diff green" / "diff red" without
/// pushing the card out of the surrounding monochrome chrome. Mirrors
/// the HSL profile Zed uses for `editor_diff_hunk_added_background` /
/// `editor_diff_hunk_deleted_background` (hsl 134/55/40 and hsl
/// 350/88/25) -- the background swatches lift the line off the
/// surface at 16 % alpha, the foreground tone lifts the marker /
/// counter label to high contrast.
fn added_bg() -> gpui::Hsla {
    gpui::hsla(134.0 / 360.0, 0.55, 0.40, 0.16)
}
fn removed_bg() -> gpui::Hsla {
    gpui::hsla(350.0 / 360.0, 0.88, 0.25, 0.16)
}
pub(crate) fn added_fg() -> gpui::Hsla {
    gpui::hsla(134.0 / 360.0, 0.55, 0.60, 1.0)
}
pub(crate) fn removed_fg() -> gpui::Hsla {
    gpui::hsla(350.0 / 360.0, 0.55, 0.65, 1.0)
}

fn render_diff(diff: &DiffSnapshot, ui: UiColors) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .font_family("Lilex")
        .text_size(px(11.))
        .text_color(ui.text);

    // When a per-diff path differs from the header, surface it as a
    // small caption so multi-file edits stay legible.
    let path_caption = diff.path.display().to_string();
    if !path_caption.is_empty() {
        col = col.child(
            div()
                .px(px(12.))
                .py(px(4.))
                .text_size(px(10.))
                .text_color(ui.muted)
                .font_family("Lilex")
                .child(path_caption),
        );
    }

    let lines = line_diff(diff.old_text.as_deref().unwrap_or(""), &diff.new_text);
    let gutter_width = compute_gutter_width(&lines);

    for line in lines {
        let (bg, marker, marker_color) = match line.kind {
            DiffLineKind::Added => (added_bg(), "+", added_fg()),
            DiffLineKind::Removed => (removed_bg(), "-", removed_fg()),
            DiffLineKind::Context => (gpui::transparent_black(), " ", ui.muted),
        };
        col = col.child(render_diff_row(
            line,
            bg,
            marker,
            marker_color,
            ui,
            gutter_width,
        ));
    }

    col.into_any_element()
}

fn render_diff_row(
    line: DiffLine,
    bg: Hsla,
    marker: &'static str,
    marker_color: Hsla,
    ui: UiColors,
    gutter_width: f32,
) -> AnyElement {
    let old_no = line
        .old_no
        .map(|n| n.to_string())
        .unwrap_or_else(|| String::from(" "));
    let new_no = line
        .new_no
        .map(|n| n.to_string())
        .unwrap_or_else(|| String::from(" "));

    div()
        .flex()
        .flex_row()
        .items_start()
        .bg(bg)
        .px(px(8.))
        .child(
            div()
                .flex_none()
                .w(px(gutter_width))
                .text_color(ui.muted)
                .text_size(px(10.))
                .child(old_no),
        )
        .child(
            div()
                .flex_none()
                .w(px(gutter_width))
                .text_color(ui.muted)
                .text_size(px(10.))
                .child(new_no),
        )
        .child(
            div()
                .flex_none()
                .w(px(14.))
                .text_color(marker_color)
                .child(marker),
        )
        .child(div().flex_1().min_w_0().child(line.content))
        .into_any_element()
}

fn compute_gutter_width(lines: &[DiffLine]) -> f32 {
    let max_no = lines
        .iter()
        .filter_map(|l| l.old_no.or(l.new_no))
        .max()
        .unwrap_or(1);
    let digits = max_no.to_string().len() as f32;
    // ~7 px per monospace digit at 10 px font + 4 px right padding.
    (digits * 7.0).max(20.0) + 4.0
}

// ---------------------------------------------------------------------------
// Minimal line-level diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct DiffLine {
    kind: DiffLineKind,
    old_no: Option<usize>,
    new_no: Option<usize>,
    content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLineKind {
    Added,
    Removed,
    Context,
}

/// Compute a unified-style line diff via the longest-common-subsequence
/// algorithm. This is intentionally simple (O(n*m) time / memory) so we
/// don't pull a diff crate just for the agent UI; for typical Edit tool
/// payloads (a few dozen to a few hundred lines) the cost is invisible.
///
/// Output: every line from the longer of the two sequences appears in
/// the result, tagged Added / Removed / Context, with both legacy and
/// new line numbers filled in where they apply.
fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = if old.is_empty() {
        Vec::new()
    } else {
        old.split('\n').collect()
    };
    let new_lines: Vec<&str> = if new.is_empty() {
        Vec::new()
    } else {
        new.split('\n').collect()
    };

    // Guard against pathological inputs (e.g. a 200K-line file pasted
    // into both sides). Anything beyond MAX_LINES skips the LCS pass
    // and falls back to "remove-all-then-add-all" which is still
    // readable.
    const MAX_LINES: usize = 4_000;
    if old_lines.len() > MAX_LINES || new_lines.len() > MAX_LINES {
        return naive_replace(&old_lines, &new_lines);
    }

    let lcs = lcs_table(&old_lines, &new_lines);
    let mut out = Vec::with_capacity(old_lines.len() + new_lines.len());
    let mut i = old_lines.len();
    let mut j = new_lines.len();
    let mut backtrack: Vec<DiffLine> = Vec::with_capacity(old_lines.len() + new_lines.len());

    while i > 0 && j > 0 {
        if old_lines[i - 1] == new_lines[j - 1] {
            backtrack.push(DiffLine {
                kind: DiffLineKind::Context,
                old_no: Some(i),
                new_no: Some(j),
                content: old_lines[i - 1].to_string(),
            });
            i -= 1;
            j -= 1;
        } else if lcs[i][j - 1] >= lcs[i - 1][j] {
            backtrack.push(DiffLine {
                kind: DiffLineKind::Added,
                old_no: None,
                new_no: Some(j),
                content: new_lines[j - 1].to_string(),
            });
            j -= 1;
        } else {
            backtrack.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_no: Some(i),
                new_no: None,
                content: old_lines[i - 1].to_string(),
            });
            i -= 1;
        }
    }
    while i > 0 {
        backtrack.push(DiffLine {
            kind: DiffLineKind::Removed,
            old_no: Some(i),
            new_no: None,
            content: old_lines[i - 1].to_string(),
        });
        i -= 1;
    }
    while j > 0 {
        backtrack.push(DiffLine {
            kind: DiffLineKind::Added,
            old_no: None,
            new_no: Some(j),
            content: new_lines[j - 1].to_string(),
        });
        j -= 1;
    }
    backtrack.reverse();
    out.extend(backtrack);
    out
}

fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
    let n = a.len();
    let m = b.len();
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            table[i][j] = if a[i - 1] == b[j - 1] {
                table[i - 1][j - 1] + 1
            } else {
                table[i][j - 1].max(table[i - 1][j])
            };
        }
    }
    table
}

fn naive_replace(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    let mut out = Vec::with_capacity(old.len() + new.len());
    for (i, line) in old.iter().enumerate() {
        out.push(DiffLine {
            kind: DiffLineKind::Removed,
            old_no: Some(i + 1),
            new_no: None,
            content: (*line).to_string(),
        });
    }
    for (j, line) in new.iter().enumerate() {
        out.push(DiffLine {
            kind: DiffLineKind::Added,
            old_no: None,
            new_no: Some(j + 1),
            content: (*line).to_string(),
        });
    }
    out
}

/// Aggregate added / removed counts across all diffs on this tool
/// call. Used for the header chip (`+12 -3`).
pub(crate) fn diff_stats(diffs: &[DiffSnapshot]) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for d in diffs {
        let lines = line_diff(d.old_text.as_deref().unwrap_or(""), &d.new_text);
        for l in lines {
            match l.kind {
                DiffLineKind::Added => added += 1,
                DiffLineKind::Removed => removed += 1,
                DiffLineKind::Context => {}
            }
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diff_empty_old_marks_everything_added() {
        let d = line_diff("", "a\nb\n");
        assert_eq!(d.len(), 3); // trailing empty line counts
        assert!(d.iter().all(|l| l.kind == DiffLineKind::Added));
    }

    #[test]
    fn line_diff_empty_new_marks_everything_removed() {
        let d = line_diff("a\nb", "");
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|l| l.kind == DiffLineKind::Removed));
    }

    #[test]
    fn line_diff_keeps_context_lines() {
        let old = "fn main() {\n    println!(\"hi\");\n}\n";
        let new = "fn main() {\n    println!(\"hello\");\n}\n";
        let d = line_diff(old, new);
        // 4 logical lines (incl trailing empty): context, removed, added, context, context
        let kinds: Vec<DiffLineKind> = d.iter().map(|l| l.kind).collect();
        assert!(kinds.contains(&DiffLineKind::Context));
        assert!(kinds.contains(&DiffLineKind::Added));
        assert!(kinds.contains(&DiffLineKind::Removed));
    }

    #[test]
    fn diff_stats_counts_added_and_removed() {
        let diffs = vec![DiffSnapshot {
            path: std::path::PathBuf::from("x.rs"),
            old_text: Some("a\nb\nc".to_string()),
            new_text: "a\nB\nc".to_string(),
        }];
        let (added, removed) = diff_stats(&diffs);
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }
}
