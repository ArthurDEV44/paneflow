---
name: Windows bug report
about: Report a PaneFlow bug that reproduces on Windows 10 (1809+) or Windows 11
title: "[windows] "
labels: ["windows", "bug"]
assignees: []
---

<!--
Thanks for filing a Windows-specific report. PaneFlow's Windows build
ships from the `windows-2022` GitHub-hosted CI runner; the smoke-test
checklist at `docs/WINDOWS-SMOKE-TEST.md` covers only 10 scenarios
across Windows 10 1809 and Windows 11, so bugs that surface outside
that matrix are especially valuable.

If the bug reproduces on Linux or macOS too, use the generic bug
template instead so it can be triaged by anyone rather than waiting
on someone with a Windows box.

For context on known v1 limitations + upstream risks we already
track (IME panic, ConPTY Ctrl+C, RDP init, devcontainer freeze, GPU
driver floor), skim `docs/WINDOWS.md` first — your report may be a
duplicate of a risk we already document.
-->

## Environment

<!--
Privacy note: `systeminfo` includes your machine hostname and domain.
Before pasting output below, scrub anything you'd rather not publish
— a hostname redaction is usually enough.
-->

- **Windows version + build** (required — run `winver` or
  `systeminfo | Select-String "OS Version"` in PowerShell; expected
  shape `Windows 11 Pro 23H2 (build 22631.xxxx)` or
  `Windows 10 Enterprise 1809 (build 17763.xxxx)`):
- **Architecture** (pick one):
  - [ ] x86_64 (Intel / AMD 64-bit — supported)
  - [ ] ARM64 (Snapdragon X, Surface Pro X — **not in v1**, file
    anyway, we may still accept the report for future reference)
- **CPU** (e.g. `Intel Core i7-13700H`, `AMD Ryzen 7 7840U`):
- **GPU + driver** — run in PowerShell:
  ```powershell
  Get-CimInstance Win32_VideoController |
    Select-Object Name, DriverVersion, DriverDate
  ```
  (Useful for triaging `NoSupportedDeviceFound` and DX11-related
  render bugs against the upstream-risks catalog in `docs/WINDOWS.md`.)
- **PaneFlow version** (`paneflow --version`):
- **Install format** (pick one):
  - [ ] Signed MSI from GitHub Release (`paneflow-*-x86_64-pc-windows-msvc.msi`)
  - [ ] `winget install ArthurDev44.PaneFlow`
  - [ ] Built from source (`cargo build --release --target x86_64-pc-windows-msvc`)
- **Display environment** (pick one):
  - [ ] Local desktop session
  - [ ] Remote Desktop (RDP) — known-upstream risk, see
    `docs/WINDOWS.md` §4 "RDP initialization broken"
  - [ ] Nested via WSL2 / devcontainer — known-upstream risk
  - [ ] HiDPI or multi-monitor setup (specify scaling % and monitor count)

## Reproduction

Steps to reproduce, starting from a fresh `paneflow.exe` launch:

1.
2.
3.

## Expected behaviour

<!-- What should have happened? -->

## Actual behaviour

<!-- What actually happened? Include screenshots or a short screen recording if the bug is visual. -->

## Logs

Launch PaneFlow from a PowerShell prompt with logging + backtraces
enabled:

```powershell
$env:RUST_LOG = "info"
$env:RUST_BACKTRACE = "1"
& "C:\Program Files\PaneFlow\paneflow.exe"
```

Copy the stderr block covering the bug window (and the backtrace, if
the process crashed).

<details>
<summary>PaneFlow log output</summary>

```
paste log / backtrace here
```

</details>

## Additional context

<!--
Anything else that might help. Examples:
  - Does the bug reproduce on Linux or macOS with the same config?
    (If yes, please use the generic bug template instead — this
    Windows-specific template is reserved for Windows-only bugs.)
  - Shell in use (pwsh 7 / Windows PowerShell 5.1 / cmd.exe / WSL) —
    Ctrl+C and OSC 7 behaviour varies per shell.
  - Antivirus in use (Microsoft Defender / third-party) — some AVs
    false-positive on recently-signed binaries during reputation build.
  - If the bug is install-time, the output of `msiexec /i <path> /l*v install.log`
    and attach `install.log`.
-->
