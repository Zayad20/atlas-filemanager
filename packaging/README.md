# Packaging & OS integration

Atlas ships as a single Rust binary. The application UI lives in `atlas-ui`
(`gpui` + `gpui-component`), while filesystem, cloud, extension and search backends
live in `atlas-core`; platform-specific code is `#[cfg]`-gated so the same workspace
builds on Linux, Windows and macOS.

> **Important about "default file manager" / "Save As":** a third-party app **cannot**
> replace the system *Save As* / open panels on macOS or Windows — those dialogs are
> owned by the OS. What *is* possible, and what the manifests here enable, is: appearing
> in **"Open With"**, **associating file types**, registering as a **folder handler**,
> and (on Linux) being set as the **default file manager**.

## Build the release binary

```sh
cargo build --release        # target/release/atlas (rename to atlas)
```

The SQLite tag store is statically linked (rusqlite `bundled`), so there is no external
libsqlite dependency on any platform.

## Linux

```sh
# Bundle (.deb) with cargo-bundle, or just install the binary + desktop entry manually:
install -Dm755 target/release/atlas           ~/.local/bin/atlas
install -Dm644 packaging/linux/atlas.desktop ~/.local/share/applications/atlas.desktop

# Make Atlas the default file manager (handler for directories):
xdg-mime default atlas.desktop inode/directory
```

An AppImage can be produced with `linuxdeploy`/`appimagetool` over the same binary +
`.desktop` file.

## macOS

```sh
cargo install cargo-bundle
cargo bundle --release        # produces target/release/bundle/osx/Atlas.app
```

Merge the `CFBundleDocumentTypes` block from `packaging/macos/Info.plist` into the
generated `Atlas.app/Contents/Info.plist` (cargo-bundle writes the base keys from
`[package.metadata.bundle]` in `Cargo.toml`). After installing to `/Applications`, macOS
will list Atlas under Finder's **Open With** for folders and files.

## Windows

```powershell
cargo build --release          # target\release\atlas.exe -> rename atlas.exe
```

Edit the install path inside `packaging/windows/atlas-register.reg`, then import it:

```powershell
reg import packaging\windows\atlas-register.reg
```

This adds **"Open with Atlas"** to the context menu of folders and folder
backgrounds, and registers Atlas in the **"Open with"** application picker so file
types can be associated with it.

### Installer prototype

Install [Inno Setup](https://jrsoftware.org/isinfo.php), then build the installer:

```powershell
.\scripts\build-windows-installer.ps1 -Version 0.1.0
```

The script builds `target\installer\AtlasSetup-0.1.0.exe`, with optional tasks for
desktop shortcut, `atlas` in Terminal, Explorer integration, and launch-after-install.
Publish it to the configured R2 bucket:

```powershell
.\scripts\publish-windows-installer.ps1 -Version 0.1.0 -Notes "First public build"
```

That uploads the installer to `app/windows/versions/` and writes
`app/windows/latest.json`. The in-app updater only needs to read that tiny manifest.

## Icons

Drop platform icon files into `packaging/icons/` (`atlas.icns` for macOS,
`atlas.ico` for Windows, a PNG named `atlas` for the Linux theme) and uncomment
the `icon = [...]` line in `[package.metadata.bundle]`.

## CI note

Real Windows/macOS binaries must be built on their own runners (the `rusqlite` bundled
SQLite needs a native C toolchain per target). A matrix CI (`ubuntu-latest`,
`windows-latest`, `macos-latest`) running `cargo build --release` is the recommended way
to produce all three.
