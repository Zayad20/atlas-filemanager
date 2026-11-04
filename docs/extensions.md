# Atlas extensions

The feature guide's **Add extensions** topic opens Extension Manager and explains
marketplace and local ZIP/folder installation. Its inventory uses the loaded
manifests for names, descriptions, versions and totals. Installed includes visible
tool providers such as Pandoc and Opus; hidden shared runtime tools are counted
separately in the guide. Marketplace totals come from the remote registry and
can differ from the bundled inventory.

DOCX text previews work without extensions. Document Previews uses the same native
reader, which preserves paragraph/run text, XML entities, tabs and line breaks.
This is a text preview rather than Word page-layout rendering. DOCX files inside
archives also use this reader. Changing providers invalidates the selected preview
so the file is retried without needing to select another file first.

To compare bundled metadata against a downloaded marketplace registry:

```powershell
py -3 scripts/audit-extension-catalog.py --registry registry.json --output audit.json
```

The audit reads local files only; it does not publish packages. Updating source
manifests does not update the remote registry; use the publishing workflow below
after building the corresponding package and sidecar assets.

Atlas extensions live in the user data directory under
`extensions/<extension-id>/`. Each extension contains an `extension.toml`.

## Minimal manifest

```toml
id = "image-tools"
name = "Image Tools"
version = "0.1.0"
author = "Example"
description = "Image conversion commands"
kind = "declarative"

[[commands]]
id = "convert-png"
title = "Convert to PNG"
requires = ["magick"]
per_file = true
accepts = { extensions = ["jpg", "jpeg", "webp"] }
run = "magick {input} {dir}/{stem}.png"
```

`id` and command IDs must remain stable. `accepts.extensions` uses lowercase
extensions without a dot; `dirs = true` accepts folders and `any = true`
accepts everything.

## Extension kinds

- `declarative`: executes the tokenized `run` template without a shell.
- `builtin`: implemented inside Atlas.
- `tool_provider`: hidden package that supplies approved external tools.
- `sidecar`: an isolated executable. Provider sidecars use the versioned JSON-lines
  API; command sidecars receive one request on stdin and return one response on stdout.

Declarative templates support `{input}`, `{dir}`, `{stem}`, `{ext}`, `{output}`
and `{prompt:NAME}`. Every executable named by a template must also appear in
that command's `requires` list. Atlas rejects undeclared programs.

## Distribution and security

Installing a local folder or ZIP is an explicit user action. Marketplace ZIPs
and downloadable tools require a lowercase or uppercase SHA-256 digest in their
registry metadata; Atlas verifies it before writing or extracting anything.
Mutable `latest` download URLs should not be used with pinned hashes.

Publish a marketplace extension with:

```powershell
cargo run -p atlas-publisher -- publish .\path\to\extension `
  --bucket $env:R2_BUCKET --public-url $env:R2_PUBLIC_URL
```

The publisher packages the directory, uploads an immutable versioned ZIP, and
writes its SHA-256 digest to `registry.json`.

Build the official Cloud sidecar and its pinned rclone payload with:

```powershell
.\scripts\build-cloud-extension.ps1
cargo run -p atlas-publisher -- publish .\target\extensions\cloud `
  --bucket $env:R2_BUCKET --public-url $env:R2_PUBLIC_URL
```

On Windows, install the complete `target/extensions/cloud` folder or
`target/extensions/cloud.zip` produced by the build script. It includes `extension.toml`, `atlas-cloud-extension.exe`,
`rclone.exe`, the icon and third-party notices. Installing only the manifest or
an older executable leaves Cloud unavailable. Browsing and transfers need no
separate system installation of rclone. WinFsp is needed only for mounting a
remote as a Windows drive; Atlas offers its installation when mounting.

The host and provider use a versioned protocol. `build-release.ps1` and
`run-release-perf.ps1` build the provider alongside the host so a fresh host does
not reuse an incompatible Cloud executable. The Windows installer also carries
the provider and the verified rclone payload. When starting Cloud, Atlas copies
these companion binaries into the extension directory.

Before publishing, verify that the extension:

- performs only functionality described in its manifest and Store listing;
- requests the minimum tools and file access needed;
- contains no secrets;
- handles malformed input and failed host calls;
- uses a new version and an immutable download artifact.

Native sidecars are reserved for reviewed, trusted integrations. Untrusted
third-party code is not supported until the sandbox runtime is restored.
