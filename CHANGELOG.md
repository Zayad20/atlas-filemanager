# Changelog

Write user-facing release notes here. Release headings use
`## [VERSION] - YYYY-MM-DD`; the workflow requires notes for the version it builds.

## [Unreleased]

## [0.1.5] - 2026-11-05

### New

- Settings redesigned as a searchable palette, with French and more languages selectable.
- Atlas Ember theme, theme editor improvements and new interface icons.
- Local file conversion with a radial menu, including FLAC, H.264/AAC video and optional Opus.
- Quick actions ring for converting and acting on selected files.
- Atlas Free and Atlas Premium. Premium unlocks with a license key, or in-app through the Microsoft Store in regional-price markets.

### Fixed

- Crash on launch caused by some shell extensions, such as Google Drive for desktop (#1, #3).
- Opening sub-folders on SMB and other cloud connections (#2).
- Typing into the fields of the add-connection dialog.

### Stability

- Crash reports with symbolized stacks that can be sent from the app.
- Recovery from worker failures and poisoned locks instead of closing the app.

## [0.1.0] - 2026-10-11

First public alpha.

### Highlights

- Native, GPU-accelerated file manager for Windows built with Rust and GPUI.
- Grid, List, Details, Miller columns and Tree views; tabs and split panes.
- Command palette, fuzzy go-to, indexed search and previews.
- Local file conversion, archives browsed as folders, tags and profiles.
- Optional first-party extensions (icon themes, thumbnails, Pandoc, Opus, cloud drives, document previews).
- Experimental Linux packaging (Debian/Ubuntu, Fedora/openSUSE, Arch, portable); not yet a supported release.
