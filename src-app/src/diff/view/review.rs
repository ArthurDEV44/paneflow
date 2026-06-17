//! Review-terminal lifecycle (launch / pick / close) + per-column review menu
//! for the Review view (US-004 code-motion). See [`super`] for `DiffView`.

use super::*;

impl DiffView {
    /// Append `text` to the column's review CLI input WITHOUT Enter, then focus
    /// it, so the user types their question after. If NO session is open on the
    /// column, default to launching Claude Code and pre-fill `text` once it boots
    /// (prd-ai-in-diff-2026-Q3.md: left-click a line with no session running).
    pub(super) fn send_to_review(
        &mut self,
        col_idx: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Existing session -> send immediately + focus.
        if let Some(col) = self.columns.get(col_idx)
            && let Some(rt) = col.review_terminals.first()
        {
            let term = rt.terminal.clone();
            term.read(cx).send_text(&text);
            term.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        // No session -> launch Claude Code by default, pre-fill `text` after boot.
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let cli = super::super::review_terminal::ReviewCli::ClaudeCode;
        let term = cx.new(|cx| crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd), None, cx));
        let config = paneflow_config::loader::load_config();
        let command = cli.launch_command(&config);
        // US-011: configurable prefill delay (default 2000 ms). The clipboard
        // write below stays the synchronous safety net regardless of the delay.
        let delay = config.resolved_review_prefill_delay_ms();
        term.read(cx).send_command(&command);
        let prefill = text.clone();
        let term_weak = term.downgrade();
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            smol::Timer::after(Duration::from_millis(delay)).await;
            cx.update(|cx| {
                if let Some(t) = term_weak.upgrade() {
                    t.read(cx).send_text(&prefill);
                }
            });
        })
        .detach();
        term.read(cx).focus_handle(cx).focus(window, cx);
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals.push(ReviewTerminal {
                label: cli.label().into(),
                terminal: term,
            });
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        cx.notify();
    }

    /// Send a changed line (`path:line` + content) into the review CLI input so
    /// the user can ask about it.
    pub(super) fn ask_review_about_line(
        &mut self,
        col_idx: usize,
        line: ClickedLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = if line.removed {
            format!("{}:{} (removed)", line.path, line.lineno)
        } else {
            format!("{}:{}", line.path, line.lineno)
        };
        let text = format!("`{tag}` `{}` — ", line.content.trim());
        self.send_to_review(col_idx, text, window, cx);
    }

    /// Send a hunk's unified diff into the review CLI input so the user can ask
    /// about it.
    pub(super) fn ask_review_about_hunk(
        &mut self,
        col_idx: usize,
        scope: DiffBodyScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let ColumnState::Loaded { files_full, .. } = &col.state else {
                return;
            };
            let Some(file) = files_full.get(scope.file_idx) else {
                return;
            };
            let Some(hunk) = scope.hunk_idx.and_then(|h| file.hunks.get(h)) else {
                return;
            };
            format!(
                "About this change:\n{}\n",
                super::super::extract::hunk_to_unified(file, hunk)
            )
        };
        self.send_to_review(col_idx, text, window, cx);
    }
    /// EP-005 US-018/019/020: send a hunk to the active review CLI framed for an
    /// "act" intent (direct / fix-and-stage / discard), pre-filled, no
    /// auto-submit. Reuses [`Self::send_to_review`] (launches Claude Code if no
    /// CLI is open). Paneflow runs no git write itself — the agent performs any
    /// staging/discard in the witnessed terminal.
    pub(super) fn act_on_hunk(
        &mut self,
        col_idx: usize,
        scope: DiffBodyScope,
        action: super::super::review_terminal::HunkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let ColumnState::Loaded { files_full, .. } = &col.state else {
                return;
            };
            let Some(file) = files_full.get(scope.file_idx) else {
                return;
            };
            let Some(hunk) = scope.hunk_idx.and_then(|h| file.hunks.get(h)) else {
                return;
            };
            let diff = super::super::extract::hunk_to_unified(file, hunk);
            super::super::review_terminal::build_cli_hunk_prompt(action, &file.path, &diff)
        };
        // Any act clears a pending armed-discard (the user committed to an action).
        self.hunk_discard_armed = None;
        self.send_to_review(col_idx, text, window, cx);
    }

    /// A branch column has something to review when it's loaded with > 0 files.
    pub(super) fn column_has_changes(col: &Column) -> bool {
        matches!(&col.state, ColumnState::Loaded { file_count, .. } if *file_count > 0)
    }

    /// Open/close a column's Review CLI multi-select. On open, sync the pick
    /// toggles to the CLI list (default all-on). Clicking the same column's
    /// Review button again (or a different one) toggles / re-targets the popover.
    pub(super) fn toggle_review_menu(&mut self, col_idx: usize, cx: &mut Context<Self>) {
        if self.review_menu_open == Some(col_idx) {
            self.review_menu_open = None;
        } else {
            self.review_menu_open = Some(col_idx);
            let n = super::super::review_terminal::ReviewCli::all().len();
            if self.review_picks.len() != n {
                self.review_picks = vec![true; n];
            }
        }
        cx.notify();
    }

    /// Toggle one CLI's inclusion in the next review.
    pub(super) fn toggle_review_pick(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.review_picks.get_mut(i) {
            *p = !*p;
            cx.notify();
        }
    }

    /// Launch the selected CLIs to review column `col_idx`'s branch: one real
    /// terminal per CLI, embedded UNDER the column's diff (in the Diff interface,
    /// not the CLI mode), cwd-pinned to the worktree, with a compact review prompt
    /// pre-filled (the human submits). Human-in-the-loop — no headless session.
    pub(super) fn launch_review(
        &mut self,
        col_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.review_menu_open = None;
        let clis = super::super::review_terminal::ReviewCli::all();
        let selected: Vec<usize> = (0..clis.len())
            .filter(|i| self.review_picks.get(*i).copied().unwrap_or(true))
            .collect();
        if selected.is_empty() {
            self.set_flash("Select at least one CLI".into(), cx);
            return;
        }
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let branch = col.branch.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let base = col
            .base_override
            .clone()
            .unwrap_or_else(|| self.base_ref.clone());

        // One terminal per selected CLI; the 2nd+ get the adversarial framing so a
        // multi-CLI panel is a real second opinion, not an echo.
        let mut created: Vec<ReviewTerminal> = Vec::new();
        let mut first_prompt: Option<String> = None;
        let mut focus_target: Option<Entity<crate::terminal::TerminalView>> = None;
        let config = paneflow_config::loader::load_config();
        // US-011: configurable prefill delay (default 2000 ms); clipboard write
        // (below) remains the synchronous safety net.
        let delay = config.resolved_review_prefill_delay_ms();
        for (rank, &i) in selected.iter().enumerate() {
            let cli = clis[i];
            let prompt =
                super::super::review_terminal::build_cli_review_prompt(&branch, &base, rank > 0);
            let term = cx.new(|cx| {
                crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd.clone()), None, cx)
            });
            // Launch the CLI in the embedded terminal's shell.
            let command = cli.launch_command(&config);
            term.read(cx).send_command(&command);
            // Pre-fill the prompt once the CLI has booted (tmux send-keys style):
            // a delayed write with NO Enter — the human reviews + submits. The
            // clipboard fallback (below) covers a missed timing window.
            let prefill = prompt.clone();
            let term_weak = term.downgrade();
            cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(delay)).await;
                cx.update(|cx| {
                    if let Some(t) = term_weak.upgrade() {
                        t.read(cx).send_text(&prefill);
                    }
                });
            })
            .detach();
            let label = if rank > 0 {
                format!("{} · 2nd opinion", cli.label())
            } else {
                cli.label().to_string()
            };
            if focus_target.is_none() {
                focus_target = Some(term.clone());
            }
            if first_prompt.is_none() {
                first_prompt = Some(prompt);
            }
            created.push(ReviewTerminal {
                label: label.into(),
                terminal: term,
            });
        }

        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals = created; // replace any prior run (drops old PTYs)
        }
        if let Some(t) = focus_target {
            t.read(cx).focus_handle(cx).focus(window, cx);
        }
        if let Some(p) = first_prompt {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(p));
        }
        cx.notify();
    }

    /// Close one embedded terminal (drops it → PTY shutdown).
    pub(super) fn close_review_terminal(
        &mut self,
        col_idx: usize,
        term_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(col) = self.columns.get_mut(col_idx) else {
            return;
        };
        if term_idx < col.review_terminals.len() {
            col.review_terminals.remove(term_idx);
            cx.notify();
        }
    }

    /// Terminal button on a column header: open a plain shell terminal in the
    /// branch's worktree, embedded under the diff. Just a terminal — no CLI
    /// launch, no prefill (distinct from Review).
    pub(super) fn open_terminal_for_column(
        &mut self,
        col_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let term = cx.new(|cx| crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd), None, cx));
        term.read(cx).focus_handle(cx).focus(window, cx);
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals.push(ReviewTerminal {
                label: "Terminal".into(),
                terminal: term,
            });
        }
        cx.notify();
    }

    /// Render the embedded review terminals under a column's diff body (one card
    /// per CLI, side by side). `None` when the column has no review running.
    pub(super) fn render_review_terminals(
        &self,
        col_idx: usize,
        col: &Column,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if col.review_terminals.is_empty() {
            return None;
        }
        // US-049: the review prompt is pre-filled after a fixed delay, but on a
        // slow CLI cold-start (notably Windows ConPTY) the auto-fill can miss
        // its window. The prompt is always copied to the clipboard as a fallback
        // — surface that explicitly so the user can paste it instead of staring
        // at an empty input. Shown on the first terminal only, since the
        // clipboard holds the first CLI's prompt (2nd-opinion prompts differ).
        let paste_key = if cfg!(target_os = "macos") {
            "⌘V"
        } else {
            "Ctrl+V"
        };
        let terminals = div().flex_1().min_h_0().flex().flex_row().children(
            col.review_terminals.iter().enumerate().map(|(ti, rt)| {
                let header = div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(3.))
                    .bg(ui.surface)
                    .border_b_1()
                    .border_color(ui.border)
                    .child(
                        gpui::svg()
                            .size(px(11.))
                            .flex_none()
                            .path("icons/terminal.svg")
                            .text_color(ui.accent),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(crate::ui_primitives::LABEL_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text)
                            .child(rt.label.clone()),
                    )
                    // US-011: surface the clipboard safety net PROMINENTLY and
                    // immediately (this renders the instant the terminal mounts,
                    // before the prefill timer fires) so a slow cold-start that
                    // misses the auto-fill window degrades to one paste, not a
                    // silent empty input.
                    .when(ti == 0, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.))
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(ui.accent.opacity(0.14))
                                .text_size(crate::ui_primitives::LABEL_XS)
                                .text_color(ui.accent)
                                .child(
                                    gpui::svg()
                                        .size(px(10.))
                                        .flex_none()
                                        .path("icons/sparkles.svg")
                                        .text_color(ui.accent),
                                )
                                .child(format!("Prompt ready · {paste_key} to paste")),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "diff-review-term-close-{col_idx}-{ti}"
                            )))
                            .flex_none()
                            .px(px(4.))
                            .text_size(crate::ui_primitives::BODY)
                            .text_color(ui.muted)
                            .cursor_pointer()
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.text_color(ui.text)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                this.close_review_terminal(col_idx, ti, cx);
                            }))
                            .child("×"),
                    );
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(ti > 0, |d| d.border_l_1().border_color(ui.border))
                    .child(header)
                    .child(div().flex_1().min_h_0().child(rt.terminal.clone()))
            }),
        );
        // Drag handle (top edge): drag up/down to resize the review region.
        let divider = div()
            .id(SharedString::from(format!("diff-review-resize-{col_idx}")))
            .flex_none()
            .h(px(6.))
            .cursor(CursorStyle::ResizeUpDown)
            .bg(ui.border)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.accent)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _w, cx| {
                    let start_h = this
                        .columns
                        .get(col_idx)
                        .map(|c| c.review_height)
                        .unwrap_or(REVIEW_DEFAULT_HEIGHT);
                    this.review_resizing = Some((col_idx, f32::from(ev.position.y), start_h));
                    cx.stop_propagation();
                }),
            );
        let region = div()
            .flex_none()
            .h(px(col.review_height))
            .flex()
            .flex_col()
            .child(divider)
            .child(terminals);
        Some(region.into_any_element())
    }

    /// The Review chip's CLI multi-select popover. Lists the CLIs as toggles and
    /// a Review button that opens one terminal pane per checked CLI under the
    /// branch's worktree.
    pub(super) fn render_review_menu(
        &self,
        col_idx: usize,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let clis = super::super::review_terminal::ReviewCli::all();
        let mut menu = menu_surface(div().id("diff-review-menu"), ui)
            .occlude()
            .absolute()
            // Anchored just below this branch's header.
            .top(px(COL_HEADER_HEIGHT))
            .right(px(6.))
            .w(px(256.))
            .flex()
            .flex_col()
            .p(px(6.))
            .gap(px(2.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.review_menu_open = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .text_size(crate::ui_primitives::LABEL_XS)
                    .text_color(ui.muted)
                    .child("Launch a CLI to review this branch"),
            );
        for (i, cli) in clis.iter().enumerate() {
            let checked = self.review_picks.get(i).copied().unwrap_or(true);
            let label = cli.label();
            menu = menu.child(
                select_item(
                    SharedString::from(format!("diff-review-pick-{i}")),
                    false,
                    ui,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.toggle_review_pick(i, cx);
                }))
                .child(
                    div()
                        .flex_none()
                        .size(px(14.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(ui.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(checked, |d| {
                            d.bg(ui.accent.opacity(0.18)).child(
                                gpui::svg()
                                    .size(px(10.))
                                    .path("icons/check.svg")
                                    .text_color(ui.accent),
                            )
                        }),
                )
                .child(div().flex_1().text_color(ui.text).child(label)),
            );
        }
        menu = menu.child(
            div()
                .id("diff-review-run")
                .mt(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .py(px(5.))
                .rounded(px(5.))
                .bg(ui.accent.opacity(0.15))
                .text_size(crate::ui_primitives::BODY)
                .text_color(ui.accent)
                .cursor_pointer()
                .hover(|s| {
                    let ui = crate::theme::ui_colors();
                    s.bg(ui.accent.opacity(0.25))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.launch_review(col_idx, window, cx);
                }))
                .child("Review"),
        );
        deferred(menu).priority(8).into_any_element()
    }
}
