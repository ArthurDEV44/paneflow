# Rosetta QA runbook

Use this checklist for Rosetta changes before moving a story past `IN_REVIEW`.

## Automated gates

Run from the repository root:

```powershell
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

For focused iteration:

```powershell
cargo test -p paneflow-app app::rosetta -- --nocapture
cargo test -p paneflow-config rosetta -- --nocapture
```

## Manual matrix

Record each target as `pass`, `fail`, or `not verified`, with date and OS.

| Target | Result | Notes |
| --- | --- | --- |
| Windows 10 or 11 x64 | not verified | |
| macOS Apple Silicon | not verified | |
| macOS Intel | not verified | |
| Linux X11 | not verified | |
| Linux Wayland | not verified | |

Do not infer Linux Wayland coverage from X11, and do not infer Windows ARM64
coverage from x64.

## Scenarios

1. CLI mode, waiting agent:
   - Start Paneflow in CLI mode.
   - Create or simulate one `WaitingForInput` agent session with a live pane.
   - Confirm Rosetta appears top-center in the main content area, not in the
     sidebar or titlebar.
   - Click the row or press `Enter` in the expanded panel.
   - Confirm focus lands on the original workspace pane.

2. Agents mode thread:
   - Open Agents mode with one running or waiting terminal thread.
   - Confirm Rosetta includes the thread title and project/chat context.
   - Activate the row.
   - Confirm Paneflow stays in Agents mode and selects the correct thread.

3. Expanded keyboard behavior:
   - Open Rosetta expanded with at least three rows.
   - Press `Down` and `Up`.
   - Confirm selection moves one row at a time without terminal input leaking.
   - Press `Enter` on a navigable row.
   - Confirm the selected target focuses.
   - Reopen Rosetta, press `Esc`, and confirm it collapses with focus restored
     to the prior terminal or thread.

4. Narrow width:
   - Resize to 900 px wide or below.
   - Confirm the compact card stays within viewport minus 32 px.
   - Confirm long workspace/thread names and messages truncate without overlap.

5. Closed target fallback:
   - Open Rosetta expanded on a navigable row.
   - Close the target pane or thread before activating the row.
   - Press `Enter`.
   - Confirm there is no panic and Rosetta reports `Target unavailable` or
     `No pane`, without focusing an unrelated target.

6. Untrusted text:
   - Use an agent message containing Markdown, ANSI color escapes, URL-like
     text, bidi/zero-width controls, and button-looking copy.
   - Confirm Rosetta displays inert plain text only.
   - Confirm no `Approve` or `Deny` control appears unless typed action
     metadata is complete and internally generated.

7. Settings:
   - In Settings -> AI Agent -> Rosetta, disable Rosetta.
   - Confirm the card disappears while sidebar status, Attention Queue, and OS
     notification behavior are unchanged.
   - Re-enable Rosetta and toggle `Show running agents`.
   - Restart Paneflow and confirm the choices persisted.
   - Put invalid values in `paneflow.json` for `rosetta_enabled` or
     `rosetta_show_passive`.
   - Confirm the app does not panic and falls back through the config resolver.

8. Diff/Review surface:
   - Switch to Diff mode.
   - Confirm Rosetta is gated off there for v1. Rosetta currently supports CLI
     and Agents mode only; the PRD Review-mode question remains documented
     until a distinct Review app mode exists.
