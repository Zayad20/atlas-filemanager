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
```

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
