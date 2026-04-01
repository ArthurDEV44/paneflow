use std::any::Any;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Errors that can occur when interacting with a panel.
#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error("no PTY handle attached to panel {panel_id}")]
    NoPtyHandle { panel_id: Uuid },

    #[error("panel {panel_id} is closed")]
    PanelClosed { panel_id: Uuid },

    #[error("{0}")]
    Other(String),
}

/// The kind of content a panel displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelType {
    Terminal,
    Browser,
    Markdown,
}

/// A single panel inside a tab.
///
/// The trait is object-safe so it can be stored as `Box<dyn Panel>`.
pub trait Panel: Send + Sync {
    /// Unique identifier for this panel.
    fn id(&self) -> Uuid;

    /// The kind of content this panel holds.
    fn panel_type(&self) -> PanelType;

    /// Human-readable title (tab label, status bar, etc.).
    fn title(&self) -> &str;

    /// Gracefully close the panel and release resources.
    fn close(&mut self);

    /// Send text to the panel's underlying process (e.g. a shell).
    fn send_text(&self, text: &str) -> Result<(), PanelError>;

    /// Resize the panel to the given dimensions.
    fn resize(&mut self, rows: u16, cols: u16);

    /// Downcast support — returns `&dyn Any` for the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Downcast support (mutable) — returns `&mut dyn Any` for the concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
