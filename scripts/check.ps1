# Local gate: compile + workspace tests.
# Clippy does not run here yet: atlas-ui still carries preexisting COM macro
# lint debt. Run it against touched files until that debt is paid down.
$ErrorActionPreference = "Stop"
cargo check --workspace
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test --workspace
exit $LASTEXITCODE
