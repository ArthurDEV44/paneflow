//! US-001: Validate GPUI N-child flexbox
//!
//! Proves that GPUI's Taffy-backed flexbox distributes space correctly
//! across N > 2 children using `flex_basis(relative(ratio))`.
//! This gates the N-ary layout tree architecture (EP-001).

use gpui::{
    div, point, px, relative, size, AvailableSpace, InteractiveElement, ParentElement, Styled,
    TestAppContext,
};

const TOLERANCE: f32 = 2.0;

/// Helper: assert a pixel value is within tolerance of expected.
fn assert_px_eq(actual: gpui::Pixels, expected: f32, label: &str) {
    let diff = (actual.as_f32() - expected).abs();
    assert!(
        diff < TOLERANCE,
        "{label}: expected ~{expected:.1}px, got {:.1}px (diff {diff:.1}px)",
        actual.as_f32()
    );
}

/// AC-1: A flex_row container with 3 children using ratios [0.33, 0.33, 0.34]
/// renders all three children with proportional widths.
#[gpui::test]
fn test_three_children_flex_basis(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();

    let container_w = 900.0_f32;
    let container_h = 600.0_f32;

    cx.draw(
        point(px(0.), px(0.)),
        size(
            AvailableSpace::Definite(px(container_w)),
            AvailableSpace::Definite(px(container_h)),
        ),
        |_, _| {
            div()
                .flex()
                .flex_row()
                .w(px(container_w))
                .h(px(container_h))
                .child(
                    div()
                        .flex_basis(relative(0.33))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "c3-child-0".into()),
                )
                .child(
                    div()
                        .flex_basis(relative(0.33))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "c3-child-1".into()),
                )
                .child(
                    div()
                        .flex_basis(relative(0.34))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "c3-child-2".into()),
                )
        },
    );

    let b0 = cx.debug_bounds("c3-child-0").expect("child-0 not painted");
    let b1 = cx.debug_bounds("c3-child-1").expect("child-1 not painted");
    let b2 = cx.debug_bounds("c3-child-2").expect("child-2 not painted");

    // All three children should be visible
    assert!(b0.size.width > px(0.), "child-0 has zero width");
    assert!(b1.size.width > px(0.), "child-1 has zero width");
    assert!(b2.size.width > px(0.), "child-2 has zero width");

    // Children should fill the container
    let total = b0.size.width + b1.size.width + b2.size.width;
    assert_px_eq(total, container_w, "total width");

    // Each child should be proportional to its ratio
    // With ratios summing to 1.0 and flex_grow=1, Taffy distributes exactly by basis
    assert_px_eq(b0.size.width, container_w * 0.33, "child-0 width");
    assert_px_eq(b1.size.width, container_w * 0.33, "child-1 width");
    assert_px_eq(b2.size.width, container_w * 0.34, "child-2 width");

    // Children should be laid out left-to-right without overlap
    assert!(b1.origin.x >= b0.origin.x + b0.size.width - px(TOLERANCE));
    assert!(b2.origin.x >= b1.origin.x + b1.size.width - px(TOLERANCE));
}

/// AC-2: A flex_row container with 5 children, each ratio = 0.2, renders
/// all five children with equal widths.
#[gpui::test]
fn test_five_children_equal(cx: &mut TestAppContext) {
    const SELECTORS: [&str; 5] = [
        "c5-child-0",
        "c5-child-1",
        "c5-child-2",
        "c5-child-3",
        "c5-child-4",
    ];

    let cx = cx.add_empty_window();

    let container_w = 1000.0_f32;
    let container_h = 600.0_f32;

    cx.draw(
        point(px(0.), px(0.)),
        size(
            AvailableSpace::Definite(px(container_w)),
            AvailableSpace::Definite(px(container_h)),
        ),
        |_, _| {
            let mut container = div()
                .flex()
                .flex_row()
                .w(px(container_w))
                .h(px(container_h));

            for sel in SELECTORS {
                container = container.child(
                    div()
                        .flex_basis(relative(0.2))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| sel.into()),
                );
            }
            container
        },
    );

    let expected_w = container_w / 5.0; // 200px each

    for sel in SELECTORS {
        let bounds = cx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} not painted"));
        assert_px_eq(bounds.size.width, expected_w, sel);
    }
}

/// AC-3: Ratios that don't sum to exactly 1.0 (0.33+0.33+0.33=0.99) must not
/// leave a visible gap or overflow. flex_grow absorbs the remainder.
#[gpui::test]
fn test_imprecise_sum(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();

    let container_w = 900.0_f32;
    let container_h = 600.0_f32;

    cx.draw(
        point(px(0.), px(0.)),
        size(
            AvailableSpace::Definite(px(container_w)),
            AvailableSpace::Definite(px(container_h)),
        ),
        |_, _| {
            div()
                .flex()
                .flex_row()
                .w(px(container_w))
                .h(px(container_h))
                .child(
                    div()
                        .flex_basis(relative(0.33))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "imp-child-0".into()),
                )
                .child(
                    div()
                        .flex_basis(relative(0.33))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "imp-child-1".into()),
                )
                .child(
                    div()
                        .flex_basis(relative(0.33))
                        .flex_grow()
                        .flex_shrink()
                        .h_full()
                        .debug_selector(|| "imp-child-2".into()),
                )
        },
    );

    let b0 = cx.debug_bounds("imp-child-0").expect("child-0 not painted");
    let b1 = cx.debug_bounds("imp-child-1").expect("child-1 not painted");
    let b2 = cx.debug_bounds("imp-child-2").expect("child-2 not painted");

    // No gap: children should fill the entire container width
    let total = b0.size.width + b1.size.width + b2.size.width;
    assert_px_eq(total, container_w, "total width (imprecise sum)");

    // No overflow: no child extends beyond the container
    let rightmost = b2.origin.x + b2.size.width;
    assert!(
        rightmost <= px(container_w) + px(TOLERANCE),
        "rightmost edge {rightmost:?} exceeds container {container_w}px"
    );

    // All children should be approximately equal.
    // flex_grow distributes the 1% remainder (~3px each), so allow slightly wider tolerance.
    let expected_each = container_w / 3.0; // ~300px
    let grow_tolerance = 4.0_f32;
    for (bounds, label) in [
        (b0, "child-0 (imprecise)"),
        (b1, "child-1 (imprecise)"),
        (b2, "child-2 (imprecise)"),
    ] {
        let diff = (bounds.size.width.as_f32() - expected_each).abs();
        assert!(
            diff < grow_tolerance,
            "{label}: expected ~{expected_each:.1}px, got {:.1}px (diff {diff:.1}px)",
            bounds.size.width.as_f32()
        );
    }
}
