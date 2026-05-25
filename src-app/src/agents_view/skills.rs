//! Skills browser: scan the well-known skills directories
//! (`~/.claude/skills`, `~/.codex/skills`, `~/.agents/skills`),
//! parse each `<dir>/SKILL.md` frontmatter, and render the result as
//! a tabbed grid. One tab per source directory; cards inside each
//! tab share the visual language of the Agents welcome page
//! (`ui.surface` background, neutral border, 10 px radius).

use crate::PaneFlowApp;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px,
};
use std::fs;
use std::path::{Path, PathBuf};

/// Which source directory the Skills page is currently filtered to.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SkillsTab {
    #[default]
    Claude,
    Codex,
    Agents,
}

impl SkillsTab {
    const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Agents => "Agents",
        }
    }

    /// The source tag we stamp on each `SkillEntry` during discovery
    /// (matches the parent directory under `~/`).
    const fn source_tag(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Codex => ".codex",
            Self::Agents => ".agents",
        }
    }
}

/// A single skill resolved from a `SKILL.md` frontmatter block.
#[derive(Clone, Debug)]
struct SkillEntry {
    name: String,
    description: String,
    /// Agent dir tag (e.g. ".claude") -- used to bucket entries per tab.
    source: String,
    #[allow(dead_code)]
    path: PathBuf,
}

const FRONTMATTER_SCAN_BYTES: usize = 16 * 1024;
const DESCRIPTION_MAX_CHARS: usize = 180;
/// Fixed card width drives the grid layout via `flex_wrap`: at the
/// page's `max_w(960)` we get a clean 3-column grid on wide windows
/// and naturally collapses to 2 / 1 column on narrower ones.
const CARD_WIDTH_PX: f32 = 300.0;

pub(crate) fn render_skills_page(
    active_tab: SkillsTab,
    copied_name: Option<String>,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let ui = crate::theme::ui_colors();
    let theme = crate::theme::active_theme();
    let all = discover_skills();
    let filtered: Vec<SkillEntry> = all
        .into_iter()
        .filter(|s| s.source == active_tab.source_tag())
        .collect();

    let body: AnyElement = if filtered.is_empty() {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .py(px(48.))
            .gap(px(6.))
            .child(
                div()
                    .text_color(ui.text)
                    .text_size(px(13.))
                    .child("No skills found"),
            )
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(12.))
                    .child(SharedString::from(format!(
                        "Drop skill folders under ~/{}{}skills.",
                        active_tab.source_tag(),
                        std::path::MAIN_SEPARATOR,
                    ))),
            )
            .into_any_element()
    } else {
        let copied_for_cards = copied_name.clone();
        let cards = filtered.into_iter().map(|s| {
            let is_copied = copied_for_cards
                .as_deref()
                .is_some_and(|c| c == s.name.as_str());
            render_skill_card(s, is_copied, ui, cx)
        });
        // `flex_wrap` + fixed card width gives a clean grid that
        // collapses gracefully on narrow windows.
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(12.))
            .w_full()
            .children(cards)
            .into_any_element()
    };

    div()
        .id("agents-skills-page")
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        // Same panel bg as the Connect / welcome page so the cards'
        // `ui.surface` background pops the way it does there.
        .bg(theme.title_bar_background)
        .text_color(ui.text)
        .px(px(20.))
        .py(px(16.))
        .gap(px(14.))
        .child(
            div()
                .w_full()
                .max_w(px(960.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(ui.text)
                        .child("Skills"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("Skills discovered across your agent home directories."),
                ),
        )
        .child(
            div()
                .w_full()
                .max_w(px(960.))
                .child(render_tab_bar(active_tab, ui, cx)),
        )
        .child(div().w_full().max_w(px(960.)).child(body))
        .into_any_element()
}

fn render_tab_bar(
    active: SkillsTab,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let tab = |this_tab: SkillsTab| {
        let is_active = this_tab == active;
        let label = this_tab.label();
        let id: SharedString = format!("agents-skills-tab-{}", label.to_lowercase()).into();
        let mut row = div()
            .id(id)
            .px(px(12.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::NORMAL);
        if is_active {
            row = row.bg(ui.subtle).text_color(ui.text);
        } else {
            row = row.text_color(ui.muted).hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle).text_color(ui.text)
            });
        }
        row.on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
            if this.agents_skills_tab != this_tab {
                this.agents_skills_tab = this_tab;
                cx.notify();
            }
        }))
        .child(label)
        .into_any_element()
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .child(tab(SkillsTab::Claude))
        .child(tab(SkillsTab::Codex))
        .child(tab(SkillsTab::Agents))
        .into_any_element()
}

fn render_skill_card(
    skill: SkillEntry,
    is_copied: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    // Same chrome as the Agents welcome card (`ui.surface` bg, neutral
    // border, rounded(10)) so the two pages read as one design language.
    // Fixed `w(CARD_WIDTH_PX)` drives the parent's `flex_wrap` grid.
    let name_for_copy = skill.name.clone();
    let name_for_mark = skill.name.clone();
    let copy_id: SharedString = format!("skill-copy-{}-{}", skill.source, skill.name).into();
    let copy_label = if is_copied { "Copied" } else { "Copy" };
    let copy_text_color = if is_copied { ui.text } else { ui.muted };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w(px(CARD_WIDTH_PX))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(ui.surface)
        .border_1()
        .border_color(ui.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .w_full()
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(12.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(ui.text)
                        .child(SharedString::from(skill.name.clone())),
                )
                .child(
                    div()
                        .id(copy_id)
                        .flex_none()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_color(copy_text_color)
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle).text_color(ui.text)
                        })
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                name_for_copy.clone(),
                            ));
                            this.mark_skill_copied(name_for_mark.clone(), cx);
                        }))
                        .child(copy_label),
                ),
        )
        .when(!skill.description.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .child(SharedString::from(skill.description)),
            )
        })
        .into_any_element()
}

fn discover_skills() -> Vec<SkillEntry> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let sources: [(&str, PathBuf); 3] = [
        (".claude", home.join(".claude/skills")),
        (".codex", home.join(".codex/skills")),
        (".agents", home.join(".agents/skills")),
    ];
    let mut out: Vec<SkillEntry> = Vec::new();
    for (label, dir) in sources {
        if !dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let (name, description) = match read_frontmatter(&skill_md) {
                Some(pair) => pair,
                None => continue,
            };
            let name = if name.is_empty() {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("(unnamed)")
                    .to_string()
            } else {
                name
            };
            out.push(SkillEntry {
                name,
                description: ellipsize(description, DESCRIPTION_MAX_CHARS),
                source: label.to_string(),
                path,
            });
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

fn read_frontmatter(path: &Path) -> Option<(String, String)> {
    let raw = read_capped(path, FRONTMATTER_SCAN_BYTES).ok()?;
    let mut lines = raw.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = String::new();
    let mut description = String::new();
    for line in lines {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.trim() == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            name = strip_yaml_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            description = strip_yaml_value(rest);
        }
    }
    Some((name, description))
}

fn strip_yaml_value(raw: &str) -> String {
    let v = raw.trim();
    if v.len() >= 2
        && let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
    {
        return inner.to_string();
    }
    if v.len() >= 2
        && let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
    {
        return inner.to_string();
    }
    v.to_string()
}

fn read_capped(path: &Path, max_bytes: usize) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut buf = vec![0u8; max_bytes];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn ellipsize(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut cut: String = text.chars().take(max_chars).collect();
    if let Some(last_space) = cut.rfind(' ') {
        cut.truncate(last_space);
    }
    cut.push('\u{2026}');
    cut
}
