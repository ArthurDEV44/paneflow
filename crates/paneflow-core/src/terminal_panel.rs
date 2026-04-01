use std::any::Any;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::panel::{Panel, PanelError, PanelType};

/// A terminal panel that wraps a PTY process.
///
/// The `pty_handle` field is a placeholder (`Option<()>`) until US-006 integrates
/// actual PTY management.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalPanel {
    id: Uuid,
    working_directory: PathBuf,
    custom_title: Option<String>,
    rows: u16,
    cols: u16,
    closed: bool,
    /// Placeholder for the real PTY handle (US-006).
    #[serde(skip)]
    pty_handle: Option<()>,
}

impl TerminalPanel {
    /// Create a new terminal panel.
    ///
    /// `working_directory` is the initial cwd for the shell process.
    /// `custom_title` overrides the default title derived from the directory name.
    pub fn new(working_directory: PathBuf, custom_title: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            working_directory,
            custom_title,
            rows: 24,
            cols: 80,
            closed: false,
            pty_handle: None,
        }
    }

    /// Returns the working directory of this terminal.
    pub fn working_directory(&self) -> &PathBuf {
        &self.working_directory
    }

    /// Returns the custom title, if set.
    pub fn custom_title(&self) -> Option<&str> {
        self.custom_title.as_deref()
    }

    /// Sets a custom title for this terminal panel.
    pub fn set_custom_title(&mut self, title: Option<String>) {
        self.custom_title = title;
    }

    /// Returns the current row count.
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Returns the current column count.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Whether this panel has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Attach a PTY handle (placeholder until US-006).
    pub fn attach_pty(&mut self, handle: Option<()>) {
        self.pty_handle = handle;
    }
}

impl Panel for TerminalPanel {
    fn id(&self) -> Uuid {
        self.id
    }

    fn panel_type(&self) -> PanelType {
        PanelType::Terminal
    }

    fn title(&self) -> &str {
        if let Some(ref t) = self.custom_title {
            return t.as_str();
        }
        // Fall back to the last component of the working directory, or "Terminal".
        self.working_directory
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Terminal")
    }

    fn close(&mut self) {
        self.closed = true;
        self.pty_handle = None;
    }

    fn send_text(&self, _text: &str) -> Result<(), PanelError> {
        if self.closed {
            return Err(PanelError::PanelClosed { panel_id: self.id });
        }
        match self.pty_handle {
            Some(_) => {
                // Real implementation will write to the PTY fd (US-006).
                Ok(())
            }
            None => Err(PanelError::NoPtyHandle { panel_id: self.id }),
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        // When a real PTY is attached, propagate TIOCSWINSZ here (US-006).
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn create_terminal_panel_defaults() {
        let panel = TerminalPanel::new(PathBuf::from("/home/user"), None);
        assert_eq!(panel.panel_type(), PanelType::Terminal);
        assert_eq!(panel.rows(), 24);
        assert_eq!(panel.cols(), 80);
        assert!(!panel.is_closed());
        assert!(panel.pty_handle.is_none());
        assert_eq!(panel.working_directory(), &PathBuf::from("/home/user"));
        assert!(panel.custom_title().is_none());
    }

    #[test]
    fn title_falls_back_to_directory_name() {
        let panel = TerminalPanel::new(PathBuf::from("/home/user/projects"), None);
        assert_eq!(panel.title(), "projects");
    }

    #[test]
    fn title_uses_custom_when_set() {
        let panel = TerminalPanel::new(PathBuf::from("/home/user"), Some("my-shell".to_string()));
        assert_eq!(panel.title(), "my-shell");
    }

    #[test]
    fn title_root_directory_fallback() {
        // PathBuf("/") has no file_name component.
        let panel = TerminalPanel::new(PathBuf::from("/"), None);
        assert_eq!(panel.title(), "Terminal");
    }

    #[test]
    fn close_marks_panel_closed() {
        let mut panel = TerminalPanel::new(PathBuf::from("/tmp"), None);
        assert!(!panel.is_closed());
        panel.close();
        assert!(panel.is_closed());
    }

    #[test]
    fn send_text_without_pty_returns_error() {
        let panel = TerminalPanel::new(PathBuf::from("/tmp"), None);
        let result = panel.send_text("ls\n");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            PanelError::NoPtyHandle { panel_id } => {
                assert_eq!(panel_id, panel.id());
            }
            other => panic!("expected NoPtyHandle, got: {other}"),
        }
    }

    #[test]
    fn send_text_with_pty_succeeds() {
        let mut panel = TerminalPanel::new(PathBuf::from("/tmp"), None);
        panel.attach_pty(Some(()));
        let result = panel.send_text("echo hello\n");
        assert!(result.is_ok());
    }

    #[test]
    fn send_text_on_closed_panel_returns_error() {
        let mut panel = TerminalPanel::new(PathBuf::from("/tmp"), None);
        panel.attach_pty(Some(()));
        panel.close();
        let result = panel.send_text("ls\n");
        assert!(result.is_err());
        match result.unwrap_err() {
            PanelError::PanelClosed { .. } => {}
            other => panic!("expected PanelClosed, got: {other}"),
        }
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut panel = TerminalPanel::new(PathBuf::from("/tmp"), None);
        panel.resize(50, 120);
        assert_eq!(panel.rows(), 50);
        assert_eq!(panel.cols(), 120);
    }

    #[test]
    fn panel_is_object_safe() {
        // Verify we can store TerminalPanel as Box<dyn Panel>.
        let panel = TerminalPanel::new(PathBuf::from("/home/user"), None);
        let boxed: Box<dyn Panel> = Box::new(panel);
        assert_eq!(boxed.panel_type(), PanelType::Terminal);
        assert!(!boxed.id().is_nil());
    }

    #[test]
    fn downcast_via_as_any() {
        let panel = TerminalPanel::new(PathBuf::from("/home/user"), Some("test".to_string()));
        let boxed: Box<dyn Panel> = Box::new(panel);
        let concrete = boxed.as_any().downcast_ref::<TerminalPanel>().unwrap();
        assert_eq!(concrete.custom_title(), Some("test"));
    }

    #[test]
    fn downcast_mut_via_as_any_mut() {
        let panel = TerminalPanel::new(PathBuf::from("/home/user"), None);
        let mut boxed: Box<dyn Panel> = Box::new(panel);
        let concrete = boxed.as_any_mut().downcast_mut::<TerminalPanel>().unwrap();
        concrete.set_custom_title(Some("updated".to_string()));
        assert_eq!(concrete.custom_title(), Some("updated"));
    }
}
