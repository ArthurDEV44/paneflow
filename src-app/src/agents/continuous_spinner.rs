//! Continuous rotating spinner element.
//!
//! Mirrors the rotation pattern used by `window_chrome::title_bar`
//! for the self-update pill, so the visual cadence of background
//! activity stays consistent across the app.

use gpui::{
    Animation, AnimationExt, ElementId, Hsla, IntoElement, Pixels, Styled, Transformation,
    percentage, svg,
};
use std::time::Duration;

/// Render a `loader-circle.svg` that rotates continuously (one full
/// turn per second). Call sites pass a stable `id` so GPUI threads the
/// animation state across repaints.
pub(crate) fn continuous_spinner(
    id: impl Into<ElementId>,
    size: Pixels,
    color: Hsla,
) -> impl IntoElement {
    svg()
        .size(size)
        .flex_none()
        .path("icons/loader-circle.svg")
        .text_color(color)
        .with_animation(
            id.into(),
            Animation::new(Duration::from_secs(1)).repeat(),
            |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
        )
}
