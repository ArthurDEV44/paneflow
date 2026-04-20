//! Window chrome — title bar and CSD (client-side decoration) helpers.
//!
//! Groups the window-controls-and-resize-edge code that used to live as
//! sibling files at the crate root. Public items are re-exported at the
//! `window_chrome` level so callers can write `crate::window_chrome::X`
//! without reaching into the submodule path.

pub mod csd;
pub mod title_bar;

// Re-exports required by US-019 acceptance criteria; `#[allow(unused_imports)]`
// because `paneflow-app` is a bin crate and the re-exports have no external
// consumer. Callers today still reach the items via `window_chrome::csd::…`
// and `window_chrome::title_bar::…`, but the flat re-export path must stay
// available per the PRD.
#[allow(unused_imports)]
pub use csd::*;
#[allow(unused_imports)]
pub use title_bar::*;
