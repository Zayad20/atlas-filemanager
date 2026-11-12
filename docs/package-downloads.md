# Downloading Atlas builds

In GitHub, open **Actions**, choose the workflow run, then download its
**Artifacts** at the bottom of the page. GitHub requires sign-in for artifact
downloads. Extract the downloaded ZIP before using its contents.

## CI development binaries

Every successful Windows/Linux CI job uploads `atlas-dev-...`, containing the
application, companions, notices, build commit and SHA-256 checksum. Windows uses
a ZIP; Linux uses a tarball so executable permissions survive downloading.
Windows archives and installers include the app-local Microsoft C++ runtime.
These debug builds are for testing and are not optimized release installers.

## Optimized packages

Run **Actions → Packages → Run workflow** with a version from `CHANGELOG.md`.
Select both platforms, or just Windows/Linux when checking a platform-specific fix.
This creates downloadable candidate packages without publishing a GitHub release
or submitting anything to Microsoft Store, Flathub or Snap Store.

- `atlas-windows-installer-x86_64`: run `AtlasSetup-<version>.exe` for a normal
  Windows installation outside Microsoft Store. This includes the companion
  programs and rclone. The workflow verifies installation, startup and uninstall.
- `atlas-windows-msix-store-unsigned-x86_64`: MSIX for Store submission or signing
  with your own trusted production certificate. **It cannot be installed as-is.**
  Microsoft signs MSIX submitted through the Store. For the Store Free baseline,
  select `free` when running Packages; the selected edition applies to both
  Windows formats.
- `test-msix-windows-x86_64`: self-signed MSIX plus `AtlasDev.cer` (public key
  only). For deliberate local testing, review and run `Install-TestPackage.ps1`
  in an elevated PowerShell. This trusts the supplied certificate in Trusted
  People, then installs the package. It is not a production-trusted signature.
- `atlas-linux-x86_64`: DEB, RPM, Arch package, tarball, AppImage, AUR recipe,
  metadata and checksums, including the search daemon, cloud/icon programs and
  third-party notices. Builds target glibc 2.35 or newer. The Linux cloud backend
  locates rclone on PATH or downloads a checksum-verified copy on first use.
- `atlas-linux-flatpak` / `atlas-linux-snap`: locally installable store-format
  packages; they have not been submitted to their stores.
- `atlas-source`: matching source ZIP with locked dependency sources and a hash
  receipt. Keep it with the binaries when sharing a candidate.

Verify `SHA256SUMS` (Linux: `sha256sum --check SHA256SUMS`; Windows:
`Get-FileHash <file> -Algorithm SHA256`). Candidate artifacts expire according to
GitHub retention. Stable, permanent downloads belong in GitHub Releases.

Release tags run the same packaging workflows before publication. Existing
published releases are never overwritten by the workflow. Private signing keys,
test certificates and PDB symbol artifacts are not published as release assets.
