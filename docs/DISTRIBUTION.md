# Distribution: what is signed, and how to get Pill into winget and Scoop

Nothing here publishes anything. Every step that sends something to another service is marked **YOU** (done by the repository owner).

## 1. What is signed today

Audited from `docs/SIGNING.md`, `tools/release.ps1`, `.github/workflows/release.yml` (same as `ci-templates/release.yml` except the trigger) and `src-tauri/tauri.conf.json`. No key was read.

| Artifact | Signed? | With what | Effect |
|---|---|---|---|
| `Pill_<ver>_x64-setup.exe` (NSIS installer) | **No Authenticode signature** (confirmed: `Get-AuthenticodeSignature` on the v0.9.0 asset says `NotSigned`) | - | SmartScreen "Unknown publisher" on first run; some antivirus tools and the winget pipeline are more suspicious |
| `pill.exe` inside the installer, the uninstaller | No | - | same; `bundle.windows.signCommand` is not set |
| `Pill_<ver>_x64-setup.exe.sig` + `latest.json` | **Yes**, Ed25519 (Tauri updater key) | `TAURI_SIGNING_PRIVATE_KEY` (`%USERPROFILE%\.tauri\pill-updater.key` locally, a repository secret in CI) | The in-app updater refuses an installer whose signature does not match the public key in `tauri.conf.json`. Protects updates, does NOT affect SmartScreen |
| winget / Scoop manifests | Not applicable: hash only (SHA256 of the installer) | - | - |

Things the audit also found:

* `.github/workflows/release.yml` is manual-only (`workflow_dispatch`); `ci-templates/release.yml` also triggers on a `v*` tag. The workflow stops at its first step if the secret is missing.
* The Authenticode step is only a comment in the workflow (after "Build and sign", before "Write latest.json"). Signing changes the installer's bytes, so the updater `.sig` MUST be recreated afterwards (`npx tauri signer sign <exe>`), see `docs/SIGNING.md` steps 1-6.
* `tools/release.ps1` (local release) has the same gap and the same needed change.
* The README already tells users about the SmartScreen warning.
* `.gitignore` keeps `*.key` and `.env*` out of the repository.

## 2. Signing: what you have to do

Pick one; the details and eligibility rules change, so re-read the linked terms before applying. (`docs/SIGNING.md` has the long version.)

| Option | Cost | Publisher name shown | Fits Pill because | Catch |
|---|---|---|---|---|
| **SignPath Foundation** (free for open source) | free | "SignPath Foundation" | Pill is MIT and builds in GitHub Actions | application and review, MFA for all members, a "Code signing policy" section on the project page, manual approval per signing request, signs only what CI built from source |
| **Azure Artifact Signing** (formerly Trusted Signing) | about 10 USD / month for the basic plan (check the [pricing page](https://azure.microsoft.com/pricing/details/artifact-signing/)); billing starts when the account is created | your validated name | cloud HSM, a GitHub Actions integration, no token to carry | paid Azure subscription, identity validation, only some countries / regions (individual vs organization eligibility changes: check the [FAQ](https://learn.microsoft.com/azure/artifact-signing/faq)); no EV certificate |
| **OV certificate from a CA** | roughly 100-400 USD / year | your validated name | works anywhere | the private key must be on a hardware token or cloud HSM, awkward in CI |

SmartScreen reputation builds up over time for any option; a signed file can still warn at first.

Steps once an option is chosen (**YOU**):

1. Apply / create the account and finish identity validation (SignPath: application form on signpath.org; Azure: create the Artifact Signing account, a certificate profile and a service principal limited to signing).
2. Put the credentials into repository secrets (Settings > Secrets and variables > Actions). Never commit them, never print them in the workflow log.
3. Add the signing step to `.github/workflows/release.yml` and `ci-templates/release.yml` where the comment marks it, then re-create the updater signature: `npx tauri signer sign "$exe"` (needs `TAURI_SIGNING_PRIVATE_KEY`), before "Write latest.json". Optionally also sign `pill.exe` inside the installer with `bundle.windows.signCommand` in `tauri.conf.json` (a change in `src-tauri/`).
4. Do a test release to a draft / throwaway tag first; check `Get-AuthenticodeSignature Pill_<ver>_x64-setup.exe` says `Valid`, install it, and check that the in-app updater still accepts the next update.
5. Re-run `tools/update-manifests.ps1` (hashes change when the installer is signed).

## 3. winget

Templates: `packaging/winget/` (schema 1.6.0, `InstallerType: nullsoft`, silent `/S`, per-user). They already carry the real asset name `Pill_<ver>_x64-setup.exe`.

```powershell
pwsh tools/update-manifests.ps1 -Version 0.9.0 -Only winget    # or: no -Version = the latest release
winget validate --manifest packaging\winget\out\0.9.0           # passed for 0.9.0 on this machine
winget settings --enable LocalManifestFiles                     # once, elevated
winget install --manifest packaging\winget\out\0.9.0            # test in a VM / Sandbox
```

Submitting (**YOU**, first time):

1. Publish the release (`release-all`, or the manual workflow), so the asset URL answers.
2. Fork [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), copy the three files of `out/<ver>/` to `manifests/c/chvdbhcx-del/Pill/<ver>/`, open one pull request (one manifest per PR). The bot validates the schema, installs the package and scans it; fix what it reports. Read the current [CONTRIBUTING](https://github.com/microsoft/winget-pkgs/blob/master/CONTRIBUTING.md) and the [Microsoft submission guide](https://learn.microsoft.com/windows/package-manager/package/repository) first.
3. If an unsigned installer is flagged as malware by mistake, send it to Microsoft for analysis (the submission page of Defender) and answer in the pull request. Signing (section 2) makes this much less likely.
4. For later versions: `wingetcreate update chvdbhcx-del.Pill --version <ver> --urls <url> --submit` (needs a GitHub token with the rights wingetcreate asks for) or repeat step 2.

Nothing in the repository submits anything automatically.

## 4. Scoop

`packaging/scoop/pill.json` is a manifest for a bucket of your own (a repository `scoop-pill` or any `bucket/` folder). It runs the NSIS installer silently into Scoop's app folder (`/S /D=$dir`, `/D` must stay last), creates a Start-menu shortcut and uninstalls with `uninstall.exe /S`. `checkver` / `autoupdate` follow GitHub releases (the hash is computed by `scoop` / `checkver.ps1 -u`).

```powershell
pwsh tools/update-manifests.ps1 -Version 0.9.0 -Only scoop      # writes packaging\scoop\out\0.9.0\pill.json
scoop install packaging\scoop\out\0.9.0\pill.json              # test in a VM / Sandbox
```

**UNVERIFIED - check live:** that the Tauri NSIS installer accepts `/D=` in per-user mode, that the uninstaller is named `uninstall.exe` there, and that the `pill.exe` shortcut target is right. If the install ends in `%LOCALAPPDATA%\Pill` instead, drop `/D=$dir` and `shortcuts`, and describe the package as "runs the installer" (the in-app updater then also keeps the app current; Scoop will only know the first version).

Publishing (**YOU**): create a bucket repository, copy `pill.json` to `bucket/pill.json`, tell users `scoop bucket add pill https://github.com/<owner>/scoop-pill` then `scoop install pill`. Submitting to the official `extras` bucket needs the project to meet its criteria (notable, signed or well-known): check [its README](https://github.com/ScoopInstaller/Extras) first.

## 5. Per-release routine (after the signing is in place)

1. `release-all` (publishes the GitHub release).
2. `pwsh tools/update-manifests.ps1` (latest release).
3. winget: `wingetcreate update ...` or a pull request. Scoop: copy `out/<ver>/pill.json` into the bucket (or let the bucket's autoupdate do it).
