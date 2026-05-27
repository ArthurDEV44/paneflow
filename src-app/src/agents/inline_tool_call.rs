//! Single-line inline tool-call row, rendered as a top-level
//! `ThreadItem::ToolCall`. Mirrors Zed's `render_tool_call_label`
//! (`agent_ui/src/conversation_view/thread_view.rs:7699`) for the
//! `use_card_layout = false` path: small muted icon + muted markdown
//! title + chevron on hover. Edit / Delete / Move tools still render
//! through the card-layout `edit_tool_block` path.
//!
//! Closed by default. Clicking the row toggles the body, which
//! mirrors the inline thinking treatment: indented under a left
//! border with muted "Input" / "Output" labels.
//!
//! `WaitingForConfirmation` forces the body open and surfaces an
//! Allow / Deny row anchored under it (US-018 / US-110).

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, ClickEvent, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use markdown::{Markdown, MarkdownElement};
use paneflow_acp::PermissionDecision;
use theme::ActiveTheme;
use ui::{Color, Disclosure, Icon, IconName, IconSize, prelude::*};

use super::runtime::{ToolCallSnapshot, ToolCallStatusKind, ToolKindKind};
use crate::theme::UiColors;

/// Render one inline tool-call row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_inline_tool_call(
    entry_ix: usize,
    snap: &ToolCallSnapshot,
    mut label_markdown: Option<Entity<Markdown>>,
    ui: UiColors,
    window: &Window,
    cx: &gpui::App,
    on_toggle: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    on_permission: impl Fn(PermissionDecision, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
    // US-124: extra callbacks for the inline pattern picker. The
    // toggle flips the popover visible/hidden; apply_pattern persists
    // an Allow Always with an optional substring pattern (None ==
    // bare any-input rule, matches the v1 behavior).
    on_toggle_picker: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_apply_pattern: impl Fn(Option<String>, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> AnyElement {
    // Promote the actual shell command to the row label for Execute
    // tools. The agent typically sends `title = "Bash"` / `"shell"`
    // and ships the real command in `raw_input.command`, so the row
    // would otherwise read as a generic "Bash" line with no clue
    // about what's running. Wrapping in backticks turns it into a
    // markdown inline-code chip, matching the look of the path chip
    // we get on Read / Search rows. Drop the cached label_markdown
    // so the renderer falls back to the override (the fresh command
    // string); the stale "Bash" markdown built by `add_tool_call`
    // would otherwise win.
    //
    // The override is computed locally so the snapshot itself stays
    // immutable -- the storage Arc in ThreadView never has to clone
    // when this renderer runs.
    let title_override: Option<String> = if matches!(snap.kind, ToolKindKind::Execute)
        && let Some(cmd) = extract_terminal_command(snap.raw_input_pretty.as_deref())
        && !cmd.trim().is_empty()
    {
        label_markdown = None;
        Some(format!("`{cmd}`"))
    } else {
        None
    };
    let title_for_display: &str = title_override.as_deref().unwrap_or(snap.title.as_str());
    let needs_confirmation = matches!(snap.status, ToolCallStatusKind::WaitingForConfirmation);
    let is_failed = matches!(
        snap.status,
        ToolCallStatusKind::Failed | ToolCallStatusKind::Rejected | ToolCallStatusKind::Canceled
    );
    let has_body = !snap.content_text.is_empty()
        || snap.raw_input_pretty.is_some()
        || snap.raw_output_pretty.is_some();
    let is_collapsible = has_body && !needs_confirmation;
    let mut is_open = snap.expanded;
    is_open |= needs_confirmation;
    let body_open = is_open && has_body;

    // Execute (terminal) tools used to early-return into a
    // `render_terminal_card_inline` helper -- a 1:1 port of Zed's
    // `render_terminal_tool_call` (`thread_view.rs:6328`) that wrapped
    // shell commands in a bordered card with `tool_card_header_bg`
    // header + scrollback body + exit-status footer. The card chrome
    // was visually heavy in long sequences (multiple shell commands
    // interleaved with Read rows produced a stacked-card look that
    // disrupted the flat tool-log rhythm). Execute now renders the
    // same way as Read / Search: a single inline row with the tool
    // icon + the command as the label. The expandable body still
    // shows the full Output on click via the standard `render_body`
    // path. The `extract_terminal_command` helper is reused below to
    // promote `raw_input.command` into the row label so the user
    // sees the actual shell command rather than the agent's generic
    // "Bash" / "shell" title.

    let card_header_id: SharedString = format!("tool-call-header-{entry_ix}").into();

    // Outer vertical spacing: switched from `my(10.)` to `py(10.)`
    // because Paneflow renders the thread inside a GPUI virtual
    // `list` widget (see `ThreadView::render` -> `list(...)`). The
    // list lays out items based on each child's reported intrinsic
    // size, which counts padding but NOT margin -- so `my_1()` /
    // `my(N.)` looked tight in earlier passes regardless of value
    // because the list packed rows flush. `py(10.)` makes the
    // breathing room part of the row's box, which the list honors.
    // 10px top + 10px bottom = 20px between rows.
    let mut col = v_flex().py(px(5.)).mx_5();

    let header = h_flex()
        .group(card_header_id.clone())
        .relative()
        .w_full()
        .justify_between()
        .child(render_tool_call_label(
            snap,
            label_markdown,
            title_for_display,
            window,
            cx,
            ui,
        ))
        .when(is_collapsible || is_failed, |this| {
            this.child(
                h_flex()
                    .pr_0p5()
                    .gap_1()
                    .when(is_collapsible, |this| {
                        this.child(
                            Disclosure::new(("expand-output", entry_ix), is_open)
                                .opened_icon(IconName::ChevronUp)
                                .closed_icon(IconName::ChevronDown)
                                .visible_on_hover(card_header_id.clone())
                                .on_click(on_toggle),
                        )
                    })
                    .when(is_failed, |this| {
                        this.child(
                            Icon::new(IconName::Close)
                                .color(Color::Error)
                                .size(IconSize::Small),
                        )
                    }),
            )
        });

    col = col.child(header);

    if body_open {
        col = col.child(
            div()
                .ml(gpui::rems(0.4))
                .px_3p5()
                .pt_2()
                .border_l_1()
                .when(is_failed, |d| d.border_dashed())
                .border_color(cx.theme().colors().border.opacity(0.8))
                .child(render_body(snap, ui)),
        );
    }

    if needs_confirmation {
        col = col.child(
            div()
                .mt_1()
                .px(px(10.))
                .py(px(8.))
                .rounded(px(5.))
                // Paneflow monochrome surface (was
                // `cx.theme().colors().editor_background` -- GPUI's
                // default editor bg ships as a slightly tinted color
                // that read as greenish against our palette and broke
                // the otherwise neutral chrome of the agent panel).
                .bg(ui.subtle)
                .border_1()
                .border_color(ui.border)
                .child(render_permission_row(
                    snap,
                    ui,
                    on_permission,
                    on_toggle_picker,
                    on_apply_pattern,
                )),
        );
    }

    col.into_any_element()
}

/// Render the single-line "icon + title-markdown + right-edge fade"
/// row that's the visual heart of an inline tool call.
fn render_tool_call_label(
    snap: &ToolCallSnapshot,
    label_markdown: Option<Entity<Markdown>>,
    title_for_display: &str,
    window: &Window,
    cx: &gpui::App,
    ui: UiColors,
) -> AnyElement {
    let _ = window;
    let tool_icon = Icon::new(match snap.kind {
        ToolKindKind::Read => IconName::ToolSearch,
        ToolKindKind::Edit => IconName::ToolPencil,
        ToolKindKind::Delete => IconName::ToolDeleteFile,
        ToolKindKind::Move => IconName::ArrowRightLeft,
        ToolKindKind::Search => IconName::ToolSearch,
        ToolKindKind::Execute => IconName::ToolTerminal,
        ToolKindKind::Think => IconName::ToolThink,
        ToolKindKind::Fetch => IconName::ToolWeb,
        ToolKindKind::SwitchMode => IconName::ArrowRightLeft,
        ToolKindKind::Other => IconName::ToolHammer,
    })
    .size(IconSize::Small)
    .color(Color::Muted);

    let body_color = gpui::rgb(0x9c9c9c).into();
    let label_style = super::markdown_style::paneflow_markdown_style(ui, body_color, 13.0);

    // Gradient overlay deliberately omitted (2026-05-24): Zed's
    // `render_tool_call_label` paints a `w_12` right-edge fade in
    // `cx.theme().colors().panel_background` so a clipped long title
    // dissolves softly into the surrounding panel. In Paneflow the
    // panel paints `theme.title_bar_background` (`0x21252b`), which
    // doesn't match GPUI's default `panel_background` -- so the
    // gradient rendered as a visible dark band that *hid* the right
    // portion of every label instead of blending. Even matching the
    // color, the fade would still consume 32-48px of visible label
    // area at Paneflow's panel widths. `overflow_hidden` already
    // truncates cleanly, so the soft-fade adds no value here.
    //
    // Row sizing: was `h(window.line_height() - px(2.))`. In Paneflow
    // the window default text style is the terminal mono font (~14-16px
    // line height), so subtracting 2 produced a 12-14px row -- too
    // short for a 13px label, and especially for an inline-code chip
    // whose background paints a couple of pixels above/below the
    // baseline. Result: content visually overflowed downward into the
    // next row, causing the "Read Fil" / "Task" stacking artefact.
    //
    // `min_h(26)` gives the row vertical breathing room above the
    // tight 22px floor we set in the first pass -- enough to leave
    // visible padding around the 13px label + inline-code chip on
    // both sides, which makes consecutive rows read as a list rather
    // than a wall of text. Combined with the outer `my(6.)` on the
    // column wrapper, adjacent rows now sit ~24px apart total (12px
    // margin + ~12px of internal vertical padding).
    h_flex()
        .relative()
        .w_full()
        .min_h(px(26.))
        .items_center()
        .text_size(px(13.)) // Zed `tool_name_font_size()` == rems_from_px(13.)
        .gap_1p5()
        .overflow_hidden()
        .child(tool_icon)
        .child(if let Some(md) = label_markdown {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(cx.theme().colors().text_muted)
                .child(MarkdownElement::new(md, label_style))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(cx.theme().colors().text_muted)
                .child(SharedString::from(title_for_display.to_string()))
                .into_any_element()
        })
        .into_any_element()
}

fn render_body(snap: &ToolCallSnapshot, ui: UiColors) -> AnyElement {
    let mut body = v_flex()
        .gap(px(6.))
        .pt(px(4.))
        .pb(px(4.))
        .text_size(px(11.));
    if let Some(raw_in) = &snap.raw_input_pretty
        && !raw_in.trim().is_empty()
    {
        body = body.child(labeled_block("Input", raw_in, ui));
    }
    if !snap.content_text.is_empty() {
        body = body.child(labeled_block("Output", &snap.content_text, ui));
    } else if let Some(raw_out) = &snap.raw_output_pretty
        && !raw_out.trim().is_empty()
    {
        body = body.child(labeled_block("Output", raw_out, ui));
    }
    body.into_any_element()
}

fn labeled_block(label: &'static str, text: &str, ui: UiColors) -> AnyElement {
    const HARD_BYTE_CAP: usize = 16 * 1024;
    let (display, truncated) = if text.len() > HARD_BYTE_CAP {
        let mut cut = text[..HARD_BYTE_CAP].to_string();
        cut.push_str("\n... [truncated]");
        (cut, true)
    } else {
        (text.to_string(), false)
    };
    let mut col = v_flex().gap(px(3.)).child(
        div()
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(ui.muted)
            .child(format!("{label}:")),
    );
    col = col.child(
        div()
            .id(SharedString::from(format!("tool-inline-{label}")))
            .overflow_x_scroll()
            .px(px(8.))
            .py(px(6.))
            .rounded(px(4.))
            .bg(ui.base)
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .font_family("Lilex")
            .text_size(px(10.))
            .child(display),
    );
    if truncated {
        col = col.child(
            div()
                .text_size(px(10.))
                .text_color(ui.muted)
                .italic()
                .child("(output > 16 KB; truncated for display)"),
        );
    }
    col.into_any_element()
}

fn render_permission_row(
    snap: &ToolCallSnapshot,
    ui: UiColors,
    on_permission: impl Fn(PermissionDecision, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_toggle_picker: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_apply_pattern: impl Fn(Option<String>, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> AnyElement {
    let allow_once_label: SharedString = snap
        .permission_options
        .iter()
        .find(|o| matches!(o.kind, super::runtime::PermissionOptionKindKind::AllowOnce))
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "Allow Once".to_string())
        .into();
    let allow_always_label: SharedString = snap
        .permission_options
        .iter()
        .find(|o| {
            matches!(
                o.kind,
                super::runtime::PermissionOptionKindKind::AllowAlways
            )
        })
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "Allow Always".to_string())
        .into();
    let reject_label: SharedString = snap
        .permission_options
        .iter()
        .find(|o| {
            matches!(
                o.kind,
                super::runtime::PermissionOptionKindKind::RejectOnce
                    | super::runtime::PermissionOptionKindKind::RejectAlways
            )
        })
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "Reject".to_string())
        .into();

    // US-124 AC #4: non-POSIX shells (Nu, Elvish, Rc, pwsh, cmd) do
    // not support the substring-pattern semantic cleanly -- shell
    // grammar varies, and a literal substring of a Nushell command
    // would match unintended pipelines. Hide the picker AND the
    // bare-rule "Allow Always" entirely on those shells: only "Allow
    // Once" + "Reject" remain.
    let active_shell_is_posix = crate::terminal::shell::is_posix_shell_basename(
        crate::terminal::shell::active_basename().as_str(),
    );
    let is_execute = matches!(snap.kind, ToolKindKind::Execute);
    let hide_always = is_execute && !active_shell_is_posix;

    // US-124 AC #1: for terminal calls we propose concrete patterns
    // parsed from the command. For non-terminal kinds, the single
    // "everywhere" proposal preserves the v1 any-input semantic.
    let pattern_proposals = if is_execute {
        let command = extract_terminal_command(snap.raw_input_pretty.as_deref());
        let mut props =
            super::panel_config::propose_terminal_patterns(command.as_deref().unwrap_or(""));
        // Always offer the broadest "everywhere" option last so the
        // ordering reads from most-specific to least-specific.
        props.push(super::panel_config::PatternProposal::everywhere());
        props
    } else {
        vec![super::panel_config::PatternProposal::everywhere()]
    };
    // The picker is meaningful only when there are at least 2
    // distinct proposals -- otherwise we'd just be wrapping the
    // single "everywhere" click in an extra step.
    let show_picker_button = pattern_proposals.len() > 1 && !hide_always;
    let picker_open = snap.permission_picker_open && show_picker_button;

    let id = snap.id.clone();
    let allow_once_id: SharedString = format!("tool-allow-once-{id}").into();
    let allow_always_id: SharedString = format!("tool-allow-always-{id}").into();
    let reject_id: SharedString = format!("tool-reject-{id}").into();
    let on_once = on_permission.clone();
    let on_reject = on_permission.clone();
    let on_apply_any = on_apply_pattern.clone();
    let tool_kind = snap.kind;

    // Monochrome permission buttons (2026-05-25): the three actions
    // share an identical ghost look so the row reads as neutral
    // chrome rather than a colorful CTA. Distinction comes from the
    // label + icon shape, not the colour -- a deliberate move away
    // from the previous accent-fill Allow Once + green/red icons,
    // which clashed with the rest of the panel's monochrome palette
    // (mirrors the same call the send button got: every state
    // collapses to one muted ghost treatment).
    //
    // Structurally still parallel to Zed's
    // `render_permission_buttons_flat` (`thread_view.rs:7572`) --
    // same three options, same kinds, same actions -- only the
    // colour mapping differs (Zed gives each kind a tint via
    // `Color::Success` / `Color::Error`; Paneflow renders them all
    // in `ui.muted`).
    let ghost_button = |id: SharedString, label: SharedString, icon: IconName| {
        div()
            .id(id)
            .px(px(10.))
            .py(px(4.))
            .rounded(px(4.))
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|d| d.bg(ui.subtle))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(Icon::new(icon).size(IconSize::XSmall).color(Color::Muted))
                    .child(label),
            )
    };

    let allow_once_btn = ghost_button(allow_once_id, allow_once_label, IconName::Check).on_click(
        move |_ev: &ClickEvent, w, cx| {
            on_once(PermissionDecision::AllowOnce, w, cx);
        },
    );

    let allow_always_btn = ghost_button(allow_always_id, allow_always_label, IconName::CheckDouble)
        .on_click({
            let on_apply_any = on_apply_any.clone();
            let on_toggle_picker = on_toggle_picker.clone();
            move |_ev: &ClickEvent, w, cx| {
                if show_picker_button {
                    on_toggle_picker(w, cx);
                } else {
                    let _ = tool_kind; // keep parity with prior signature
                    on_apply_any(None, w, cx);
                }
            }
        });

    let reject_btn = ghost_button(reject_id, reject_label, IconName::Close).on_click(
        move |_ev: &ClickEvent, w, cx| {
            on_reject(PermissionDecision::Reject, w, cx);
        },
    );

    // US-015: Zed's flat permission row sits flush against the tool card
    // with `border_t_1` above and only the buttons inside (no "waiting for
    // approval" filler -- the surrounding card chrome conveys the wait state).
    // We mirror that here: drop the muted filler text and right-align the
    // buttons inside an h_flex with gap_1 (Zed flat uses gap_0p5 = 2px;
    // 4px reads better at Paneflow's row density).
    let mut row = h_flex()
        .items_center()
        .justify_end()
        .gap(px(4.))
        .child(reject_btn);
    if !hide_always {
        row = row.child(allow_always_btn);
    }
    let row = row.child(allow_once_btn);

    // US-124 AC #1: render the pattern picker as a vertical column
    // below the button row when toggled open. Each row is a single
    // clickable line; clicking it persists the corresponding pattern
    // and commits AllowAlways via `on_apply_pattern`.
    let mut col = v_flex().gap(px(6.)).child(row);
    if picker_open {
        let mut picker_col = v_flex().gap(px(4.)).mt_1();
        for proposal in pattern_proposals {
            let label: SharedString = proposal.label.clone().into();
            let pattern = proposal.pattern.clone();
            let on_apply = on_apply_pattern.clone();
            let id_btn: SharedString = format!("tool-perm-pattern-{}-{}", id, pattern).into();
            picker_col = picker_col.child(
                div()
                    .id(id_btn)
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(ui.subtle)
                    .border_1()
                    .border_color(ui.border)
                    .text_color(ui.text)
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|d| d.bg(ui.border))
                    .child(label)
                    .on_click(move |_ev: &ClickEvent, w, cx| {
                        let p = if pattern.is_empty() {
                            None
                        } else {
                            Some(pattern.clone())
                        };
                        on_apply(p, w, cx);
                    }),
            );
        }
        col = col.child(picker_col);
    }
    col.into_any_element()
}

/// US-124 helper: peel the `command` string out of a pretty-printed
/// raw_input JSON. Returns `None` when the payload is not parseable
/// or carries no `command` field. The `command` key is the
/// convention every ACP wrapper today uses for terminal tool input.
fn extract_terminal_command(raw_input_pretty: Option<&str>) -> Option<String> {
    let pretty = raw_input_pretty?;
    let value: serde_json::Value = serde_json::from_str(pretty).ok()?;
    value
        .as_object()?
        .get("command")?
        .as_str()
        .map(|s| s.to_string())
}
