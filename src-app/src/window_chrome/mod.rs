//! Window chrome — title bar and CSD (client-side decoration) helpers.
//!
//! Groups the window-controls-and-resize-edge code that used to live as
//! sibling files at the crate root. Callers reach into the submodules
//! directly via `window_chrome::csd::…` and `window_chrome::title_bar::…`.

pub mod csd;
pub mod title_bar;
