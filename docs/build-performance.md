# Build performance

Atlas keeps production builds on the pinned stable Rust toolchain and offers
nightly only as an opt-in development experiment.

## Daily commands

```powershell
# Fastest stable edit/run loop; excludes Atlas' default feature set.
cargo devfast

# Same development profile with default Atlas features.
cargo devfull

# Nightly LLVM with the experimental parallel frontend (-Zthreads=8).
powershell -NoProfile -File scripts/run-dev-toolchain.ps1 -Backend llvm

# Nightly LLVM with all default features.
powershell -NoProfile -File scripts/run-dev-toolchain.ps1 `
  -Backend llvm -Mode devfull

# Nightly Cranelift experiment. Do not use this to measure final GPUI speed.
powershell -NoProfile -File scripts/run-dev-toolchain.ps1 -Backend cranelift

# Nightly Cranelift with all default features.
powershell -NoProfile -File scripts/run-dev-toolchain.ps1 `
  -Backend cranelift -Mode devfull

# Fully optimized production build.
cargo rel

# Equivalent explicit release script: stable LLVM + rust-lld.
powershell -NoProfile -File scripts/build-release.ps1

# Build and immediately launch the stable release for manual testing.
powershell -NoProfile -File scripts/build-release.ps1 -Run

# Fast iterative release-like build with split/frame perf logs enabled.
# Keeps opt-level=3 and stable LLVM, but skips LTO and uses 16 codegen units.
powershell -NoProfile -File scripts/run-release-perf.ps1

# Same perf logs using the exact definitive release profile (slower).
powershell -NoProfile -File scripts/run-release-perf.ps1 -Exact

# Optional nightly LLVM release, isolated from target/release.
powershell -NoProfile -File scripts/build-release.ps1 -Nightly

# Build and launch the nightly LLVM release.
powershell -NoProfile -File scripts/build-release.ps1 -Nightly -Run

# Optional lean release without archive/provider features.
powershell -NoProfile -File scripts/build-release.ps1 -Lean
```

PowerShell script parameters use a space, not `=`: write
`-Backend cranelift -Mode devfast`, not `-Backend="cranelift"`.

`build-release.ps1` always selects LLVM; omit `-Nightly` for the supported
stable release. Cranelift intentionally has no release mode.

`run-release-perf.ps1` also stays on the pinned stable LLVM toolchain. Its
default `release-perf` profile writes to `target/release-perf`, enables
`perf-log`, `ATLAS_PERF`, and `ATLAS_PERF_EVENTS`, and uses the persisted Cargo
incremental cache. Pass `-ThresholdMs 0` to emit every timed span, `-NoEvents`
to suppress Atlas event/frame summaries, or `-BuildOnly` to compile without launching.
By default it also sets `GPUI_FRAME_LOG=0` and `RUST_LOG=info`, and saves the
combined Cargo/Atlas/GPUI output to `perf-logs/release-perf-<timestamp>.log`.
Use `-NoGpuiFrames` or `-NoFileLog` for quieter/terminal-only runs, or pass
`-LogPath path\to\session.log` to choose the file.
For A/B isolation, `-SkipSidebar`, `-SkipOverlays`, and `-SkipInspector` map to
the corresponding `ATLAS_PERF_SKIP_*` variables and are cleared on normal runs.
Use `-Exact` before trusting absolute runtime numbers: that selects the normal
Thin-LTO, one-codegen-unit release profile while keeping the logs compiled in.
The log header records the effective values, and every dock-only render emits a
`dock_frame` event with `tiles`, `panes`, `split_animations`, `pane_animations`,
and `closing`. `frame_summary frame=` now identifies the completed App frame;
`sampled_at=` is the render in which that summary was observed.

`cargo run`, `devfast`, `devfull`, and `devperf` share `target/debug`, although
Cargo may retain multiple feature variants there. Nightly LLVM and Cranelift
use persistent, isolated caches under `target/toolchains/`; their object files
are incompatible and sharing one directory would cause rebuilds instead of
saving space.

## Benchmarking atlas-ui

The benchmark warms dependencies, removes only the `atlas-ui` and launcher
artifacts, rebuilds them, and writes Cargo timing reports plus CSV/JSON results.

```powershell
# Three stable samples with the normal development codegen setting.
powershell -NoProfile -File scripts/measure-build-performance.ps1 `
  -Configuration stable -Runs 3 -CodegenUnits 64

# Compare codegen-unit counts.
powershell -NoProfile -File scripts/measure-build-performance.ps1 `
  -Configuration stable -Runs 3 -CodegenUnits 256,128,64,32

# Experimental parallel frontend and Cranelift.
powershell -NoProfile -File scripts/measure-build-performance.ps1 `
  -Configuration nightly-llvm -Runs 3 -CodegenUnits 64
powershell -NoProfile -File scripts/measure-build-performance.ps1 `
  -Configuration nightly-cranelift -Runs 3 -CodegenUnits 64
```

Results are ignored build artifacts under `target/build-benchmarks`. Remove an
experimental cache without affecting stable builds:

```powershell
Remove-Item -Recurse -Force target/toolchains/stable-1.97.1
Remove-Item -Recurse -Force target/toolchains/nightly-llvm
Remove-Item -Recurse -Force target/toolchains/nightly-cranelift
```

Rust's parallel frontend and Cargo's `codegen-backend` setting are unstable.
Nightly and Cranelift must remain optional; release builds continue to use LLVM,
Thin LTO, and `rust-lld`.

## Current release-perf diagnosis

Measured on 2026-07-29 with 16 logical processors:

| Scenario | Total | `atlas-ui` | Interpretation |
| --- | ---: | ---: | --- |
| Cold `release-perf` build | 283.3 s | 54.6 s | `windows`, `image`, and `ravif` dominate the initial build |
| Warm dependencies, only `atlas-ui` rebuilt | 45.05 s | 41.8 s | `atlas-ui` is 93% of the daily local rebuild |
| No source changes | 1.13 s | fresh | The persistent cache is working |

In the warm sample, `atlas-ui` spent 10.8 s in the frontend and 31.0 s in
codegen. Cargo used 16 jobs and reached 17 concurrent units. Stable Rust already
uses the machine's logical processors for Cargo jobs; nightly `-Zthreads=8`
controls the experimental rustc frontend instead and is not required for this
profile.

The lowest-churn crate split is a new `atlas-win-shell` leaf crate containing
the native `shell_menu`, `shell_thumb`, and OLE DnD backend. Together those
modules are about 104 KB and can remain fresh while GPUI code changes. Keep the
GPUI context-menu and drop-zone adapters in `atlas-ui`; communicate through
plain request/event/data types.

Do not directly extract `sidebar`, `inspector`, `chrome`, dock rendering, or the
large action modules yet. They add inherent `impl App` blocks and access private
`App` state, so moving them across a crate boundary would require broad public
APIs and high churn. First convert a large area to a component with narrow state
and callbacks, then extract that component if timings justify it. Pure motion,
glass, surface, and icon primitives are a possible second leaf crate after
removing any `WeakEntity<App>` dependencies.

Cross-crate imports do not prevent navigation: rust-analyzer and CodeGraph
follow definitions and call paths. Directly named public types and module docs
usually make agent navigation easier; avoid opaque facades, unnecessary traits,
and generic indirection at these boundaries.

## Release size

The full Windows release executable measured 27.36 MiB uncompressed and 12.73
MiB with gzip-equivalent compression. The latter matches the previously seen
10–14 MiB range for a compressed package. Building with
`--no-default-features` produced 26.54 MiB, so archive/provider features account
for only about 0.82 MiB. The raw executable is mostly linked machine code
(`.text`) and read-only data (`.rdata`), not debug symbols.

## Reference results

Measured on a Ryzen 7 5800X (16 logical processors), rebuilding only
`atlas-ui` and the launcher after dependencies were warm:

| Configuration | Median total | Median `atlas-ui` |
| --- | ---: | ---: |
| Stable 1.97.1, 256 codegen units | 26.85 s | 19.4 s |
| Stable 1.97.1, 64 codegen units | 22.25 s | 16.9 s |
| Nightly LLVM, 8 frontend threads, 64 units | 17.39 s | 11.8 s |
| Nightly Cranelift, 8 frontend threads, 64 units | 14.23 s | 9.4 s |

`64` is therefore the checked-in development setting. Nightly LLVM is the
recommended optional experiment; it improved the total median by about 22%.
Cranelift improved compilation further but remains diagnostic because its
generated code is not representative of release runtime performance.
