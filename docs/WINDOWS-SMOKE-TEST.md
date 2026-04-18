# Windows release smoke-test runbook

Manual QA checklist run on Windows 10 (1809) and Windows 11 VMs before
every Windows release is promoted from pre-release to the public
"latest" tag. Referenced by US-021 in
[`tasks/prd-windows-port.md`](../tasks/prd-windows-port.md).

CI cannot exercise these scenarios — the `windows-2022` GitHub-hosted
runner image covers compile + signtool verification only
(`.github/workflows/ci.yml` windows-check job, US-018), and end-to-end
install + UI behaviour requires a real desktop session with SmartScreen,
the Start Menu, Add/Remove Programs, and ConPTY in their production form.

See [`docs/WINDOWS.md`](WINDOWS.md) (US-022) for the user-facing catalog
of known upstream risks this runbook's documented-limitation scenarios
point at.

---

## When to run

Run this runbook against every release candidate **before** the
Windows asset is attached to the public GitHub Release. The expected
cadence mirrors `docs/validation-aarch64.md`:

1. Maintainer pushes `vX.Y.Z-rc.1` — the release workflow cuts a
   pre-release (`release.yml` detects `rc` in tag name and sets
   `prerelease: true`). The `release-assets-x86_64-Windows` workflow
   artifact contains the signed MSI + SHA-256 sidecar.
2. Maintainer downloads the MSI to a host box, then copies it into
   each Windows VM.
3. Maintainer executes §5 on both VMs and attaches the screenshot
   gallery (§7) to the release PR.
4. If every scenario passes or is explicitly marked "known limitation
   — documented", maintainer retags `vX.Y.Z` to trigger the final
   release. Otherwise triage per §6.

Do NOT skip a release's smoke test because "nothing changed on
Windows since last release" — GPUI commits, cargo-wix output, and
Azure Artifact Signing cert rotations all silently shift behaviour;
every release is a fresh validation.

---

## VM setup

### Windows 10 1809

Microsoft publishes time-limited evaluation VHDs for older Windows 10
builds. Rather than hard-coding a download URL that may rot, navigate to
Microsoft's official page for Windows 10 Enterprise evaluation and pick
the **1809** (October 2018 Update) build specifically. This build is the
minimum Windows version PaneFlow supports (ConPTY was introduced in
1809 and the GPUI DX11 backend's minimum target is 1809).

- Recommended VM host: VirtualBox, VMware Workstation Player, or Hyper-V
- Minimum resources: 4 GB RAM, 2 vCPU, 40 GB disk, 3D acceleration
  enabled (DirectX 11 is required — GPUI renders via the DX11 backend
  per the US-001 spike)
- Snapshot after first boot so every smoke run starts from a clean state

### Windows 11

Any Windows 11 build >= 22H2 is acceptable (winget is pre-installed,
ConPTY is current). Use either:

- A Microsoft-published Windows 11 evaluation VHD (same landing page as
  Windows 10 Enterprise Evaluation, pick the Windows 11 entry), OR
- A physical Windows 11 dev box if available

Same host requirements as Windows 10. Snapshot after first boot.

---

## Prerequisites

Before starting any scenario:

1. Confirm the pre-release GitHub Release carries a
   `paneflow-<ver>-x86_64-pc-windows-msvc.msi` asset plus its
   `.sha256` sidecar. If not, the Windows matrix leg failed
   (`continue-on-error: true` per US-017 means Linux + macOS legs may
   have still shipped; block the Windows release until the MSI is
   produced).
2. Verify the SHA-256 of the downloaded MSI against the sidecar on
   the host box before copying into any VM:
   ```powershell
   Get-FileHash .\paneflow-<ver>-x86_64-pc-windows-msvc.msi -Algorithm SHA256
   # Compare against the .sha256 sidecar content
   ```
   A mismatch means the asset was corrupted between CI and your
   download — re-download before proceeding.
3. Copy the verified MSI into each VM via a shared folder or a
   throwaway HTTP server on the host. Do NOT re-download inside the
   VM from GitHub — re-downloading inside the VM re-stamps the file
   with a fresh Mark-of-the-Web zone marker, which may produce a
   different SmartScreen verdict than the file a real user downloads
   via their browser from the CDN. Keep the MSI's origin traceable
   to CI so scenario 2 measures the trust-chain users actually see.

---

## The 10 scenarios

Each scenario is run on BOTH Windows 10 1809 and Windows 11 unless
explicitly noted otherwise. Capture a screenshot at the step marked
`[📸]` — these populate the release-PR gallery per §7.

### Scenario 1 — Fresh install via double-click MSI

**Steps:**
1. In the clean VM, locate the copied-in MSI via File Explorer.
2. Double-click `paneflow-<ver>-x86_64-pc-windows-msvc.msi`.
3. Accept the UAC elevation prompt.
4. Click through the WiX installer dialogs (default install location
   `%ProgramFiles%\PaneFlow\`, "Install" button).
5. `[📸]` Capture the final "Installation complete" dialog.

**Expected:**
- Installer completes without errors.
- `%ProgramFiles%\PaneFlow\paneflow.exe` exists.
- A "PaneFlow" entry appears under `Add or Remove Programs` with
  publisher "Strivex" and the same version as the MSI filename.

---

### Scenario 2 — SmartScreen shows "Signed by Strivex"

**Steps:**
1. Restore the VM to its pre-scenario-1 clean snapshot (or use a
   second clean VM). Scenario 2 specifically measures the
   first-time-trust dialog a real user sees — do NOT run it
   immediately after scenario 1, because Windows caches the
   Authenticode decision once the user clicks through once.
2. Double-click the MSI.
3. SmartScreen may show an elevated-trust dialog on a freshly-
   published signature (reputation is still being built).
4. `[📸]` Capture the SmartScreen dialog.

**Expected:**
- Dialog says `Windows protected your PC` with publisher
  **"Strivex"** (NOT "Unknown Publisher").
- Clicking `More info` → `Run anyway` proceeds to the UAC prompt.

**Known limitation:** For the first few weeks after a fresh Azure
Artifact Signing certificate profile is issued, reputation is still
accumulating. SmartScreen may display the amber "Windows protected
your PC" dialog (not the red "Blocked" one). Either variant satisfies
AC-2 as long as the publisher string is "Strivex" and the Run-anyway
path works. Red dialog = FAIL.

---

### Scenario 3 — Launch from Start Menu

**Steps:**
1. After scenario 1's install completes, press the Windows key.
2. Type `PaneFlow`.
3. Click the `PaneFlow` Start Menu entry.
4. `[📸]` Capture the initial window.

**Expected:**
- Search returns the "PaneFlow" tile matching
  `packaging/wix/main.wxs`' Start Menu shortcut.
- Clicking launches `paneflow.exe` with no console window, no
  "Windows cannot open this file" dialog.
- Initial window shows a single terminal pane running the default
  shell (see scenario 6 for shell-specific assertions).

---

### Scenario 4 — `Ctrl+Shift+N` creates a new workspace

**Steps:**
1. With PaneFlow running from scenario 3, press `Ctrl+Shift+N`.
2. Observe the sidebar.
3. `[📸]` Capture the sidebar showing two workspaces.

**Expected:**
- A new workspace appears in the sidebar (second row).
- Focus moves to the new workspace.
- The original workspace is still listed (not replaced).

---

### Scenario 5 — `Ctrl+Shift+D` horizontal split

**Steps:**
1. In any workspace, press `Ctrl+Shift+D`.
2. `[📸]` Capture the pane with a horizontal divider.

**Expected:**
- The pane splits into two: top and bottom halves (horizontal
  divider per the split system in `src-app/src/split.rs` — note
  `SplitDirection::Horizontal` means a horizontal divider bar, panes
  stacked vertically, consistent with the repo convention).
- Both halves host their own shell; focus is on the new (bottom) pane.
- Dragging the divider bar resizes the panes (min 80px per pane,
  ratio clamped 0.1–0.9).

---

### Scenario 6 — PowerShell 7 spawns correctly

**Steps:**
1. Ensure `pwsh.exe` is installed on the VM. Windows 11 typically
   ships it; Windows 10 1809 does not — install via
   `winget install Microsoft.PowerShell` (or skip this scenario and
   note `pwsh: not installed — testing windows powershell 5.1
   fallback` in the run log).
2. Close any open PaneFlow windows, relaunch.
3. At the shell prompt in the default pane, run:
   ```
   $PSVersionTable.PSVersion
   ```
4. `[📸]` Capture the output.

**Expected:**
- On a VM with `pwsh.exe` installed: `Major: 7, Minor: >= 3` displays.
- On a VM without `pwsh.exe`: the Windows shell fallback chain
  (US-006) picks `powershell.exe` 5.1 or `cmd.exe`; the scenario
  still passes as long as *some* shell spawns and responds.
- No "shell process exited with code 1" banner; no blank pane.

---

### Scenario 7 — 100 chars + resize does not crash ConPTY

**Steps:**
1. At any shell prompt, type 100 characters of plain text (e.g.
   hold `a` until the line wraps).
2. Grab the window edge and drag-resize the window 10+ times rapidly
   (smaller → larger → smaller) while the typed buffer remains on
   screen.
3. Press Enter (so the shell interprets the 100-char line as a
   command and prints "command not found" or similar).
4. `[📸]` Capture the window after the final resize.

**Expected:**
- No crash dialog, no application exit.
- Text remains legible; resizing causes reflow without dropped
  characters.
- ConPTY does not emit a panic — PaneFlow's stderr (if captured
  via `RUST_LOG=info` on a follow-up run) shows no
  `ClosePseudoConsole` / `ResizePseudoConsole` errors.

**Known limitation pointer:** This scenario catches the class of
bugs documented in zed#12563 (GPUI IME panic) and in the alacritty
ConPTY driver. Crash → FAIL; harmless render glitch → PASS with note.

---

### Scenario 8 — `Ctrl+C` at shell prompt interrupts

**Steps:**
1. At a shell prompt, start a long-running foreground command:
   ```powershell
   Start-Sleep -Seconds 300
   ```
   (or `ping -t 8.8.8.8` on cmd.exe / `sleep 300` on pwsh).
2. Wait 2 seconds.
3. Press `Ctrl+C`.
4. `[📸]` Capture the prompt returning.

**Expected:**
- The running command terminates.
- The shell prompt returns within 1 second.
- No PaneFlow application crash.

**Known limitation (AC-8):** Per [alacritty#3075](https://github.com/alacritty/alacritty/issues/3075),
ConPTY `Ctrl+C` signal propagation is known-imperfect. In some shell
configurations the signal can propagate to parent processes
unexpectedly. The scenario is still testable: as long as the running
command stops and the prompt returns, the scenario PASSES. Mark
"PASS with upstream caveat" in the run log — do NOT block the
release on this specific edge case unless PaneFlow itself crashes.

---

### Scenario 9 — Close + reopen preserves config

**Steps:**
1. Open PaneFlow, confirm its config file exists at
   `%APPDATA%\paneflow\paneflow.json`.
   - On Windows this path resolves via `dirs::config_dir()` to
     `C:\Users\<you>\AppData\Roaming\paneflow\`.
2. Edit the config (e.g., change `theme` from "Catppuccin Mocha" to
   "Dracula") and save. Watch for hot-reload (500ms polling) — theme
   should update within ~1 second.
3. Close all PaneFlow windows.
4. Relaunch from Start Menu.
5. `[📸]` Capture the window showing the Dracula theme still applied.

**Expected:**
- Config file persists across launches — the "Dracula" setting from
  step 2 is honored on relaunch.
- No "default config created" message (would indicate the config
  file was wiped).

---

### Scenario 10 — Uninstall preserves user config

**Steps:**
1. Open `Add or Remove Programs` (Settings → Apps → Apps & features
   on Windows 10; Settings → Apps → Installed apps on Windows 11).
2. Locate the `PaneFlow` entry, click → `Uninstall`.
3. Confirm the UAC + MSI uninstall dialogs.
4. After uninstall completes:
   - Verify `%ProgramFiles%\PaneFlow\` is absent (or empty).
   - Verify `%APPDATA%\paneflow\paneflow.json` **STILL EXISTS** and
     retains the Dracula theme setting from scenario 9.
5. `[📸]` Capture both `explorer` windows side by side: the empty
   Program Files path and the still-populated AppData path.

**Expected:**
- WiX uninstall removes install-time files from `Program Files` but
  leaves user config at `%APPDATA%\paneflow\` untouched (AC-6 of
  US-013 — idiomatic Windows uninstall behaviour).
- Reinstalling the same version and relaunching picks up the
  preserved config — the user's theme and preferences from
  `paneflow.json` survive the reinstall cycle.

---

## Failure triage (AC-4)

When a scenario fails on either VM, classify the failure into exactly
one of three buckets, in this order:

### Bucket A — Fix in-PRD (reopen a prior story)

The failure traces to a code defect in a story that is still within
the current PRD's scope. Reopen the offending story from `IN_REVIEW`
→ `IN_PROGRESS`, fix the code, and re-cut the release candidate.
Examples:

- Scenario 1 fails because the MSI is unsigned: reopen US-015 /
  US-016 (signing wrapper + pipeline wiring).
- Scenario 3 fails because the Start Menu shortcut is missing:
  reopen US-013 (WiX config).
- Scenario 9 fails because config is written to `%LOCALAPPDATA%`
  instead of `%APPDATA%`: reopen US-009 (interprocess ipc + the
  `runtime_paths` Windows branch that resolves `dirs::config_dir()`).

### Bucket B — Log as known-v1 issue in `docs/WINDOWS.md`

The failure is a known upstream limitation, a cosmetic bug, or a
functional gap explicitly scoped out of v1 (see "Out of Scope" in
`tasks/prd-windows-port.md`). Add or update an entry in the
"Known limitations" section of `docs/WINDOWS.md` (US-022) with:

- Title, description, upstream GitHub issue link
- Severity: cosmetic / functional / blocker
- Workaround, if any

The release still ships; users are warned.

### Bucket C — Block release

The failure is a user-visible crash, data loss, or a security issue.
Do NOT ship the release. Open a GitHub issue with a clear repro, a
screenshot, and a link to this runbook entry. Decide whether to hot-
fix (bucket A applied selectively) or defer the entire Windows
release one cadence cycle.

**Default triage for ambiguity:** when in doubt between bucket B and
bucket C, prefer bucket C — shipping a known-broken installer
damages publisher reputation faster than a one-release delay. The
SmartScreen "Signed by Strivex" reputation takes 6-8 weeks to build
and can be reset by a single crash-on-first-launch bug.

---

## Screenshot gallery (AC-5)

Each `[📸]` step above produces one screenshot per VM. Name files:

```
smoke/<tag>/win10-<N>-<slug>.png
smoke/<tag>/win11-<N>-<slug>.png
```

Where:
- `<tag>` is the release tag (e.g. `v0.1.7`)
- `<N>` is the scenario number (1..10)
- `<slug>` is a kebab-case one-liner (e.g. `smartscreen-strivex`,
  `start-menu-launch`, `uninstall-config-preserved`)

**Attachment:** Zip the `smoke/<tag>/` directory and drag-drop the
archive into the release PR description on GitHub, OR attach the
individual PNGs inline. Either format satisfies AC-5 as long as one
reviewer can download all 20 screenshots (10 scenarios × 2 VMs)
without leaving GitHub.

Retention: the `smoke/` archive is deliberately NOT committed to the
repo — images inflate clone size and the release PR thread serves
as the permanent record.

---

## Cross-references

- [`tasks/prd-windows-port.md`](../tasks/prd-windows-port.md) — the
  PRD containing US-021 (this runbook) and the risks table
- [`docs/WINDOWS.md`](WINDOWS.md) — user-facing known-limitations
  catalog. **NOTE:** the file does not exist in the repo until US-022
  ships; the link above 404s today. Triage bucket B entries land here
  once the doc is authored.
- [`docs/validation-aarch64.md`](validation-aarch64.md) —
  structural sibling for aarch64 Linux validation
- [`.github/workflows/release.yml`](../.github/workflows/release.yml)
  — the pipeline that emits the MSI under test
- [`packaging/wix/main.wxs`](../packaging/wix/main.wxs) — WiX source
  under test (ProductCode, UpgradeCode, install path)
- alacritty#3075 — upstream ConPTY Ctrl-C limitation referenced by
  scenario 8
