# Signing Selora IDE releases

This document lists the signing material the repository owner must provide.
Nothing here creates, generates, or stores a real key. The build is unsigned
until the owner supplies a certificate.

## What the release workflow does

`.github/workflows/release.yml` signs two files with `signtool.exe` when
signing is enabled:

1. `target/release/selora-desktop.exe`, before the installer is compiled.
2. `installer/output/Selora-IDE-Setup-<version>.exe`, after the installer is
   compiled.

Each signature uses SHA-256 and an RFC 3161 timestamp from
`https://timestamp.digicert.com`. The workflow then runs
`signtool verify /pa /v` on the file. A failed or unverifiable signature stops
the release.

The installer verification step also checks the expected state:

- Signing enabled: `Get-AuthenticodeSignature` must report `Valid`.
- Signing disabled: it must report `NotSigned`. A half-signed artifact fails
  the release.

## Required repository secrets

Add these as **Actions secrets** (Settings > Secrets and variables > Actions >
Secrets):

| Secret | Contents |
| --- | --- |
| `SIGNING_CERT_BASE64` | The Authenticode code-signing certificate and private key, exported as a `.pfx`/`.p12` file, then base64-encoded as one string. |
| `SIGNING_CERT_PASSWORD` | The password that protects that `.pfx` file. |

Generate the base64 value on a trusted machine, never in the repository:

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes('C:\path\to\selora-signing.pfx'))
```

Paste the single-line output into `SIGNING_CERT_BASE64`. Do not commit the
`.pfx`, the password, or the base64 string. `.gitignore` already excludes
`*.pfx`, `*.pem`, and `*.key`, but the secret must never reach a tracked file.

The certificate must be a Windows Authenticode code-signing certificate with
an exportable private key. A hardware-token or EV certificate that cannot be
exported as a `.pfx` does not work with this pipeline; use a file-based
certificate or change the workflow deliberately.

## Required repository variable

Add this as an **Actions variable** (Settings > Secrets and variables >
Actions > Variables):

| Variable | Value | Effect |
| --- | --- | --- |
| `SIGNING_REQUIRED` | `true` | A missing `SIGNING_CERT_BASE64` or `SIGNING_CERT_PASSWORD` becomes a hard failure. Leave it unset or any other value to allow unsigned builds with a warning. |

Set `SIGNING_REQUIRED=true` once releases are public. Without it, the workflow
silently produces an unsigned installer when the secrets are absent.

## Behaviour matrix

| `SIGNING_CERT_BASE64` | `SIGNING_CERT_PASSWORD` | `SIGNING_REQUIRED` | Result |
| --- | --- | --- | --- |
| empty | any | not `true` | Unsigned installer; workflow logs that signing is disabled. |
| empty | any | `true` | Release fails before building. |
| set | empty | any | Release fails: a certificate without a password is unusable. |
| set | set | any | Binary and installer are signed and verified. |

## Operational notes

- The certificate is written to `%RUNNER_TEMP%\selora-signing.pfx` and removed
  in a `finally` block. It is never uploaded as an artifact.
- Renew the certificate before it expires and replace both secrets. The
  workflow does not check the certificate's expiry date in advance;
  `signtool verify` fails if the timestamp or chain is not valid.
- Rotate the password by uploading a new `.pfx` and password together. They
  must match.
- Verify a downloaded installer locally with:

```powershell
Get-AuthenticodeSignature .\Selora-IDE-Setup-<version>.exe | Format-List Status, SignerCertificate, TimeStamperCertificate
```

## Why this matters for updates

The in-app updater requires a valid Authenticode signature before it hands an
installer to Windows. An unsigned release can be downloaded and installed
manually, but it cannot be installed by the automatic update path. Enable
signing before relying on updates.