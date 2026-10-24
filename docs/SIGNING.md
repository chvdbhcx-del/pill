# Code signing

## Why Windows warns about the installer

`Pill_<version>_x64-setup.exe` is not Authenticode-signed, so Windows SmartScreen shows "Windows protected your PC / Unknown publisher" when it is downloaded and run. Signing only gives the file a verifiable publisher name. SmartScreen also uses download reputation of the file and the certificate, which builds up over time, and Microsoft says that even signed files can show the prompt until it has accumulated (see the Artifact Signing FAQ below, "SmartScreen prompts for signed files").

This is separate from the **updater signature** (`.sig` + `latest.json`, key in `%USERPROFILE%\.tauri`), which protects in-app updates and does not influence SmartScreen.

## Options

### 1. Leave it unsigned (current state)
Free. Users click "More info" and "Run anyway". winget and some antivirus tools may be more suspicious of unsigned installers.

### 2. SignPath Foundation (free for open-source)
Free signing of open-source releases, with a certificate issued to SignPath Foundation (so the publisher name shown is SignPath Foundation, not the maintainer). The published conditions are at <https://signpath.org/terms> (read them again before applying: they can change). As of this writing they include:

- every component under an OSI-approved licence (Pill is MIT), no proprietary parts, no malware or unwanted programs;
- the project is actively maintained and already released in the form that will be signed, and its functionality is described on the download page;
- the team that signs owns and develops the project and signs only artifacts built from its own source;
- data sent to other systems needs a privacy statement and a way to switch it off (Pill's network features are off by default; the README says so);
- multi-factor authentication for every team member on both SignPath and the source repository;
- defined roles (Authors, Reviewers, Approvers) and manual approval of each signing request;
- a "Code signing policy" section or link on the project home page (roles, privacy information, "Free code signing provided by SignPath.io, certificate by SignPath Foundation");
- binaries built verifiably from source with build scripts and CI configuration in the repo (this is what `ci-templates/release.yml` is for), with product name and version metadata fixed.

What the owner has to do: apply at <https://signpath.org/> (the application form is linked from the terms page), add the "Code signing policy" section to `README.md`, keep building releases in GitHub Actions, and install/configure the SignPath GitHub integration they send after approval. I am not certain of the exact application steps or the current integration details: follow <https://about.signpath.io/documentation/> and the instructions SignPath sends.

### 3. Azure Artifact Signing (formerly Trusted Signing), paid
Microsoft's managed signing service, billed through a paid Azure subscription (free, trial or sponsored subscriptions are not supported). It needs identity validation (organization or individual) and is only offered in supported countries/regions. It does not issue EV certificates. Pricing, eligibility and regions change, so check: <https://learn.microsoft.com/azure/artifact-signing/overview>, <https://learn.microsoft.com/azure/artifact-signing/faq>, <https://azure.microsoft.com/pricing/details/artifact-signing/>. There is a GitHub Actions integration, see <https://learn.microsoft.com/azure/artifact-signing/how-to-signing-integrations>.

### 4. A conventional certificate from a CA
Paid, yearly; the private key must live on a hardware token or cloud HSM, which is awkward in CI. Not evaluated here.

## What to change in `release.yml` once a service exists

1. Add the service's credentials as repository secrets (never in the repo).
2. Add a signing step in `ci-templates/release.yml` right after "Build and sign" and before "Write latest.json" (a comment marks the place). It signs `src-tauri/target/release/bundle/nsis/Pill_<version>_x64-setup.exe`.
3. Signing changes the bytes of the installer, so the updater signature made by `tauri build` no longer matches. Recreate it right after the signing step, in the same directory:
   ```powershell
   npx tauri signer sign "$exe"      # uses TAURI_SIGNING_PRIVATE_KEY[_PASSWORD] from the environment; writes "$exe.sig"
   ```
   ("Write latest.json" then reads the new `.sig`.) Check the exact flags with `npx tauri signer sign --help`.
4. Optionally also sign the app executable inside the installer; Tauri supports a custom sign command (`bundle.windows.signCommand` in `tauri.conf.json`, see <https://v2.tauri.app/distribute/sign/windows/>). That needs a change in `src-tauri/`, which is outside this file's scope.
5. Update `tools/release.ps1` the same way if you still release from a local machine.
6. Update `packaging/winget` hashes (`tools/winget-manifest.ps1` does this from the published asset).
