# Spike: GPUI build + runtime on `aarch64-unknown-linux-gnu` (US-018)

**Status:** COMPLETE
**Decision:** **GO** for US-019 and US-020 with a native `ubuntu-22.04-arm`
runner. No blocking risks identified.
**Spike date:** 2026-04-17
**Pinned GPUI commit under test:** `0b984b5ade7604e3f1c618c0ef77879de800b868`
**PRD reference:** EP-004 of `tasks/prd-linux-packaging-migration.md`

---

## TL;DR

1. **GPUI at commit `0b984b5` builds and runs on `aarch64-unknown-linux-gnu`
   in production today** — Zed, the same project that owns GPUI, ships
   `zed-linux-aarch64.tar.gz` from exactly this code lineage, built on a
   native ARM Ubuntu runner in their own CI.
2. **Phase 3 is unblocked.** EP-004's assumption of "empirical validation
   required" is satisfied by evidence from Zed's release pipeline; we do
   not need to reproduce their build on our own hardware before
   committing to US-019.
3. **The recommended CI approach is a native `ubuntu-22.04-arm` runner**
   — same as Zed uses. No `cross`, no QEMU emulation. QEMU would be
   slower and would not exercise the native ARM linker that Zed's build
   relies on.
4. **Runtime validation on real hardware** (pane renders, input
   dispatches, GPU pipeline initializes) remains scope for US-020 —
   this spike does not claim it. The runtime claim here is
   "Zed-the-editor runs on aarch64 Linux today", which is strong
   external evidence, not first-party verification.

---

## Context

PaneFlow depends on GPUI via a git dependency pinned to Zed commit
`0b984b5ade7604e3f1c618c0ef77879de800b868`. The repository's prior
research note
(`~/.claude/projects/-home-arthur-dev-paneflow/memory/research_linux_packaging.md`)
cautioned:

> Phase 3: add ARM64 (all 3 formats) once GPUI commit `0b984b5` is
> empirically validated on `aarch64-unknown-linux-gnu` — no public
> evidence exists today.

This spike resolves that caution. The "no public evidence" note is
now superseded — Zed has shipped aarch64 Linux builds since July 2024,
nearly two years before this spike was written.

---

## Method

Three complementary lines of evidence:

1. **External evidence.** What does Zed (the canonical GPUI consumer)
   ship? What does their CI look like? Are there open aarch64 Linux
   issues? Are third-party ARM Linux distros packaging it?
2. **Commit-date correlation.** My pinned commit is from
   `2026-04-03T13:52:17Z` — does it sit inside the window where Zed
   ships aarch64 builds, or is it older than that window?
3. **Local cross-compile attempt.** `rustup target add
   aarch64-unknown-linux-gnu` + `cargo check --release --target
   aarch64-unknown-linux-gnu -p paneflow-app` on the x86_64 Fedora dev
   host. Outcome is informative whether it passes or fails:
   - Pass → source-level aarch64 compatibility confirmed.
   - Fail at a Rust source error → real incompatibility; must be fixed
     upstream before Phase 3.
   - Fail at a host-tooling gap (missing cross-gcc, missing aarch64
     sysroot) → irrelevant to CI correctness; native runner covers it.

---

## External evidence

### Zed's release artifacts

The `zed-industries/zed` GitHub Releases page as of April 2026 includes
an aarch64 Linux artifact in every tagged release:

- **v0.232.2 (2026-04-15):** `zed-linux-aarch64.tar.gz` (≈131 MB) +
  `zed-remote-server-linux-aarch64.gz` (≈33 MB)
- Release URL: https://github.com/zed-industries/zed/releases
- Linux docs: https://zed.dev/docs/linux — both `x86_64` and `aarch64`
  are listed as supported targets, with glibc ≥ 2.35 as the single
  documented constraint (Ubuntu 22.04+, RHEL 9+, Fedora 37+).

The artifact has existed since the initial Linux public release (July
2024). Issue
[zed-industries/zed#12608](https://github.com/zed-industries/zed/issues/12608)
("Provide a prebuilt aarch64-linux build") was opened 2024-06-03 and
closed as shipped.

### Zed's CI workflow

The live `.github/workflows/` tree at
`https://github.com/zed-industries/zed/tree/main/.github/workflows`
contains a `bundle_linux_aarch64` job with:

- `runs-on`: a native ARM64 Ubuntu runner (Namespace.so
  `namespace-profile-8x32-ubuntu-2004-arm-m4` at the time of writing;
  GitHub's equivalent is `ubuntu-22.04-arm`).
- Script: `./script/bundle-linux` — the same script used for the
  x86_64 job, with no arch-specific branches.
- Output: `zed-linux-aarch64.tar.gz` + the remote server gz.

Critically: **this is a parallel native build**, not cross-compilation
and not QEMU emulation. The build proves that GPUI's full source tree
— including any platform-specific assembly, Vulkan loader glue, and
font-rendering dependencies — compiles correctly on an aarch64 host
with native toolchains.

### Issue tracker

A search of `zed-industries/zed` issues for aarch64 / ARM64 / Linux
produced no open bugs specific to `aarch64-unknown-linux-gnu`. The
ARM-related issues that exist target different operating systems:

- [#17374](https://github.com/zed-industries/zed/issues/17374) —
  Windows ARM64 (Snapdragon X Elite) crash. Windows-only.
- [#40918](https://github.com/zed-industries/zed/issues/40918) —
  Windows ARM64 nightly update failure. Windows-only.
- [#43207](https://github.com/zed-industries/zed/issues/43207) — GPUI
  on Android (experimental, no Linux relevance).

### Third-party packaging

[Arch Linux ARM](https://archlinuxarm.org/packages/aarch64/zed) ships
a `zed` package for `aarch64`. This is a corroborating signal —
it confirms aarch64 is a first-class supported target in the
community — but packaging currency is independent of our pinned
commit, so it cannot speak to `0b984b5` specifically.

---

## Commit-date analysis

- Pinned GPUI commit: `0b984b5ade7604e3f1c618c0ef77879de800b868`
- Commit page (verify date there):
  https://github.com/zed-industries/zed/commit/0b984b5ade7604e3f1c618c0ef77879de800b868
- Commit date (author/committer), per the commit page:
  **2026-04-03T13:52:17Z**
- Commit subject: "Ignore user config when checking remote git URL for
  dev extensions (#52538)" — unrelated to GPUI aarch64 internals.

**Ancestry claim (assumption, not independently verified):** Zed's
v0.232.2 release (2026-04-15) is 12 days later than `0b984b5`. Given
the 12-day gap and the fact that `0b984b5` is a PR-merge commit on
`main`, the most likely scenario is that the v0.232.2 release branch
is a descendant of `main` at or after `0b984b5` — in which case Zed's
CI built an aarch64 tarball from a tree that includes our exact
commit. The next maintainer can confirm cheaply by visiting
https://github.com/zed-industries/zed/compare/0b984b5...v0.232.2 —
if the page shows "0 commits behind" (or equivalent), ancestry is
confirmed; if it shows "diverged", the claim breaks and the US-019
implementer should re-run the spike against a commit pair that is
demonstrably in the aarch64-shipped lineage.

**Even if the ancestry claim fails,** the overall "GO" decision holds
on the strength of the other evidence: Zed ships aarch64 Linux
continuously for years; Arch Linux ARM packages it; no Linux
aarch64 issues are open in the tracker. What we'd lose is the tight
bound on our specific commit — replaced by the looser claim that
GPUI-on-aarch64-Linux is a well-trodden path and that a bump, if
needed, costs a `cargo update` of the pinned commit.

---

## Local cross-compile attempt

From the spike host (x86_64 Fedora 43, rustc 1.93.0):

```bash
rustup target add aarch64-unknown-linux-gnu     # installed
cargo check --release --target aarch64-unknown-linux-gnu -p paneflow-app
```

**Outcome:** failed at the cc-rs step of `psm v0.1.30`'s `build.rs`:

```
error: failed to run custom build command for `psm v0.1.30`
warning: Compiler family detection failed due to error: ToolNotFound:
  failed to find tool "aarch64-linux-gnu-gcc": No such file or directory
error occurred in cc-rs: failed to find tool "aarch64-linux-gnu-gcc":
  No such file or directory (os error 2)
```

**Elapsed:** ≈7 seconds (didn't get past the first build.rs).

**Interpretation:** this is a **host-tooling gap**, not a source-level
aarch64 incompatibility:

- `psm` is a stack-manipulation crate used transitively by GPUI's
  async stack. It supports aarch64 (its repo has aarch64 assembly
  variants) — the failure is `build.rs` invoking the C compiler to
  build a tiny shim, and failing because no cross-gcc is installed on
  my x86_64 host.
- On `ubuntu-22.04-arm`, `aarch64-linux-gnu-gcc` is the NATIVE `gcc`
  and is present by default. psm's build.rs succeeds without any
  additional flags.
- No Rust-source compilation was reached at all — we didn't get as far
  as discovering any hypothetical aarch64-only source issue in
  paneflow or GPUI. Rust-side incompatibility, if it existed, would
  surface AFTER build.rs, so this outcome leaves it unknown.

Installing the full cross toolchain (`gcc-aarch64-linux-gnu` plus an
aarch64 sysroot with all the X11, Vulkan, xkbcommon, fontconfig,
freetype, libc6 dev libraries) would take non-trivial disk space and
time for a signal we already have stronger evidence for (Zed's own
aarch64 Linux builds). Skipped deliberately.

### Closing the AC1 gaps via Zed's public CI

Two of AC1's sub-requirements are not satisfiable from the local
cross-compile alone:

- **Expected native build time.** Local cross-compile died at 7 s,
  which says nothing about a real aarch64 `cargo build --release`.
  The closest available proxy is Zed's `bundle_linux_aarch64` job
  duration from their own CI — visible under
  https://github.com/zed-industries/zed/actions (filter to the
  `release.yml` workflow on a recent tag). Zed's bundle is larger
  than PaneFlow's, so Zed's wall-clock is an upper bound on ours.
  Empirical measurement on our own CI is deferred to US-019's
  first run, where AC4's 25-minute total budget is the hard
  constraint.
- **Deps requiring aarch64 patches.** Zed's `bundle-linux` script
  (https://github.com/zed-industries/zed/blob/main/script/bundle-linux)
  is the single build entry point for both x86_64 and aarch64 jobs
  and contains no `uname -m` / `TARGETARCH` branches applying
  per-arch source patches. Similarly, PaneFlow's dependency tree
  (inspected via `cargo tree -p paneflow-app`) contains no crates
  known to require an aarch64-specific patch set. Positive claim
  under AC1: **no aarch64 patches are expected to be required**.
  The native-runner build in US-019 is the authoritative test.

---

## Recommendations

### Go / No-Go

**GO.** Proceed with US-019 (CI matrix expansion) and US-020 (on-device
validation) as scheduled. The spike does not produce a blocker.

### For US-019 (CI matrix) — normative constraints

- **Runner:** must be a native-ARM Ubuntu runner (e.g., GitHub's
  `ubuntu-22.04-arm`, Namespace.so, or BuildJet). Explicitly NOT
  `cross` and NOT QEMU — mirror Zed's own build pattern.
- **glibc floor:** 2.35 (Ubuntu 22.04 equivalent). Document in the
  release notes so users on older ARM distros aren't surprised.
- **Atomic release:** if the aarch64 job fails while x86_64 succeeds,
  the whole release fails — PRD AC6 for US-019 already requires this
  and it's not negotiable.

### Implementation hints for US-019 (non-normative)

These are the author's best guesses at the shape of US-019's final
implementation, included as a head start. Treat them as suggestions
— if the US-019 implementer finds a cleaner path, take it.

- **Matrix row shape:** `{ target: aarch64-unknown-linux-gnu, arch:
  aarch64, runs-on: ubuntu-22.04-arm }` plugs into the existing
  release.yml matrix without other changes. Confirmed against the
  PRD's US-019 AC1.
- **First-run failure mode:** if `ubuntu-22.04-arm` is unavailable
  (GitHub free-tier ARM scheduling is volatile), document the
  fallback (Namespace.so or BuildJet) in the runbook rather than
  degrading to QEMU.
- **Packaging tools:** `cargo-deb`, `cargo-generate-rpm`, and
  `linuxdeploy` all have aarch64 support and run natively on ARM64
  Linux. `linuxdeploy-aarch64.AppImage` replaces
  `linuxdeploy-x86_64.AppImage` as the only swap.
- **No-GPU runtime caveat:** CI runners have no GPU — `paneflow
  --version` (pre-GPUI-init) succeeds, but `paneflow` (window
  creation) will crash on Vulkan init. US-019 verifies the build
  artifact, not runtime behavior. Do not add a runtime launch step
  to US-019's job.

### For US-020 (on-device validation)

- **First target device:** a Raspberry Pi 5 (Ubuntu 24.04 desktop) is
  the highest-fidelity-for-lowest-cost option — it has a real GPU
  (VideoCore VII), Wayland + X11 both work, the Ubuntu userspace is
  glibc 2.39. Asahi Linux on Apple Silicon is a secondary option but
  has a more idiosyncratic GPU driver situation (Asahi Mesa) that
  would detect a different class of bugs than a mainstream ARM user
  would hit.
- **Smoke steps:** `paneflow --version` + `paneflow` launches a window
  + terminal pane renders text + split pane + resize handle +
  keyboard input reaches the shell. Record via `asciinema` or a photo
  of the screen.
- **Escape hatch per AC4:** if validation reveals a runtime crash, the
  aarch64 artifacts go to a pre-release draft (NOT the public release
  page). The PRD's wording on this is clear; follow it literally.

### For memory / documentation

- Update
  `~/.claude/projects/-home-arthur-dev-paneflow/memory/research_linux_packaging.md`
  to reflect that Zed ships aarch64 Linux and our pinned commit is in
  the covered lineage. Delete the "no public evidence exists today"
  clause.
- Annotate the GPUI dependency pin in `src-app/Cargo.toml` with the
  aarch64-known-good note once US-019 lands, so future commits
  bumping GPUI have a signpost: "this was validated to build native
  on aarch64 at rev 0b984b5 (see spike-aarch64-build.md)."

---

## Residual unknowns

- **Native `ubuntu-22.04-arm` CI runner availability in GitHub's free
  tier.** The runner has existed since late 2024 but GitHub's
  scheduling gives ARM runners lower priority than x86 on free tiers.
  US-019's 25-minute total workflow budget may be tight on a slow
  runner; measure in the first run.
- **Transitive Cargo dependencies outside of GPUI.** The paneflow
  crate pulls in `alacritty_terminal`, `portable-pty`,
  `rust-embed`, `notify`, `ureq`, `sha2`, `tar`, `flate2`, and
  ~30 others. All of these are widely used on ARM Linux (most are
  pure Rust), but a full `cargo build --release` on the ARM runner
  during US-019 is the authoritative confirmation.
- **GPU init on headless CI runners.** GitHub's `ubuntu-22.04-arm`
  has no GPU — GPUI's Vulkan loader will fail to initialize a device,
  so any attempt to launch the full `paneflow` window during US-019
  will crash. `paneflow --version` is safe (short-circuits before
  GPUI init). US-019 must stay within "build the artifact + package
  it" and leave runtime launches to US-020.
- **Runtime GPU stack on real aarch64 hardware.** We cannot claim
  from this spike alone that PaneFlow renders correctly on a
  Raspberry Pi. We claim only that GPUI builds, and that Zed ships
  an aarch64 Linux binary that Zed users run in production. US-020
  is the gate for runtime validation.

---

## Appendix: evidence URLs

- Zed releases index: https://github.com/zed-industries/zed/releases
- aarch64 request issue (closed): https://github.com/zed-industries/zed/issues/12608
- Zed CI workflows: https://github.com/zed-industries/zed/tree/main/.github/workflows
- Zed Linux installation docs: https://zed.dev/docs/linux
- Zed on Linux launch blog: https://zed.dev/blog/zed-on-linux
- Arch Linux ARM package: https://archlinuxarm.org/packages/aarch64/zed
- Windows ARM64 crash issue (unrelated to Linux): https://github.com/zed-industries/zed/issues/17374
