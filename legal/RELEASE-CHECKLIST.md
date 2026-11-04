# Atlas legal artifacts and release checks

The existing workspace software license is AGPL-3.0-or-later. This work preserves
that license; it does not silently convert Atlas or its dependencies to proprietary
software. A commercial plan can charge for delivery, support and entitlements,
but it cannot remove rights granted by an applicable open-source license.

## Authored documents

`legal/documents.json` is the reviewed source for privacy, website terms and the
third-party introduction. `legal/privacy-choices.js` supplies the shared privacy
controls. Run `python scripts/sync-legal-documents.py` from the repository root
after editing either source. The React routes, their data, the static HTML pages
and both copies of the privacy controls are generated. Dates are explicit,
not the current date of each page request.

The published policy and terms must change together with the implementation.
The hosting provider's security/essential processing remains independent from
the optional PostHog/OpenPanel choice. Confirm the actual retention settings
in those services before publishing a specific retention period; none was invented.
The operator is identified using the current Store publisher, Cuber Labs, and
the support address supplied by the owner. Confirm those details before publishing.

## Reproducible dependency inventory

1. Install locked Cargo and npm dependencies. Use `cargo fetch --locked` and
   `npm ci --ignore-scripts` in `website` on a trusted build machine.
2. Run `python scripts/generate-third-party.py` to audit the workspace with all
   features and classify the default Windows release graph. Build/dev/optional
   entries are explicitly labeled and are not assertions that everything ships.
3. For upstream packages lacking a standalone license file, run
   `python scripts/fetch-license-notices.py` to retrieve notices at the recorded
   source revision. Review the downloaded files and provenance.
4. Where a published artifact itself omits a license file,
   `python scripts/complete-license-evidence.py` preserves its own declaration,
   any embedded notices and the standard texts for its declared SPDX alternatives.
   These entries identify their basis in `notice-overrides.json`; they are not
   claimed to be publisher-supplied standalone files. Nonstandard/missing license
   expressions are not guessed. Re-review these entries when upgrading.
5. Run `python scripts/prepare-asset-notices.py` when updating fonts, icons or
   explicit bundled tools (requires fontTools). This records the actual font
   copyright/version metadata and hashes of shipped artwork.
6. Regenerate the dependency notices after any evidence/resource change.
7. Run `node scripts/check-legal.mjs`. Web build/deploy hooks and Windows
   packaging check that the recorded inputs, resource hashes and distributed
   notices remain synchronized. The checker does not replace legal review.

The generated inventory covers Cargo workspace metadata, the installed locked
website dependency tree, the explicit edge-site JS bundle, fonts and artwork.
It is an inventory of these inputs, not a universal audit of every executable
that an optional extension may later download. Preserve all licenses/notices in
third-party tool distributions and audit their own bundled dependencies when
updating or redistributing them.

## Distribution obligations requiring source

The inventory identifies AGPL Atlas packages, the GPL-3.0 `winfsp` and
`winfsp-sys` Rust bindings, and MPL components including nucleo and option-ext.
Notices alone do not satisfy their corresponding-source obligations.

Before distributing a new release under the current licenses:

- Provide the exact corresponding source and required build/install materials
  to recipients through a license-compliant distribution method. Retain the
  source snapshot matching each binary, including local vendor modifications.
- Preserve third-party copyrights and state relevant modifications. Do not
  replace upstream licenses with a blanket Atlas commercial EULA.
- A future public-beta repository is not a substitute for source rights owed
  to recipients of an already distributed copyleft build.
- Do not promise a closed-source build without replacing the GPL dependencies
  or obtaining all needed rights from the relevant holders. The WinFsp driver
  publisher's commercial license does not automatically relicense the separate
  winfsp-rs bindings.

The owner has chosen to retain the current copyleft licenses and distribute matching release source. Prepare it with scripts/build-source-release.py, verify it with scripts/verify-source-release.py, and publish the ZIP plus its JSON SHA-256 receipt before deploying source links or distributing the corresponding MSIX. Standalone Icons8 PNGs are excluded; the included helper recovers exact build inputs from a recipient's matching binary under their existing resource license.

## WinFsp and creative resources

The normal Atlas UI offers an explicit Download and install WinFsp action. It downloads the pinned official MSI, checks SHA-256 and invokes Windows Installer with a progress interface and no forced restart. The user must approve any Windows elevation prompt. Atlas retries mounting after success. Cancellation preserves the session; restart-required codes display restart instructions. Basic Atlas operation remains available without the driver.

WinFsp's FLOSS exception and the Rust bindings' GPL are distinct. Preserve the
WinFsp copyright/repository credit in the UI and documentation. Declare the optional driver and its requested download/install flow in Store certification notes. Store policy 10.1.5 excludes non-Microsoft drivers from normally permitted add-on acquisition, and 10.2.4 limits external driver dependencies. This assisted installer therefore requires explicit certification assessment; generating an MSIX or including source does not establish Store approval.

Icons8's free desktop attribution guide requires an About link. Both settings
renderers retain/add that credit. Public pages using its artwork also include
an Icons8 link. Attribution does not authorize excluded formats, uses or edits;
keep each downloaded resource's applicable terms and any purchase evidence.

## Store release notes and remaining runtime checks

Use `packaging/windows/msix/CERTIFICATION-NOTES.md` as the concrete starting point
for the next submission. Review its factual assumptions against the exact binary.
The new packaging scripts include both provider companions, rclone, Atlas's
license and complete generated notices. The old submitted MSIX is unchanged.

Before uploading: install the new package in a clean profile; exercise cloud
setup, assisted WinFsp installation, private-folder access, copying, overwriting,
canceling, cleanup and uninstallation; verify paid/Free entitlement wording;
run WACK on the final package. Verify the source delivery method as described
above. Do not cancel the current certification or upload a replacement merely
by running the legal generators.

The normal Store hardware/declarations review remains in
`docs/store-review-2026-10-30.md`. User-supplied pasted text did not expose the
selected product-declaration/hardware checkboxes, and this work did not edit them.
