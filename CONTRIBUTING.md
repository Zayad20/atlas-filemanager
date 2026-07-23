# Contributing

Atlas is a Rust/GPUI desktop file manager.

## Local Check

Before opening a PR or commit you care about, run:

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

Strict clippy is tracked as cleanup work, but it is not a required gate yet:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

## Build Profiles

Use `cargo devfast` for the smallest feature surface, `cargo devfull` when the
default archive/provider features are required, and `cargo rel` for the
production artifact. Both development commands and plain `cargo run` share the
same `target/debug` profile. Optional nightly compiler experiments and the
repeatable `atlas-ui` benchmark are documented in [docs/build-performance.md](docs/build-performance.md).

## Vendored Code

Atlas keeps modified GPUI and gpui-component sources under `vendor/`.
Changes to those forks should be committed in this repo with the app change that
needs them.
