# Local gate: compile + workspace tests.
# atlas-wasm-* crates have `test = false` because they only link for wasm32, so
# `cargo test --workspace` no longer tries to link extism on the host.
# Clippy does not run here yet: atlas-ui still carries preexisting COM macro
# lint debt. Run it against touched files until that debt is paid down.
$ErrorActionPreference = "Stop"
cargo check --workspace
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test --workspace
exit $LASTEXITCODE
