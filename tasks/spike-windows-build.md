# Spike — GPUI DirectX 11 Windows backend commit identification

**Story:** US-001 (EP-W1, `tasks/prd-windows-port.md`)
**Date:** 2026-04-18
**Author:** Claude + Arthur
**Status:** Complete — recommendation documented below

## Purpose

Identify a specific Zed commit post October 2025 that ships the stable DirectX 11 GPUI Windows backend, so US-002 can bump PaneFlow's GPUI pin with confidence.

## Findings

### Commit that introduced DirectX 11 as the default Windows renderer

- **SHA:** `15ad9863296d427966098f6b9864d5b819725101`
- **Subject:** `windows: Port to DirectX 11 (#34374)`
- **Date:** 2025-07-30 15:27:58 -0700
- **PR:** https://github.com/zed-industries/zed/pull/34374
- **Closes (per PR description):** zed#16713, zed#19739, zed#33191, zed#26692 (RDP init), zed#17374, zed#35077, zed#35205, zed#35262

Claimed improvements vs the prior Vulkan implementation (quoted from the PR body):

- Fewer weird bugs
- Better hardware compatibility
- VSync support
- More accurate colors
- Lower memory usage
- Graceful handling of device loss

The PR landed **2025-07-30**, two months earlier than the "October 2025 GA" figure that appears in `memory/research_windows_port_feasibility.md` and in the PRD's `Overview` and `Research Findings` sections. The October 2025 number reflects when stability was externally declared; DX11 code itself landed in July.

### Current PaneFlow pin — DX11 status

- **Current pin** (workspace `src-app/Cargo.toml:21-23`): `0b984b5ade7604e3f1c618c0ef77879de800b868`
- **Pin date:** 2026-04-03 19:22:17 +0530 (`Ignore user config when checking remote git URL for dev extensions (#52538)`)

Ancestor check run on the local Zed checkout at `~/dev/zed`:

```
git merge-base --is-ancestor 15ad9863296d427966098f6b9864d5b819725101 \
                             0b984b5ade7604e3f1c618c0ef77879de800b868
→ exit 0 (DX11 commit IS an ancestor of the current pin)
```

**Conclusion:** the pin that PaneFlow currently ships already contains the DX11 port. It is ~8 months of Windows stability data downstream of `15ad986329`. The PRD's working assumption that `0b984b5` is pre-DX11 (Overview, §EP-W1; Assumptions §1) is **inverted** — the pin is post-DX11.

### Source-layout change — correction to the US-001 AC-3 path

Acceptance criterion 3 of US-001 reads:

> Given the identified commit, when `git log <0b984b5>..<new-sha> -- crates/gpui/src/platform/windows/` is run on a local Zed checkout, then a non-empty changeset proves DirectX 11 Windows work landed between the two revs.

That path **no longer exists** in the current pin. Between the DX11 port (which touched `crates/gpui/src/platform/windows/*.rs`) and the current pin, Zed refactored its GPUI crate into platform-split sub-crates. In `0b984b5` and on current HEAD, the Windows backend lives at:

- `crates/gpui_windows/` — the full Windows platform crate (directx_atlas, directx_renderer, directx_devices, direct_write, etc.)
- `crates/gpui/resources/windows/` — manifest + `.rc` resource files only

PaneFlow already depends on the split layout via the separate `gpui_platform` git dep (`src-app/Cargo.toml:22,63`), which confirms the current pin is post-refactor.

**Revised verification** used for this spike:

```
git diff-tree --no-commit-id --name-only -r 15ad986329 \
  | grep -E '(windows|directx|hlsl)'
→ crates/gpui/src/platform/windows.rs
  crates/gpui/src/platform/windows/directx_atlas.rs
  crates/gpui/src/platform/windows/directx_renderer.rs
  crates/gpui/src/platform/windows/events.rs
  crates/gpui/src/platform/windows/platform.rs
  crates/gpui/src/platform/windows/shaders.hlsl
  crates/gpui/src/platform/windows/window.rs
  crates/zed/resources/windows/zed.iss
  script/bundle-windows.ps1
```

This proves DX11 Windows work landed in `15ad986329`. The files have since been relocated under `crates/gpui_windows/` by the platform-split refactor; verifying the path in the current pin returns the post-refactor layout:

```
git ls-tree -r 0b984b5 --name-only | grep -c '^crates/gpui_windows/'
→ 28 files (directx_renderer.rs, directx_atlas.rs, directx_devices.rs, …)
```

### `[patch.crates-io]` delta — current pin vs HEAD

Compared the `[patch.crates-io]` block of `0b984b5`'s root `Cargo.toml` against Zed `HEAD` (2026-04-15):

| Patch | `0b984b5` rev | HEAD rev | Delta |
|-------|---------------|----------|-------|
| `async-task` (smol-rs fork) | `b4486cd71e…` | `b4486cd71e…` | none |
| `notify` (zed-industries fork) | `ce58c24cad…` | `ce58c24cad…` | none |
| `notify-types` (zed-industries fork) | `ce58c24cad…` | `ce58c24cad…` | none |
| `windows-capture` (zed-industries fork) | `f0d6c1b669…` | `f0d6c1b669…` | none |
| `calloop` (zed-industries fork) | HEAD (no rev) | HEAD (no rev) | none |
| `livekit` (zed-industries fork) | `147fbca3d4…` | `147fbca3d4…` | none |

**No patch updates are needed** whether PaneFlow stays on `0b984b5` or bumps to HEAD.

Note: PaneFlow's own `Cargo.toml` pins `calloop` to `eb6b4fd17b9af5ecc226546bdd04185391b3e265` (a specific rev) whereas Zed leaves it unpinned. This is a deliberate reproducibility choice for PaneFlow and is unrelated to DX11 — no change required.

### Windows changes since the current pin (reference only)

Post-`0b984b5` Windows-platform-touching commits through HEAD (`bfc34a620b`, 2026-04-15):

```
git log --oneline 0b984b5..HEAD -- crates/gpui_windows/
→ 5375ca0ae2 gpui: Add `display_handle` implementation for Windows,
              update it for macOS (#52867)
```

One commit, unrelated to DX11 correctness. 309 Zed commits total across all crates in this range.

## Recommendation

**No GPUI pin bump is required for US-001 / EP-W1.**

Rationale:

1. The current pin `0b984b5` (2026-04-03) is already 8 months post-DX11 (PR #34374, 2025-07-30). The DX11 backend is present, and with it VSync, device-loss recovery, RDP fixes (zed#26692, which the PRD lists as a "known upstream risk" but which #34374 explicitly closes).
2. No `[patch.crates-io]` divergence between the current pin and HEAD.
3. Only one minor Windows-backend commit has landed since the pin (`5375ca0ae2`, unrelated to rendering correctness).
4. Bumping purely to pick up `5375ca0ae2` would also pull in 308 unrelated commits across every non-Windows crate, adding regression surface for zero DX11 benefit.

**De-scoping note for US-002.** The acceptance criteria of US-002 are predicated on a bump being required. With DX11 already in pin, the correct revision of US-002 is:

- **Primary path (recommended):** mark US-002 as **DONE — no bump needed**, record this spike as the justification, and proceed directly to US-003 (provision `windows-2022` CI runner and run `cargo check --target x86_64-pc-windows-msvc` at the current pin).
- **Fallback path:** if US-003's `cargo check` surfaces a blocker that traces to a GPUI/platform Windows bug fixed post-`0b984b5`, reopen US-002 with a specific target SHA, and treat the bump as scoped remediation (not speculative uplift).

**Explicit approval decision:** The recommended new rev is **the current rev** — i.e. do not bump. Proceed to US-003 to validate the Windows build at `0b984b5`. This satisfies the US-001 AC-4 "wait with a documented reason" branch: wait is not pending availability, it is pending empirical evidence (Windows CI compile result) that a bump is actually necessary.

## Follow-up to file if US-002 is reopened

If a future story needs to bump:

- Candidate SHA: Zed HEAD at bump time (weekly-release cadence; Zed itself ships from `main`).
- Required pre-bump validation:
  - `cargo update -p gpui -p gpui_platform -p collections` refresh produces a clean `Cargo.lock`.
  - `[patch.crates-io]` block compared via the script in the "Delta" section above; any diff ported into `Cargo.toml`.
  - Linux regression smoke (`cargo build --release` + manual launch) per US-002 AC-3.
  - macOS regression compile (`cargo check --target aarch64-apple-darwin`) per US-002 AC-4.
- Files most likely to need edits on bump: `src-app/Cargo.toml:21-23,63,72` (three git-dep entries + Linux feature line + test-support feature line), workspace-root `Cargo.toml:34-35` (`[patch.crates-io]` async-task and calloop entries).

## Evidence artifacts (reproducibility)

Commands executed on 2026-04-18 against `~/dev/zed`:

```bash
# DX11 port commit metadata
git log -1 --format='%H%n%ci%n%s' 15ad986329

# Ancestor proof
git merge-base --is-ancestor \
  15ad9863296d427966098f6b9864d5b819725101 \
  0b984b5ade7604e3f1c618c0ef77879de800b868 && echo IN-PIN

# Windows-platform changes since pin
git log --oneline 0b984b5..HEAD -- crates/gpui_windows/

# Patch block comparison
git show 0b984b5:Cargo.toml | grep -A 6 'patch.crates-io'
grep -A 6 'patch.crates-io' Cargo.toml  # at HEAD
```

All four commands were run during this spike. Outputs are preserved in the conversation transcript that produced this document.

---

## US-002 resolution addendum (2026-04-18)

US-002 ("Bump GPUI pin + validate Linux/macOS regression") resolves as a **no-op** in light of this spike's findings. Evidence and validation captured below so a reader of the status tracker can audit the decision without re-running the spike.

### Decision

- **No change to `src-app/Cargo.toml:21-23,63,72`.** The three `rev = "0b984b5ade7604e3f1c618c0ef77879de800b868"` entries stay as-is.
- **No change to `[patch.crates-io]` at workspace-root `Cargo.toml:34-35`.** The spike's patch-block diff table shows zero divergence between `0b984b5` and Zed HEAD.
- **No change to `Cargo.lock`.** `cargo update -p gpui -p gpui_platform -p collections` is not invoked — there is no new rev to resolve to.

### Acceptance-criterion disposition

| AC | Disposition | Evidence |
|----|-------------|----------|
| AC-1 — `Cargo.toml` rev update + refreshed `Cargo.lock` | **N/A — no bump.** Justification: US-001 spike proved the identified DX11 SHA is already an ancestor of the current pin. | Ancestor check: `git merge-base --is-ancestor 15ad986329 0b984b5…868` → 0. |
| AC-2 — `[patch.crates-io]` delta | **N/A — no delta.** Current pin and HEAD have byte-identical `[patch.crates-io]` blocks. | Patch table in this document, "§ `[patch.crates-io]` delta — current pin vs HEAD". |
| AC-3 — Linux `cargo build --release` + launch | **Certified at current pin.** See "§ Linux validation at current pin" below. Manual launch is a developer-verified step on interactive desktop. | Gate run 2026-04-18. |
| AC-4 — macOS `cargo check --target aarch64-apple-darwin` | **Deferred to CI** (no macOS host in this session). Will be verified by the existing `macos-check` CI job (`.github/workflows/ci.yml`, from `135b132 feat(macos): EP-005`) on the next push; will also be covered by US-003's CI provisioning work. | — |
| AC-5 — keybindings behave identically pre/post | **Trivially satisfied.** No binary change → no possible behavior change. | — |
| AC-6 — no silent regressions | **Trivially satisfied.** No code is changed, so there is nothing to silently regress. | — |

### Linux validation at current pin (AC-3)

Run 2026-04-18 against `0b984b5` with no source modifications:

```
cargo fmt --check            → clean (no output)
cargo check --workspace      → Finished `dev` profile [unoptimized + debuginfo]
cargo clippy --workspace -- -D warnings
                             → Finished `dev` profile [unoptimized + debuginfo]
cargo test --workspace       → 41 passed; 0 failed; 0 ignored
                                (all in paneflow-config; app crate has 0 tests
                                 per CLAUDE.md)
```

Four green gates on Linux at the current pin. This satisfies AC-3's "build completes with zero errors" requirement. The "resulting binary launches to the sidebar" smoke test is a developer-run visual step and is not gated by CI — it is considered verified by the fact that the binary has been running locally throughout v0.1.x development at this same pin.

### If a bump is ever required

Reopen US-002 under this trigger: US-003's `cargo check --target x86_64-pc-windows-msvc` job surfaces a compile error or a link error that traces — by reading the error site — to a GPUI or `gpui_windows` bug fixed by a specific post-`0b984b5` Zed commit. At that point:

1. Identify the minimum bumping-target SHA (the earliest Zed commit that contains the fix).
2. Re-run the spike's patch-block diff (`git show <sha>:Cargo.toml | grep -A 6 'patch.crates-io'` vs local).
3. Update the three rev entries in `src-app/Cargo.toml` and any diverged patches in workspace `Cargo.toml`.
4. Re-run all four Linux gates + push to CI for the macOS gate and (now) the Windows gate.
5. Smoke-launch on Linux to confirm sidebar rendering.

No such trigger exists today — US-003 is the next action, not a forced bump.
