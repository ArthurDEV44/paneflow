//! Alacritty event-listener bridge + grid dimensions adapter.
//!
//! `ZedListener` carries `AlacEvent`s from the VTE thread to the GPUI main
//! thread via a `futures::mpsc` channel. `SpikeTermSize` adapts our
//! `(columns, screen_lines)` pair to alacritty's `Dimensions` trait.
//!
//! Extracted from `terminal.rs` per US-011 of the src-app refactor PRD.

use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use futures::channel::mpsc::UnboundedSender;

pub struct SpikeTermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl Dimensions for SpikeTermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone)]
pub struct ZedListener(pub(super) UnboundedSender<AlacEvent>);

impl EventListener for ZedListener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.0.unbounded_send(event);
    }
}
