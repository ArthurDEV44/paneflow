// PaneFlow v2 — Native Rust Terminal Multiplexer
// US-001: Native Window with iced Application Shell

use std::collections::HashMap;
use std::sync::Arc;

use iced::widget::{column, container, horizontal_space, row, scrollable, text};
use iced::{Color, Element, Length, Size, Subscription, Task, Theme};
use paneflow_core::split_tree::SplitTree;
use paneflow_core::tab_manager::TabManager;
use paneflow_core::workspace::Workspace;
use paneflow_terminal::bridge::{PtyBridge, TerminalEvent};
use tokio::sync::mpsc;
use uuid::Uuid;

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

#[allow(dead_code)] // Fields used in later waves (US-005, US-008, US-010)
pub struct PaneFlowApp {
    tab_manager: TabManager,
    split_trees: HashMap<Uuid, SplitTree>,
    pty_bridge: Arc<PtyBridge>,
    focused_pane: Option<Uuid>,
    event_rx: Option<mpsc::UnboundedReceiver<TerminalEvent>>,
    event_tx: mpsc::UnboundedSender<TerminalEvent>,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Workspace management
    SelectWorkspace(Uuid),
    CreateWorkspace,
    CloseWorkspace(Uuid),

    // Terminal events
    PtyOutput { pane_id: Uuid, data: Vec<u8> },
    PtyExited { pane_id: Uuid, code: i32 },

    // Keyboard
    KeyboardEvent(iced::keyboard::Key, iced::keyboard::Modifiers),
}

impl PaneFlowApp {
    fn new() -> (Self, Task<Message>) {
        let mut tab_manager = TabManager::new();
        let cwd = std::env::current_dir().unwrap_or_else(|_| "/tmp".into());
        let ws = Workspace::new("default", &cwd);
        let ws_id = ws.id;
        tab_manager.add_workspace(ws);

        let pane_id = Uuid::new_v4();
        let split_trees = HashMap::from([(ws_id, SplitTree::new(pane_id))]);

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let app = Self {
            tab_manager,
            split_trees,
            pty_bridge: Arc::new(PtyBridge::new()),
            focused_pane: Some(pane_id),
            event_rx: Some(event_rx),
            event_tx,
        };

        (app, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectWorkspace(id) => {
                let _ = self.tab_manager.select_workspace(id);
            }
            Message::CreateWorkspace => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| "/tmp".into());
                let ws = Workspace::new("workspace", &cwd);
                let ws_id = ws.id;
                let pane_id = Uuid::new_v4();
                self.tab_manager.add_workspace(ws);
                self.split_trees.insert(ws_id, SplitTree::new(pane_id));
                self.focused_pane = Some(pane_id);
            }
            Message::CloseWorkspace(id) => {
                let _ = self.tab_manager.close_workspace(id);
                self.split_trees.remove(&id);
            }
            Message::PtyOutput { .. } => {
                // Wired in US-005/US-010 (Wave 3-4)
            }
            Message::PtyExited { .. } => {
                // Wired in US-005/US-010 (Wave 3-4)
            }
            Message::KeyboardEvent(key, modifiers) => {
                self.handle_keyboard(key, modifiers);
            }
        }
        Task::none()
    }

    fn handle_keyboard(&mut self, key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;

        let ctrl_shift = modifiers.control() && modifiers.shift();

        match key {
            Key::Character(ref c) if ctrl_shift && c.as_str() == "N" => {
                // Ctrl+Shift+N: new workspace
                let cwd = std::env::current_dir().unwrap_or_else(|_| "/tmp".into());
                let ws = Workspace::new("workspace", &cwd);
                let ws_id = ws.id;
                let pane_id = Uuid::new_v4();
                self.tab_manager.add_workspace(ws);
                self.split_trees.insert(ws_id, SplitTree::new(pane_id));
                self.focused_pane = Some(pane_id);
            }
            Key::Named(Named::Tab) if modifiers.control() && !modifiers.shift() => {
                // Ctrl+Tab: next workspace
                let workspaces = self.tab_manager.workspaces();
                if let Some(selected) = self.tab_manager.selected_id {
                    if let Some(idx) = workspaces.iter().position(|w| w.id == selected) {
                        let next = (idx + 1) % workspaces.len();
                        let next_id = workspaces[next].id;
                        let _ = self.tab_manager.select_workspace(next_id);
                    }
                }
            }
            Key::Named(Named::Tab) if ctrl_shift => {
                // Ctrl+Shift+Tab: previous workspace
                let workspaces = self.tab_manager.workspaces();
                if let Some(selected) = self.tab_manager.selected_id {
                    if let Some(idx) = workspaces.iter().position(|w| w.id == selected) {
                        let prev = if idx == 0 {
                            workspaces.len() - 1
                        } else {
                            idx - 1
                        };
                        let prev_id = workspaces[prev].id;
                        let _ = self.tab_manager.select_workspace(prev_id);
                    }
                }
            }
            _ => {}
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::keyboard::on_key_press(|key, modifiers| Some(Message::KeyboardEvent(key, modifiers)))
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar = self.view_sidebar();
        let main_content = self.view_main_content();
        row![sidebar, main_content].into()
    }

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
                .unwrap_or("~");

            let item_content = column![
                row![
                    text(ws.display_title()).size(14).color(title_color),
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

            let item = container(item_content).style(move |_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(bg)),
                ..Default::default()
            });

            items = items.push(item);
        }

        let sidebar_body = column![header, label, scrollable(items).height(Length::Fill),];

        container(sidebar_body)
            .width(220)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.11, 0.11, 0.13))),
                ..Default::default()
            })
            .into()
    }

    fn view_main_content(&self) -> Element<'_, Message> {
        let has_panes = self
            .tab_manager
            .selected_id
            .and_then(|id| self.split_trees.get(&id))
            .map(|tree| !tree.all_panes().is_empty())
            .unwrap_or(false);

        if has_panes {
            // Terminal panes will be rendered here via WGPU Shader widget (US-004/005)
            container(
                text("Terminal rendering area (WGPU)")
                    .size(14)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.06, 0.08))),
                ..Default::default()
            })
            .into()
        } else {
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
                .style(|_theme: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.06, 0.08))),
                    ..Default::default()
                })
                .into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
