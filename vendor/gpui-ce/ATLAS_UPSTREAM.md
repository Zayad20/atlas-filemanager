# GPUI CE in Atlas

Source: https://github.com/gpui-ce/gpui-ce
Revision: `175ef66578817bf96b2e26b0cd568dd4f4793529` (pinned, September 2026).
Rust requirement: 1.95 or newer.

Atlas and its component fork resolve to the same `gpui-ce` package through the
root Cargo patches. Keep the revision aligned in both manifests. The previous
Zed checkout remains as reference material; it is no longer a runtime dependency.

## Atlas compatibility patches

- Paint-only offsets and glyph/SVG scales: preserve final pane layout during
  split animations. Cached scene replay translates geometry, masks, layer bounds,
  backdrop filters and filter groups, and preserves surface opacity.
- Cached view replay retains input listeners and text layout data while moving.
- Bounded text layout reuse, plus allocation-free monochrome text truncation.
- Smooth wheel scrolling for tracked divs and virtual lists, preserving CE's
  axis filtering. Direct `set_offset` cancels a pending smooth scroll.
- Opt-in layout region counts and `GPUI_FRAME_LOG` thresholded frame diagnostics.
  CE's profiler remains available for the detailed renderer/phase measurements.
- Workspace paths made explicit for use as a nested vendored dependency;
  AccessKit 0.24.1 matches the Windows adapter's dependency.

FastPaneShell and the optional monolithic pane chrome paint the child bounds
retained by Drawable during prepaint; no paint-time layout queries are needed.

## Application integration

Native `Animated`/`SpringConfig` retain sidebar and inspector state and allow
retargeting during motion. Inspector teardown follows solver completion; its
content retains a fixed width during the reveal. Reduced motion and direct
resizing jump to the target. Native Motion drives split progress and alternating
drag-hover pulses. Style transitions smooth hover feedback, and SpringAnimation
drives selection checkboxes. Glass surfaces use bounded 12 px backdrop blur and
smoothed corners. Tab drag sources use CE's `AnyDrag` API.

Atlas enumerates every embedded icon for CE's on-demand asset registry.

The component fork uses Palette channels/conversions and keeps its existing hex
color theme format through serde adapters. Its generic canvas prepaint helper is
named `on_prepaint_child` to distinguish it from CE's interactive callback.

Regression tests cover interrupted panel motion, reduced motion, bundled theme
loading/serialization, color channels, and translated cached-scene filters/masks.

## Validation

- Root `cargo test -p atlas-ui --lib --locked`: 55 passed.
- CE `cargo test --manifest-path vendor/gpui-ce/Cargo.toml -p gpui-ce --lib scene::tests --features test-support --target-dir target`: 15 passed.
- Root `cargo build -p atlas --locked`: passed on Windows / Rust 1.97.1.
- Desktop smoke test: complete icons, horizontal and vertical split open/close,
  separator resize, virtual-list scrolling, inspector entrance/exit, tab dragging,
  selection checkboxes and disabling motion. Temporary tabs/splits and motion
  preferences were restored after the test.

Only the Windows build and desktop were exercised here; other platform backends
have not received runtime validation in this migration.
