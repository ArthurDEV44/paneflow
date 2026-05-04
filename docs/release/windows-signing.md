# Windows code signing runbook (US-024)

One-time setup, secret rotation, and ACME auto-renewal tracking for the
PaneFlow Windows release pipeline. The `release.yml` workflow signs every
`x86_64-pc-windows-msvc` `.msi` produced by `cargo wix build` when the six
`AZURE_*` GitHub Secrets are populated. If any secret is missing the leg
degrades to an **unsigned** build with a banner in the job summary — by
design (US-024 AC-5).

This document is operator-only. Application code never reads from any of
the secrets described here.

---

## 1. What gets signed

`cargo wix build` (driven by `[package.metadata.wix]` in
`src-app/Cargo.toml` and `packaging/wix/main.wxs`) produces a single
artifact:

```
target/wix/paneflow-<version>-x86_64.msi
```

`scripts/sign-windows.ps1` then:

1. **Fetches the Microsoft.ArtifactSigning.Client NuGet** (pinned to
   `1.0.128`) into a per-invocation temp directory and resolves
   `bin/x64/Azure.CodeSigning.Dlib.dll`.
2. **Writes a `metadata.json`** with `Endpoint`, `CodeSigningAccountName`,
   `CertificateProfileName`, and an `ExcludeCredentials` list narrowing
   `DefaultAzureCredential` to `EnvironmentCredential` only — without this
   narrowing the dlib hangs for minutes on managed-identity probes on
   GitHub-hosted runners.
3. **Calls `signtool.exe sign /dlib ... /dmdf ...`** with a 3-host
   timestamp retry chain (US-024 AC-3): `acs.microsoft.com` →
   `digicert.com` → `sectigo.com`. The chain survives short-lived Azure
   timestamp outages without requiring a manual re-sign.
4. **Verifies** with `signtool verify /pa /v` AND
   `Get-AuthenticodeSignature` (the latter catches "signtool says OK but
   Defender rejects" cert-chain failures).
5. **Asserts the signer subject** anchors `O=Strivex` to defeat a
   substring-only `CN=Strivex-Evil-Clone` impostor in a hostile trust
   store.

The CI `Verify MSI signature` step then re-runs the bare `signtool
verify /pa /v` at the workflow level, greps for the AC-6 sentinel
`"Successfully verified"`, and fails if absent — making verification
unambiguously visible in the run log.

## 2. Why a script and not the official action

US-024 AC-1 calls for "the official Azure Trusted Signing action". The
official `azure/trusted-signing-action` is a GitHub Action wrapper around
the same `Microsoft.ArtifactSigning.Client` NuGet that `sign-windows.ps1`
uses, but it accepts only **one** `timestamp-rfc3161` URL and provides
**no fallback chain**. AC-3 mandates ≥ 3 timestamp servers with retry,
which the action cannot satisfy without per-call shell wrapping that
defeats the action's ergonomic value.

`scripts/sign-windows.ps1` is therefore the canonical Azure Trusted
Signing integration in this repo: it calls the same official Microsoft
client at the `signtool /dlib ... /dmdf ...` layer and adds the retry
loop. If a future Microsoft release exposes timestamp fallback in the
action itself, revisit this decision and replace the script invocation
with the action.

## 3. One-time onboarding

`memory/project_windows_signing.md` is the single source of truth for
the Strivex Azure subscription, account name, certificate profile name,
and rotation calendar. Update it whenever any of the values below
change.

1. **Azure subscription.** Decided 2026-04-18: Azure Trusted Signing
   under the Strivex (France SAS) Azure subscription, owner
   `arthur.jean@strivex.fr`. Business onboarding is the only path —
   individual developers cannot currently use Azure Trusted Signing.
2. **Provision the Trusted Signing account** in Azure Portal → Trusted
   Signing → Create. Pick the closest region (the endpoint URL becomes
   `https://<region>.codesigning.azure.net/`). Tier: **Basic**
   ($9.99/mo) is sufficient for our signing volume.
3. **Create a Certificate Profile.** Name it `PaneFlow-Release`, type
   **Public Trust** (NOT "Private Trust" — the latter is enterprise-only
   and won't satisfy SmartScreen). Public-Trust profiles ACME-rotate the
   underlying signing certificate automatically (currently a 3-day cert
   lifetime per Microsoft Learn): no manual renewal, no expiration
   tracking on this side. The dlib fetches a fresh leaf cert per sign
   operation.
4. **Create the GitHub Actions service principal.**
   - Azure AD → App registrations → New registration. Name:
     `GitHub-ActionsSigning`. Single tenant.
   - Note the Application (client) ID and Directory (tenant) ID.
   - Certificates & secrets → New client secret. **Copy the value
     immediately** — Azure never shows it again. Set the expiration to
     24 months, mark a calendar reminder for **30 days before expiry**.
   - Trusted Signing account → Access control (IAM) → Add role
     assignment → **Trusted Signing Certificate Profile Signer** →
     assign to the service principal.
5. **Populate the six GitHub Secrets** in the repo Settings → Secrets and
   variables → Actions:

   | Secret | Value source |
   |---|---|
   | `AZURE_TENANT_ID` | Azure AD → tenant GUID (Overview blade). |
   | `AZURE_CLIENT_ID` | App registration → Overview → Application (client) ID. |
   | `AZURE_CLIENT_SECRET` | Client secret value copied at creation time. |
   | `AZURE_TRUSTED_SIGNING_ENDPOINT` | `https://<region>.codesigning.azure.net/` |
   | `AZURE_TRUSTED_SIGNING_ACCOUNT` | Trusted Signing account name (e.g., `strivex-signing`). |
   | `AZURE_TRUSTED_SIGNING_CERT_PROFILE` | `PaneFlow-Release` (or whatever you named the profile in step 3). |

   > **Naming note.** The PRD US-024 acceptance criteria use the
   > shorthand `AZURE_TRUSTED_SIGNING_PROFILE`; the canonical name in
   > code, scripts, and this repo is `AZURE_TRUSTED_SIGNING_CERT_PROFILE`
   > (matches Microsoft's `metadata.json` field
   > `CertificateProfileName`). Keep the longer name when populating the
   > secret.
6. **Dry-run on a test tag.** Push `vX.Y.Z-rc1`, watch the Windows leg in
   `release.yml`. The `Verify MSI signature` step must emit the literal
   string `Successfully verified` — that is AC-6. Download the resulting
   `paneflow-X.Y.Z-x86_64-pc-windows-msvc.msi` artifact and on a clean
   Windows 11 VM confirm the SmartScreen prompt shows `Strivex` (not
   "Unknown Publisher"). New publishers build SmartScreen reputation
   over ~3,000 unique downloads or 6–8 weeks; an initial "Unknown
   Publisher" prompt is expected and not a signing failure.

## 4. Secret rotation

### `AZURE_CLIENT_SECRET` (every 24 months by default)

Azure AD client secrets expire — usually 24 months, shorter if
subscription policy tightens. Set a calendar reminder **30 days before
expiry**. The rotation procedure:

1. Azure AD → App registrations → `GitHub-ActionsSigning` → Certificates
   & secrets → New client secret. Copy the value immediately.
2. GitHub repo → Settings → Secrets and variables → Actions →
   `AZURE_CLIENT_SECRET` → **Update**.
3. Trigger a `workflow_dispatch` run (or push a test tag) to confirm CI
   succeeds with the new secret.
4. Once green, delete the old secret in Azure AD → Certificates &
   secrets → `Delete`. Leaving both active until the green run is the
   safety net.
5. Update the rotation date in `memory/project_windows_signing.md`.

### Signing certificate (automatic — ACME)

No manual rotation required. The Public-Trust certificate profile
issues a fresh leaf cert per sign operation via ACME (currently 3-day
lifetime). The dlib fetches the fresh cert per `signtool` invocation.
This is the principal reason Azure Trusted Signing was chosen over a
3-year EV cert: zero rotation operator-burden.

## 5. Failure-mode playbook

| Failure | Symptom | Recovery |
|---|---|---|
| **Timestamp server outage (Azure)** | `signtool sign` exits non-zero against `acs.microsoft.com`. | `sign-windows.ps1` automatically retries against `digicert.com` then `sectigo.com` (5 s backoff between attempts). If all 3 fail the leg fails; the Windows matrix entry is `continue-on-error: true` so Linux + macOS still ship. Manual re-sign + asset upload from a developer machine is the recovery path. |
| **Service principal secret expired** | `AADSTS7000215 Invalid client secret`. | Rotate per §4 above; re-run the workflow via `workflow_dispatch`. |
| **Cert profile deleted** | `CertificateProfile not found`. | Recreate the profile with the same name (`PaneFlow-Release`) in Azure Portal → Trusted Signing. The next sign picks up a fresh leaf via ACME. |
| **Azure subscription suspended / billing issue** | Sign step fails immediately (`continue-on-error` absorbs it). Linux + macOS ship without a Windows asset. | Resolve billing in Azure Portal → Cost Management. Re-run via `workflow_dispatch`. |
| **SmartScreen still flags "Unknown Publisher" after onboarding** | Users report SmartScreen warning even on signed builds. | Reputation builds over time. Per Microsoft, trust propagates after ~3,000 unique verified downloads OR within 6–8 weeks of consistent signing. **No action needed** — expected for the first month of a new publisher identity. |
| **Runner image dropped Windows SDK** | `signtool.exe not found`. | Add an explicit `microsoft/setup-msbuild@v2` or Windows 11 SDK install step to the Windows leg, mirroring the `Preflight WiX v3 toolchain` pattern. Pin the SDK version. |

## 6. OV-cert fallback (decoupled — local script ready)

If onboarding stalls > 6 weeks (PRD US-024 AC-4) or Azure Trusted
Signing becomes unavailable, fall back to an OV (Organization
Validation) code-signing certificate. **Per Microsoft's March 2024
SmartScreen change, OV and EV are now equal on reputation build-time**
— there is no longer a reason to pay the $300–580/yr EV premium.

The fallback is **not implemented in CI** — only a local-only signing
script (`scripts/sign-windows-ov.ps1`) and the operator runbook below.
Wiring CI for OV is a fork of the Sign MSI step that swaps
`/dlib ... /dmdf ...` for `/f <p12> /p $env:OV_CERT_PASSWORD`.

### Procurement

- **Vendor:** Sectigo (preferred, ~$150/yr) or DigiCert (~$400/yr).
- **Validity:** 1–3 years. The cert lives in a `.p12` (PKCS#12) file.

### Local signing

```powershell
# OV cert + password locally on a developer machine. NEVER commit the
# .p12 file to the repo — keep it in 1Password / Bitwarden, base64 in a
# GitHub Secret if/when CI wiring is added.
$env:OV_CERT_PATH = 'C:\path\to\paneflow-ov.p12'
$env:OV_CERT_PASSWORD = '<password from password manager>'
pwsh -NoProfile -File scripts/sign-windows-ov.ps1 `
    -InputFile target/wix/paneflow-X.Y.Z-x86_64.msi
```

### Future CI wiring (if Azure path stays unavailable)

If Azure Trusted Signing onboarding is permanently blocked, follow the
US-024-fallback story (to be opened then):

1. Procure OV cert.
2. Upload `.p12` as `OV_CERT_P12` (base64) and `OV_CERT_PASSWORD` GitHub
   Secrets — same pattern as macOS `APPLE_DEVELOPER_CERT_P12`.
3. Branch in `sign-windows.ps1`: `Azure path` when `$env:AZURE_CLIENT_ID`
   is set, `OV path` otherwise. Keep the same 3-server timestamp retry
   chain — the timestamp servers are vendor-neutral.
4. Replace this section with the canonical OV runbook.

## 7. Verifying releases (user-facing)

Users on Windows can verify a downloaded `paneflow-*.msi` themselves:

```powershell
# In an elevated PowerShell. signtool ships with the Windows 10/11 SDK.
signtool verify /pa /v paneflow-X.Y.Z-x86_64-pc-windows-msvc.msi
```

The output must contain:

- `"Successfully verified: paneflow-X.Y.Z-x86_64-pc-windows-msvc.msi"`
- A signer chain anchoring to `O=Strivex` (the canonical organization
  RDN — anchor on `O=`, not on a bare substring; a hostile clone could
  use `CN=Strivex-Evil-Clone`).

Alternatively, right-click the `.msi` → Properties → Digital Signatures.
The "Name of signer" should display `Strivex`.

## 8. Hardening backlog (post-US-024 follow-ups)

The US-024 security audit (2026-05-01) flagged three defense-in-depth
items that are deferred to future stories rather than wedged into this
PRD's scope. They are not blockers for the first signed release but
should be addressed before the pipeline is treated as production-grade
for high-value targets:

1. **Pin the SHA-256 of `Azure.CodeSigning.Dlib.dll`** after the NuGet
   fetch in `sign-windows.ps1`. The dlib intercepts the signing
   credential flow; a compromised `1.0.128` re-publish on nuget.org
   would silently land malicious signing code in CI. Mitigation: embed
   a pinned hash constant and `throw` on mismatch.
2. **Pin `nuget install -Source https://api.nuget.org/v3/index.json`**
   in `sign-windows.ps1` so a runner-level `nuget.config` change cannot
   redirect the resolution to a private feed.
3. **Add a `workflow_dispatch:` trigger to `release.yml`** so a
   transient timestamp-server outage that hits all 3 servers
   simultaneously can be retried without a new tag push.

Items 1 and 2 are low-frequency hardening (nuget.org SLA + NuGet
sub-supply-chain are both well-maintained). Item 3 is a 2-line
ergonomic change that affects every release target, so it lives outside
US-024's Windows-only scope.

## 9. References

- `scripts/sign-windows.ps1` — primary signing implementation.
- `scripts/sign-windows-ov.ps1` — local OV-cert fallback (not in CI).
- `.github/workflows/release.yml` Windows section (lines ~790–980) — CI
  orchestration.
- `memory/project_windows_signing.md` — Strivex-specific metadata
  (account names, expiration dates, rotation reminders).
- `tasks/prd-cmux-port-2026-q2.md` US-024 — acceptance criteria.
- Microsoft Learn — Trusted Signing:
  <https://learn.microsoft.com/en-us/azure/trusted-signing/>
- Microsoft Learn — `signtool sign`:
  <https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool>
