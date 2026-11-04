<div align="center">

# Atlas

**A modern, blazing fast file manager built with Rust and GPUI.**

[![License: AGPL 3.0](https://img.shields.io/badge/License-AGPL_3.0-blue.svg)](https://opensource.org/licenses/AGPL-3.0)
[![Built with Rust](https://img.shields.io/badge/Built_with-Rust-orange.svg)](https://www.rust-lang.org/)

[![Join Discord](https://img.shields.io/badge/Join-Discord-5865F2?logo=discord&logoColor=white&style=for-the-badge)](https://discord.gg/aeAvQ27bgG)
[![Website](https://img.shields.io/badge/Website-atlasnative.com-18181B?style=for-the-badge)](https://atlasnative.com)

<br>
<br>

![Atlas Screenshot](website/public/images/app-screenshot.png)

</div>

Atlas is a desktop file manager for Windows and Linux designed for modern workflows, offering native stutter-free performance, robust extensibility, and a clean interface. Our mission is to build a highly productive file management experience, prioritizing speed, local privacy, and user customization.

## Features

- **Cross-platform**: natively supporting Windows and Linux (for now)
- **Featherweight**: extremely low RAM consumption and minimal system footprint
- **Native & blazing fast**: GPU-accelerated performance via Rust and GPUI
- **All of the eyecandy**: smooth scrolling, fluid animations, and a modern interface
- **Keyboard-first**: navigate and execute instantly via a powerful Command Palette
- **Highly extensible** with declarative file tools, preview providers, and native sidecars
- **Advanced multi-pane layouts**: split your view and drag-and-drop effortlessly
- **Five built-in views**: Grid, List, Details, Tree, and Miller Columns
- **Built-in archive manager**: native ZIP and RAR extraction/compression
- **Smart organization**: advanced filtering and custom color tags
- **A lot of customization**: switch between themes and user profiles instantly
- **Cloud ready**: seamless integration with remote storage providers
- **Native OS integration**: context menus, system drag-and-drop, and recycle bin support
- **100% local-first**: your files never leave your machine without your permission
- **Fast and active development**
- and much more...

## Installing and running Atlas

Pre-built binaries for your operating system are available in the [Releases](https://github.com/neondeex/atlas-filemanager/releases) section. Simply download and run the executable.

For Linux packages, native updates, store publishing, and the tested distributions,
see [Linux distribution](docs/linux-distribution.md).

## Building from source

To build and run Atlas locally, ensure you have [Rust](https://rustup.rs/) installed on your machine. Our repository includes a `rust-toolchain.toml` file, so `cargo` will automatically use the correct Rust version (1.95.0+) when you run these commands:

```powershell
# Run the application
cargo run --release

# Run checks and tests
cargo check --workspace
cargo test --workspace
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the workspace layout and dependency rules.
See [Alpha build editions](docs/build-editions.md) to enable the opt-in Free limits
with `cargo build --release --features free`.
See [docs/extensions.md](docs/extensions.md) for extension manifests, sidecar providers,
installation, and publishing.

## Contributing

We welcome contributions from the community. Whether you want to fix a bug, add a new feature, or improve documentation, your help is appreciated.

To get started, please join our [Discord server](https://discord.gg/aeAvQ27bgG) to discuss ideas with the community, or open an [issue](https://github.com/neondeex/atlas-filemanager/issues) to communicate your intent before creating a pull request.

## License

Atlas code is licensed under the [AGPL-3.0-or-later](LICENSE).
