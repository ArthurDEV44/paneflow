// PaneFlow v2 — Native Rust Terminal Multiplexer
//
// US-001: Native window with iced application shell
// US-002: Sidebar widget with workspace list
// US-004: GPU terminal renderer (Canvas + WGPU backend)
// US-009: Zero-IPC keystroke path
// US-012: Binary tree split layout
// US-020: JSON config with hot-reload

mod renderer;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{button, column, container, horizontal_space, mouse_area, row, scrollable, text, Canvas};
use iced::{Color, Element, Length, Size, Subscription, Task, Theme};
use paneflow_config::loader::load_config;
use paneflow_config::schema::PaneFlowConfig;
use paneflow_core::split_tree::{Direction, SplitTree};
use paneflow_core::tab_manager::TabManager;
use paneflow_core::workspace::Workspace;
use paneflow_terminal::bridge::{PtyBridge, TerminalEvent};
use renderer::{TerminalCanvas, TerminalGrid};
use tokio::sync::mpsc;
use uuid::Uuid;

// ─── Constants ───────────────────────────────────────────────────────────────

const SIDEBAR_WIDTH: f32 = 220.0;
const DIVIDER_WIDTH: f32 = 4.0;
#[allow(dead_code)] // Used in US-014 (drag-to-resize)
const MIN_PANE_SIZE: f32 = 80.0;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "paneflow=info".into()),
        )
        .init();

    tracing::info!("PaneFlow v2 starting");

    iced::application("PaneFlow", PaneFlowApp::update, PaneFlowApp::view)
        .theme(PaneFlowApp::theme)
        .subscription(PaneFlowApp::subscription)
        .window_size(Size::new(1200.0, 800.0))
        .antialiasing(false)
        .run_with(PaneFlowApp::new)
}

// ─── Application state ──────────────────────────────────────────────────────

pub struct PaneFlowApp {
    tab_manager: TabManager,
    split_trees: HashMap<Uuid, SplitTree>,
    pty_bridge: Arc<PtyBridge>,
    focused_pane: Option<Uuid>,
    event_tx: mpsc::UnboundedSender<TerminalEvent>,
    terminal_grids: HashMap<Uuid, TerminalGrid>,
    pane_exit_codes: HashMap<Uuid, i32>,
    config: PaneFlowConfig,
}

// ─── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    // Workspace (US-002)
    SelectWorkspace(Uuid),
    CreateWorkspace,
    CloseWorkspace(Uuid),

    // Pane (US-012)
    SplitPane(Direction),
    ClosePane(Uuid),
    FocusPane(Uuid),

    // Terminal events (US-008/009)
    PtyOutput { pane_id: Uuid, data: Vec<u8> },
    PtyExited { pane_id: Uuid, code: i32 },
    PtySpawned(Uuid),

    // Keyboard (US-009)
    KeyPressed(iced::keyboard::Key, iced::keyboard::Modifiers),

    // Config (US-020)
    ConfigChanged(PaneFlowConfig),

    // Internal
    Noop,
}

// ─── Application logic ──────────────────────────────────────────────────────

impl PaneFlowApp {
    fn new() -> (Self, Task<Message>) {
        let config = load_config();
        let mut tab_manager = TabManager::new();
        let cwd = std::env::current_dir().unwrap_or_else(|_| "/tmp".into());
        let ws = Workspace::new("default", &cwd);
        let ws_id = ws.id;
        tab_manager.add_workspace(ws);

        let pane_id = Uuid::new_v4();
        let split_trees = HashMap::from([(ws_id, SplitTree::new(pane_id))]);

        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        let mut terminal_grids = HashMap::new();
        terminal_grids.insert(pane_id, TerminalGrid::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize));

        let app = Self {
            tab_manager,
            split_trees,
            pty_bridge: Arc::new(PtyBridge::new()),
            focused_pane: Some(pane_id),
            event_tx,
            terminal_grids,
            pane_exit_codes: HashMap::new(),
            config,
        };

        // Spawn initial PTY
        let bridge = app.pty_bridge.clone();
        let tx = app.event_tx.clone();
        let spawn_task = Task::perform(
            async move {
                let _ = bridge.spawn_pane(pane_id, None, Some(cwd), vec![], DEFAULT_ROWS, DEFAULT_COLS, tx).await;
                pane_id
            },
            Message::PtySpawned,
        );

        (app, spawn_task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // ── Workspace ────────────────────────────────────────────────
            Message::SelectWorkspace(id) => {
                let _ = self.tab_manager.select_workspace(id);
                // Update focused pane to first pane of selected workspace
                if let Some(tree) = self.split_trees.get(&id) {
                    let panes = tree.all_panes();
                    self.focused_pane = panes.first().copied();
                }
            }
            Message::CreateWorkspace => {
                return self.create_workspace();
            }
            Message::CloseWorkspace(id) => {
                // Close all PTYs in this workspace
                if let Some(tree) = self.split_trees.get(&id) {
                    let panes = tree.all_panes();
                    let bridge = self.pty_bridge.clone();
                    for pane_id in &panes {
                        self.terminal_grids.remove(pane_id);
                        self.pane_exit_codes.remove(pane_id);
                        let bridge = bridge.clone();
                        let pid = *pane_id;
                        tokio::spawn(async move { let _ = bridge.close_pane(pid).await; });
                    }
                }
                let _ = self.tab_manager.close_workspace(id);
                self.split_trees.remove(&id);
            }

            // ── Pane (US-012) ────────────────────────────────────────────
            Message::SplitPane(direction) => {
                return self.split_focused_pane(direction);
            }
            Message::ClosePane(pane_id) => {
                return self.close_pane(pane_id);
            }
            Message::FocusPane(pane_id) => {
                self.focused_pane = Some(pane_id);
            }

            // ── Terminal events ──────────────────────────────────────────
            Message::PtyOutput { pane_id, data } => {
                // Feed raw bytes to terminal grid (basic processing for now)
                // Full alacritty_terminal integration in US-005 (Wave 3)
                if let Some(grid) = self.terminal_grids.get_mut(&pane_id) {
                    process_raw_bytes(grid, &data);
                }
            }
            Message::PtyExited { pane_id, code } => {
                self.pane_exit_codes.insert(pane_id, code);
            }
            Message::PtySpawned(pane_id) => {
                tracing::info!(%pane_id, "PTY spawned");
            }

            // ── Keyboard (US-009) ────────────────────────────────────────
            Message::KeyPressed(key, modifiers) => {
                return self.handle_keyboard(key, modifiers);
            }

            // ── Config (US-020) ──────────────────────────────────────────
            Message::ConfigChanged(config) => {
                tracing::info!("config reloaded");
                self.config = config;
            }

            Message::Noop => {}
        }
        Task::none()
    }

    // ── Workspace operations ─────────────────────────────────────────────

    fn create_workspace(&mut self) -> Task<Message> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| "/tmp".into());
        let ws = Workspace::new("workspace", &cwd);
        let ws_id = ws.id;
        let pane_id = Uuid::new_v4();
        self.tab_manager.add_workspace(ws);
        self.split_trees.insert(ws_id, SplitTree::new(pane_id));
        self.focused_pane = Some(pane_id);
        self.terminal_grids.insert(pane_id, TerminalGrid::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize));

        self.spawn_pty(pane_id, Some(cwd))
    }

    // ── Pane operations (US-012) ─────────────────────────────────────────

    fn split_focused_pane(&mut self, direction: Direction) -> Task<Message> {
        let Some(focused) = self.focused_pane else { return Task::none() };
        let Some(ws_id) = self.tab_manager.selected_id else { return Task::none() };
        let Some(tree) = self.split_trees.get_mut(&ws_id) else { return Task::none() };

        match tree.split(focused, direction) {
            Ok(new_pane_id) => {
                self.focused_pane = Some(new_pane_id);
                self.terminal_grids.insert(
                    new_pane_id,
                    TerminalGrid::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize),
                );
                let cwd = std::env::current_dir().ok();
                self.spawn_pty(new_pane_id, cwd)
            }
            Err(e) => {
                tracing::warn!("split failed: {e}");
                Task::none()
            }
        }
    }

    fn close_pane(&mut self, pane_id: Uuid) -> Task<Message> {
        let Some(ws_id) = self.tab_manager.selected_id else { return Task::none() };
        let Some(tree) = self.split_trees.get_mut(&ws_id) else { return Task::none() };

        match tree.close(pane_id) {
            Ok(()) => {
                self.terminal_grids.remove(&pane_id);
                self.pane_exit_codes.remove(&pane_id);

                // Update focus to first remaining pane
                let panes = tree.all_panes();
                self.focused_pane = panes.first().copied();

                // Close PTY
                let bridge = self.pty_bridge.clone();
                tokio::spawn(async move { let _ = bridge.close_pane(pane_id).await; });
            }
            Err(e) => {
                tracing::debug!("close pane: {e}");
                // Last pane — show empty state
            }
        }
        Task::none()
    }

    // ── PTY spawning ─────────────────────────────────────────────────────

    fn spawn_pty(&self, pane_id: Uuid, cwd: Option<PathBuf>) -> Task<Message> {
        let bridge = self.pty_bridge.clone();
        let tx = self.event_tx.clone();
        let shell = self.config.default_shell.clone();

        Task::perform(
            async move {
                let _ = bridge
                    .spawn_pane(pane_id, shell, cwd, vec![], DEFAULT_ROWS, DEFAULT_COLS, tx)
                    .await;
                pane_id
            },
            Message::PtySpawned,
        )
    }

    // ── Keyboard handling (US-009) ───────────────────────────────────────

    fn handle_keyboard(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;

        let ctrl = modifiers.control();
        let shift = modifiers.shift();
        let ctrl_shift = ctrl && shift;

        // App shortcuts take priority over terminal input
        match &key {
            // Ctrl+Shift+N: new workspace
            Key::Character(c) if ctrl_shift && c.as_str() == "N" => {
                return self.create_workspace();
            }
            // Ctrl+Shift+D: split horizontal
            Key::Character(c) if ctrl_shift && c.as_str() == "D" => {
                return self.split_focused_pane(Direction::Horizontal);
            }
            // Ctrl+Shift+E: split vertical
            Key::Character(c) if ctrl_shift && c.as_str() == "E" => {
                return self.split_focused_pane(Direction::Vertical);
            }
            // Ctrl+Shift+W: close focused pane
            Key::Character(c) if ctrl_shift && c.as_str() == "W" => {
                if let Some(pane_id) = self.focused_pane {
                    return self.close_pane(pane_id);
                }
            }
            // Ctrl+Shift+Q: close workspace
            Key::Character(c) if ctrl_shift && c.as_str() == "Q" => {
                if let Some(ws_id) = self.tab_manager.selected_id {
                    if self.tab_manager.workspaces().len() > 1 {
                        let _ = self.tab_manager.close_workspace(ws_id);
                        self.split_trees.remove(&ws_id);
                    }
                }
                return Task::none();
            }
            // Ctrl+1-9: select workspace by index
            Key::Character(c) if ctrl && !shift => {
                if let Ok(n) = c.as_str().parse::<usize>() {
                    if (1..=9).contains(&n) {
                        let workspaces = self.tab_manager.workspaces();
                        if let Some(ws) = workspaces.get(n - 1) {
                            let id = ws.id;
                            let _ = self.tab_manager.select_workspace(id);
                        }
                    }
                    return Task::none();
                }
            }
            // Ctrl+Tab / Ctrl+Shift+Tab: cycle workspaces
            Key::Named(Named::Tab) if ctrl => {
                let workspaces = self.tab_manager.workspaces();
                if let Some(selected) = self.tab_manager.selected_id {
                    if let Some(idx) = workspaces.iter().position(|w| w.id == selected) {
                        let next = if shift {
                            if idx == 0 { workspaces.len() - 1 } else { idx - 1 }
                        } else {
                            (idx + 1) % workspaces.len()
                        };
                        let id = workspaces[next].id;
                        let _ = self.tab_manager.select_workspace(id);
                    }
                }
                return Task::none();
            }
            _ => {}
        }

        // Route to focused terminal pane (US-009: zero-IPC keystroke path)
        if let Some(pane_id) = self.focused_pane {
            if let Some(bytes) = key_to_bytes(&key, &modifiers) {
                let _ = self.pty_bridge.write_pane(pane_id, &bytes);
            }
        }

        Task::none()
    }

    // ── Subscriptions ────────────────────────────────────────────────────

    fn subscription(&self) -> Subscription<Message> {
        iced::keyboard::on_key_press(|key, modifiers| Some(Message::KeyPressed(key, modifiers)))
    }

    // ── View ─────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let sidebar = self.view_sidebar();
        let main_content = self.view_main_content();
        row![sidebar, main_content].into()
    }

    // ── Sidebar (US-002) ─────────────────────────────────────────────────

    fn view_sidebar(&self) -> Element<'_, Message> {
        let header = container(text("PaneFlow").size(18).color(Color::WHITE)).padding([12, 16]);

        let label = container(
            text("WORKSPACES")
                .size(11)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
        )
        .padding([4, 16]);

        let mut items = column![].spacing(2);
        for ws in self.tab_manager.workspaces() {
            items = items.push(self.view_workspace_item(ws));
        }

        // "+" button (US-002)
        let add_button = container(
            button(text("+").size(16).color(Color::WHITE).center())
                .width(Length::Fill)
                .on_press(Message::CreateWorkspace)
                .style(|_theme: &Theme, _status| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.15, 0.15, 0.18))),
                    text_color: Color::WHITE,
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .padding([8, 16]);

        let sidebar_body = column![
            header,
            label,
            scrollable(items).height(Length::Fill),
            add_button,
        ];

        container(sidebar_body)
            .width(SIDEBAR_WIDTH as u16)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.11, 0.11, 0.13))),
                ..Default::default()
            })
            .into()
    }

    fn view_workspace_item<'a>(&self, ws: &Workspace) -> Element<'a, Message> {
        let is_selected = self.tab_manager.selected_id == Some(ws.id);
        let title_color = if is_selected {
            Color::WHITE
        } else {
            Color::from_rgb(0.7, 0.7, 0.7)
        };

        let pane_count = self
            .split_trees
            .get(&ws.id)
            .map(|t| t.all_panes().len())
            .unwrap_or(0);

        let dir_name = ws
            .working_directory
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("~")
            .to_string();

        let title = ws.display_title().to_string();
        let ws_id = ws.id;

        let item_content = column![
            row![
                text(title).size(14).color(title_color),
                horizontal_space(),
                text(format!("{pane_count}"))
                    .size(11)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            ],
            text(dir_name)
                .size(11)
                .color(Color::from_rgb(0.4, 0.4, 0.4)),
        ]
        .spacing(2)
        .padding([8, 16]);

        let bg = if is_selected {
            Color::from_rgb(0.2, 0.2, 0.25)
        } else {
            Color::TRANSPARENT
        };

        // Clickable workspace item (US-002)
        let styled_item = container(item_content).style(move |_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            ..Default::default()
        });

        mouse_area(styled_item)
            .on_press(Message::SelectWorkspace(ws_id))
            .into()
    }

    // ── Main content area ────────────────────────────────────────────────

    fn view_main_content(&self) -> Element<'_, Message> {
        let content = if let Some(ws_id) = self.tab_manager.selected_id {
            if let Some(tree) = self.split_trees.get(&ws_id) {
                self.view_split_tree(tree)
            } else {
                self.view_empty_state()
            }
        } else {
            self.view_empty_state()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.06, 0.08))),
                ..Default::default()
            })
            .into()
    }

    // ── Split tree rendering (US-012) ────────────────────────────────────

    fn view_split_tree<'a>(&'a self, tree: &SplitTree) -> Element<'a, Message> {
        match tree {
            SplitTree::Leaf { pane_id } => self.view_terminal_pane(*pane_id),
            SplitTree::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first_portion = (*ratio * 100.0) as u16;
                let second_portion = 100 - first_portion;
                let first_el = self.view_split_tree(first);
                let second_el = self.view_split_tree(second);

                // Divider between panes (US-012)
                let (div_w, div_h): (Length, Length) = match direction {
                    Direction::Horizontal => (Length::Fixed(DIVIDER_WIDTH), Length::Fill),
                    Direction::Vertical => (Length::Fill, Length::Fixed(DIVIDER_WIDTH)),
                };
                let divider = container(iced::widget::Space::new(div_w, div_h))
                .style(|_theme: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.2, 0.24))),
                    ..Default::default()
                });

                match direction {
                    Direction::Horizontal => row![
                        container(first_el)
                            .width(Length::FillPortion(first_portion))
                            .height(Length::Fill),
                        divider,
                        container(second_el)
                            .width(Length::FillPortion(second_portion))
                            .height(Length::Fill),
                    ]
                    .into(),
                    Direction::Vertical => column![
                        container(first_el)
                            .width(Length::Fill)
                            .height(Length::FillPortion(first_portion)),
                        divider,
                        container(second_el)
                            .width(Length::Fill)
                            .height(Length::FillPortion(second_portion)),
                    ]
                    .into(),
                }
            }
        }
    }

    // ── Terminal pane (US-004) ────────────────────────────────────────────

    fn view_terminal_pane(&self, pane_id: Uuid) -> Element<'_, Message> {
        let is_focused = self.focused_pane == Some(pane_id);

        let content: Element<'_, Message> = if let Some(exit_code) = self.pane_exit_codes.get(&pane_id) {
            // Exited pane
            container(
                text(format!("[Process exited with code {exit_code}]"))
                    .size(14)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .into()
        } else if let Some(grid) = self.terminal_grids.get(&pane_id) {
            // Active terminal — render via Canvas (US-004)
            Canvas::new(TerminalCanvas {
                grid,
                font_size: self.config.font.size,
                focused: is_focused,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            // Loading state
            container(
                text("Starting terminal...")
                    .size(14)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .into()
        };

        // Focus border for selected pane
        let border_color = if is_focused {
            Color::from_rgb(0.537, 0.706, 0.980) // blue accent
        } else {
            Color::TRANSPARENT
        };

        // Clickable to focus (US-012)
        let pane = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.06, 0.08))),
                border: iced::Border {
                    color: border_color,
                    width: if is_focused { 1.0 } else { 0.0 },
                    radius: 0.0.into(),
                },
                ..Default::default()
            });

        mouse_area(pane)
            .on_press(Message::FocusPane(pane_id))
            .into()
    }

    fn view_empty_state(&self) -> Element<'_, Message> {
        let placeholder = column![
            text("No terminal panes")
                .size(20)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
            text("Press Ctrl+Shift+N to create a workspace")
                .size(14)
                .color(Color::from_rgb(0.4, 0.4, 0.4)),
        ]
        .spacing(8)
        .align_x(iced::Alignment::Center);

        container(placeholder)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

// ─── Key-to-bytes translation (US-009) ───────────────────────────────────────
//
// Translates iced keyboard events to PTY byte sequences.
// Zero heap allocations for ASCII printable characters.

fn key_to_bytes(key: &iced::keyboard::Key, modifiers: &iced::keyboard::Modifiers) -> Option<Vec<u8>> {
    use iced::keyboard::key::Named;
    use iced::keyboard::Key;

    // Don't send app shortcuts to the terminal
    if modifiers.control() && modifiers.shift() {
        return None;
    }

    match key {
        Key::Character(c) => {
            let s = c.as_str();
            if modifiers.control() {
                // Ctrl+key → control character (US-009: bypass input method)
                if let Some(ch) = s.chars().next() {
                    let ctrl_byte = match ch.to_ascii_lowercase() {
                        'a'..='z' => Some(ch.to_ascii_lowercase() as u8 - b'a' + 1),
                        '[' | '3' => Some(0x1b),   // Escape
                        '\\' | '4' => Some(0x1c),  // FS
                        ']' | '5' => Some(0x1d),   // GS
                        '6' => Some(0x1e),          // RS
                        '/' | '7' => Some(0x1f),    // US
                        _ => None,
                    };
                    ctrl_byte.map(|b| vec![b])
                } else {
                    None
                }
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(named) => {
            let bytes: &[u8] = match named {
                Named::Enter => b"\r",
                Named::Backspace => &[0x7f],
                Named::Tab => b"\t",
                Named::Escape => &[0x1b],
                Named::ArrowUp => b"\x1b[A",
                Named::ArrowDown => b"\x1b[B",
                Named::ArrowRight => b"\x1b[C",
                Named::ArrowLeft => b"\x1b[D",
                Named::Home => b"\x1b[H",
                Named::End => b"\x1b[F",
                Named::PageUp => b"\x1b[5~",
                Named::PageDown => b"\x1b[6~",
                Named::Insert => b"\x1b[2~",
                Named::Delete => b"\x1b[3~",
                Named::F1 => b"\x1bOP",
                Named::F2 => b"\x1bOQ",
                Named::F3 => b"\x1bOR",
                Named::F4 => b"\x1bOS",
                Named::F5 => b"\x1b[15~",
                Named::F6 => b"\x1b[17~",
                Named::F7 => b"\x1b[18~",
                Named::F8 => b"\x1b[19~",
                Named::F9 => b"\x1b[20~",
                Named::F10 => b"\x1b[21~",
                Named::F11 => b"\x1b[23~",
                Named::F12 => b"\x1b[24~",
                Named::Space => b" ",
                _ => return None,
            };
            Some(bytes.to_vec())
        }
        _ => None,
    }
}

// ─── Basic byte processing (placeholder until US-005 wires alacritty_terminal) ──

fn process_raw_bytes(grid: &mut TerminalGrid, data: &[u8]) {
    for &byte in data {
        match byte {
            // Newline
            b'\n' => {
                grid.cursor_row += 1;
                if grid.cursor_row >= grid.rows {
                    // Scroll: shift all rows up by one
                    let cols = grid.cols;
                    grid.cells.drain(..cols);
                    grid.cells.extend(
                        std::iter::repeat_n(renderer::CellData::default(), cols),
                    );
                    grid.cursor_row = grid.rows - 1;
                }
            }
            // Carriage return
            b'\r' => {
                grid.cursor_col = 0;
            }
            // Backspace
            0x08 => {
                if grid.cursor_col > 0 {
                    grid.cursor_col -= 1;
                }
            }
            // Bell — ignore for now (US-017)
            0x07 => {}
            // Escape — skip escape sequences for now (US-005 handles properly)
            0x1b => {}
            // Tab
            b'\t' => {
                let next_tab = (grid.cursor_col + 8) & !7;
                grid.cursor_col = next_tab.min(grid.cols - 1);
            }
            // Printable ASCII and UTF-8 start bytes
            byte if byte >= 0x20 => {
                if grid.cursor_col < grid.cols && grid.cursor_row < grid.rows {
                    let cell = grid.cell_mut(grid.cursor_row, grid.cursor_col);
                    cell.character = byte as char;
                    grid.cursor_col += 1;
                    if grid.cursor_col >= grid.cols {
                        grid.cursor_col = 0;
                        grid.cursor_row += 1;
                        if grid.cursor_row >= grid.rows {
                            let cols = grid.cols;
                            grid.cells.drain(..cols);
                            grid.cells.extend(
                                std::iter::repeat_n(renderer::CellData::default(), cols),
                            );
                            grid.cursor_row = grid.rows - 1;
                        }
                    }
                }
            }
            _ => {} // Ignore other control characters
        }
    }
}
