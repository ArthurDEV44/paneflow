# Rendering golden-frame corpus (US-002)

Committed goldens for the Window-free rendering net. Each `<fixture>.txt` is the
deterministic `LayoutState::golden_repr()` of one fixture in
`golden_frame_tests` (`../mod.rs`), produced with **no Window / App / GPU /
display** — a fixed grid, theme (`one_dark`), font, and cell dimensions.

Floats are rendered at fixed precision and no GPUI `Debug` impl is relied upon,
so the bytes are reproducible across platforms and the diff is human-reviewable.

## Regenerate (after an intentional rendering change)

```bash
PANEFLOW_BLESS_GOLDEN=1 cargo test -p paneflow-app golden_frame
```

Then review the `git diff` on this directory: every changed line must map to the
rendering change you made. Commit the regenerated goldens with that change.

## Assert (default)

```bash
cargo test -p paneflow-app golden_frame
```

A drift fails the test with the fixture name and the regenerate command.
