# Atlas Architecture

## Principles

1. `atlas-model` only owns data, IDs, and snapshots. It does not depend on filesystem, cloud runtime, extension runtime, or GPUI.
2. `atlas-core` resolves real backends and background work: filesystem, cloud, extensions, and search.
3. `atlas-ui` renders GPUI and dispatches `Action`; it does not touch the filesystem directly from render paths.
4. All user input flows through `Action`. Background work responds through `flume` with `BgResult`.
5. Avoid synchronous `std::fs` on the foreground UI path. Prefer `tokio::fs` or `spawn_blocking`; remaining synchronous calls are legacy and being moved off the render path.
6. Avoid `unwrap`/`expect` in app code. Tests may use them.
7. Avoid `Rc<RefCell>`. Prefer `Arc`, channels, and clear ownership; a few UI-local views still use it.

## Workspace

- `atlas`: thin launcher; initializes logging/runtime and calls `atlas_ui::run`.
- `crates/atlas-model`: neutral model. IDs, serializable state, requests/results, no runtime.
- `crates/atlas-core`: filesystem, search, cloud, extensions, tag DB, watchers, and jobs.
- `crates/atlas-ui`: GPUI/gpui-component, textures, overlays, layout, and animations.
- `crates/atlas-win-space`: Windows disk-space and cleanup engine.
- `crates/atlas-search-service`: background file-index and search service.
- `crates/atlas-publisher`: CLI to publish extension packages.
- `crates/atlas-cloud-extension`, `crates/atlas-icon-extension`: first-party extension sidecars.

Dependency direction:

```text
atlas-ui -> atlas-core -> atlas-model
atlas    -> atlas-ui + atlas-core + atlas-model
atlas-model -> no runtime crates
```

## Current State

- Main renderer: `gpui` + `gpui-component`.
- The live GPUI layer (`App`, views, input, overlays, platform glue) lives in `crates/atlas-ui`.
- Configuration, actions, and packaging use the `atlas` name.
- Lints: `unsafe_code` is denied where possible and `unwrap_used`/`expect_used` warn. Strict clippy is a cleanup goal, not a CI gate yet (see CONTRIBUTING.md).
- `atlas-model` does not import filesystem, cloud, extension, or search runtime code. Contracts live in `cloud_model`, `ext_model`, and `preview_cache`.
- Runtime state moved out of `AppState`: watchers, `nucleo`, and the extension registry live in `App`.

## Flow

- UI/input -> `Action` -> `app::actions::process_actions`.
- `App` mutates `AppState` or launches async work.
- Async work -> `BgResult` through `flume`.
- `App::drain_bg_results` applies results and requests repaint.

## GPUI

- The root view is mounted with `gpui_platform::application`, `Root`, and `gpui_component::init`.
- Views are retained with `Entity<Self>` and mutated through `update`.
- Real focus uses `FocusHandle`; overlays capture keyboard input with their own focus.
- Large lists use `uniform_list`; only visible rows should render.
- GPUI textures/images live in UI (`RenderImage`, caches), never in state.
- Future animations should prefer `.with_animation` and local easing. Model loading/empty/error states in state.

## Contribution Rules

- If a feature needs a real backend, define or reuse a model ID/request first, then resolve it in core.
- If UI needs data, request it through `Action`/`BgResult`; do not call `std::fs` from render code.
- If a feature adds state, place neutral state first, then core behavior, then UI.
- If a strict lint is legitimate noise, use `#[allow(...)]` with a short `reason` comment.
- Gate meaningful changes with `cargo fmt --check`, `cargo check --workspace`, and `cargo test --workspace`.
