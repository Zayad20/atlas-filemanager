# Alpha build editions

Edition restrictions are opt-in at compile time. There is no settings-file or
environment-variable override.

```powershell
# Full edition: current behavior, without Free limits.
cargo build --release

# Free public alpha, with the usual default features (including cloud support).
cargo build --release --features free

# Run a Free development build.
cargo run --features free

# Lean Free build, without the launcher's optional default features.
cargo build --release --no-default-features --features free
```

The same `free` feature is exposed by `atlas-ui`, `atlas-core`, and `atlas-model`
for direct package builds. The launcher forwards it through all three crates.
Cargo features are additive: enabling `free` anywhere in the app's dependency
graph applies the Free policy throughout that build. Do not enable `free` in a
Full release job.

## Premium entitlement (runtime)

The `free` feature only sets the baseline. A verified entitlement raises a Free
build to Full while it runs (`atlas_model::edition::current()`); nothing else can,
and the baseline never changes. Two sources are checked at startup and on activation:

| Source | How it unlocks | Notes |
| --- | --- | --- |
| Microsoft Store add-on | `StoreContext` app license contains an active add-on whose Product ID (in-app offer token) is `atlas_premium` | Packaged builds only; unpackaged builds report no Store entitlement. |
| Polar license key | `atlas_core::license::activate` binds the key to this device through Polar's public customer-portal endpoints | Needs `ATLAS_POLAR_ORG_ID` at build time (the MSIX script defaults it to the public Atlas org id). Validated on every launch; works offline for 14 days after the last successful check. |

Key flow: copy the key from the Polar email, run **Activate license key** from the
command palette (reads the clipboard), run **Deactivate license on this device**
to free the slot for another PC. The saved key lives in `license.json` in the
config directory. Device limits (`limit_activations`) are set per product in the
Polar License Keys benefit, not in the app.

Store package build (Free baseline + add-on):

```powershell

.\scripts\build-windows-msix.ps1 -Version <next version> -Edition free
```

`-Edition full` keeps the previous behavior (everything unlocked, no gating).

| Capability | Free build | Full build / Premium |
| --- | --- | --- |
| Saved cloud / SFTP / FTP connections | 1 | No edition limit |
| Profiles | 2; creation and duplication blocked once reached | No edition limit |
| Side-by-side panes (tab containers, not tabs) | 2; further splits blocked | No edition limit |
| Batch rename | Up to 25 selected files per operation | No edition limit |
| Conversion | 1 source file per operation and 10 converted files per local calendar day | Batch conversion, no daily limit |

The daily conversion counter is `usage.json` in the config directory. It is a soft
limit by design (local, AGPL source) and only counts files that were actually
created; it resets at local midnight. Premium builds never read or write it.

Limits reject the whole action before it changes state or files. The UI explains
the restriction using an informational toast. Conversion plans and the executor
also enforce the policy, including manually constructed plans.

An optimistic cloud connection counts toward the limit while authorization is in
progress. Failed connections can be removed before adding a replacement. Existing
profiles and connections created with a Full build are kept and remain accessible
in a Free build; the limits prevent adding more. Nothing is silently deleted,
truncated, or locked away. Switching and deleting existing profiles, renaming or
removing connections, individual conversions, and individual renames still work.

Tabs, views, search, basic file operations, previews, themes, vaults, and Drop Zone
are unchanged. Only the five limits above are enabled. Existing panes, profiles and
connections above the limit are never closed or hidden; the limits only stop adding more.

## Verification

```powershell
cargo test -p atlas-model edition::tests
cargo test -p atlas-model --features free edition::tests
cargo test -p atlas-core --lib edition_
cargo test -p atlas-core --lib --features free edition_
cargo check -p atlas
cargo check -p atlas --features free
cargo check -p atlas --no-default-features --features free
```
