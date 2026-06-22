#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::state::layout_tree::{Container, Tile, TileId};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Entity, FocusHandle, ScrollStrategy, SharedString, UniformListScrollHandle,
};
use gpui_component::input::InputState;

use crate::search::nucleo_pool::NucleoPool;
use crate::state::app_state::{AppState, BgResult};
use crate::state::pane::TabData;
use crate::state::preview_cache::PreviewRequest;
use crate::util::config::{save_profiles, save_session, save_settings};

pub mod actions;
mod bg;
#[cfg(feature = "cloud")]
pub mod cloud;
pub mod devices;
pub mod ext;
pub mod fs_sync;
pub(crate) mod icon_loader;
pub mod nav;
pub mod ops;
pub mod pane_ops;
pub mod profiles;
mod startup;
pub mod theme;

pub struct App {
    pub(crate) state: AppState,
    pub(crate) startup_started: Instant,
    pub(crate) startup_first_render_logged: bool,
    pub(crate) tx: flume::Sender<BgResult>,
    pub(crate) rx: flume::Receiver<BgResult>,
    pub(crate) rt: tokio::runtime::Handle,
    /// Keyboard focus for the whole window; the root element tracks it so the
    /// `Atlas` key context (nav/tab shortcuts) is active.
    pub(crate) focus_handle: FocusHandle,
    /// Per-pane virtualized-list scroll handles (UI-side), so keyboard nav can
    /// scroll the cursor into view.
    pub(crate) scroll_handles: HashMap<TileId, UniformListScrollHandle>,
    /// Runtime filesystem watcher handles keyed by pane tile. Kept out of
    /// `AppState` so model/state does not depend on `fs`.
    pub(crate) watchers: HashMap<TileId, crate::fs::watcher::WatcherHandle>,
    /// Runtime fuzzy-search index. Kept out of model/state so state never owns
    /// search engine internals.
    pub(crate) nucleo: NucleoPool,
    pub(crate) nucleo_results: Vec<PathBuf>,
    /// Directory the goto's nucleo index was built from and whether it is a
    /// scoped (drill-in) index; a path-prefixed query re-scopes when either
    /// differs.
    pub(crate) goto_index_root: Option<(PathBuf, bool)>,
    /// Pre-fill for the goto input on its next open (Ctrl+L: current dir).
    pub(crate) goto_prefill: Option<String>,
    /// One-shot fuzzy ranker for the command/ext palettes (matcher reused).
    pub(crate) fuzzy_scorer: crate::search::fuzzy::FuzzyScorer,
    /// Scroll state of the open palette's result list (keyboard nav keeps the
    /// selected row in view). Shared: only one palette is open at a time.
    pub(crate) palette_scroll: gpui::ScrollHandle,
    pub(crate) ctx_menu_scroll: gpui::ScrollHandle,
    pub(crate) type_filter_scroll: gpui::ScrollHandle,
    pub(crate) settings_scroll: gpui::ScrollHandle,
    pub(crate) ext_manager_scroll: gpui::ScrollHandle,
    /// Runtime extension registry. Kept out of `AppState` because loading it scans
    /// extension folders and PATH.
    pub(crate) ext_registry: crate::ext::Registry,
    pub(crate) pending_preview: Option<PendingPreview>,
    /// Channel to the `SQLite` tag-db worker (writes fire-and-forget, reads reply via `tx`).
    pub(crate) tag_db_tx: flume::Sender<crate::fs::tag_db::TagDbCmd>,
    /// Channel to the cloud worker (rclone sidecar + transfers). Replies via `tx`.
    #[cfg(feature = "cloud")]
    pub(crate) cloud_tx: flume::Sender<crate::cloud::CloudCmd>,
    // FASE5: UI-side preview texture caches (state holds only neutral RGBA).
    // FASE5: single-preview texture per tab, keyed by its source path for invalidation.
    pub(crate) preview_textures: HashMap<TileId, (PathBuf, std::sync::Arc<gpui::RenderImage>)>,
    // FASE5: decoded multi-selection thumbnail textures per tab.
    pub(crate) multi_textures: HashMap<TileId, Vec<(PathBuf, std::sync::Arc<gpui::RenderImage>)>>,
    // Selectable code/text preview editors per tab. Keyed by path/content so the
    // inspector can preserve selection while rendering read-only preview text.

    // FASE6: per-tab image-preview interaction state (scroll=zoom, drag=pan, Space=expand).
    // Pure UI/interaction state, kept out of `AppState` like the other render caches.
    pub(crate) preview_view: HashMap<TileId, PreviewView>,
    // Transient: last cursor position while panning the preview image.
    pub(crate) preview_drag: Option<gpui::Point<gpui::Pixels>>,
    /// Code/text editor state for the currently focused tab's preview. `None` when the
    /// preview is not a text/code file or has not loaded yet. Recreated when the tab or
    /// selected file changes.
    pub(crate) preview_editor: Option<PreviewEditor>,
    /// State for the open tag-picker overlay. `None` when closed.
    pub(crate) tag_picker_menu: Option<crate::ui::tag_picker_menu::TagPickerState>,
    /// Search input inside the tag-picker overlay.
    pub(crate) tag_picker_input: Option<Entity<InputState>>,
    /// Subscription that notifies on input changes so the tag picker repaints.
    pub(crate) tag_picker_sub: Option<gpui::Subscription>,
    /// Scroll handle for the tag-picker list.
    pub(crate) tag_picker_scroll: gpui::ScrollHandle,
    // FASE5: keyboard focus for the active overlay (fuzzy bars / modals capture keys here).
    pub(crate) overlay_focus: Option<FocusHandle>,
    pub(crate) batch_rename_field: BatchRenameField,
    // FASE4: UI-side cache of downscaled grid thumbnails, keyed by file path.
    pub(crate) thumb_textures: HashMap<PathBuf, crate::ui::thumbs::ThumbState>,
    // FASE6: paths with an in-flight *extension* thumbnailer process. Caps how many
    // external processes (ffmpeg/magick) run at once so a big folder can't spawn
    // hundreds. Native image decodes are CPU-only and not throttled here.
    pub(crate) ext_thumb_inflight: std::collections::HashSet<PathBuf>,
    // Viewport-priority cancellation: paths currently visible in the grid.
    // Workers check this before decoding; paths absent are skipped (and removed
    // from thumb_textures so they re-request when scrolled back into view).
    pub(crate) thumb_wanted: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
    // Count of in-flight native image decodes (incremented on spawn, decremented
    // on Ready or Skipped). Caps concurrent decodes so the blocking pool stays
    // available for directory listing and icon rasterisation.
    pub(crate) native_thumb_inflight: usize,
    // Icon theme: parsed VSCode icon-theme index (extension → SVG path) + rasterised
    // textures. `None` when no supported theme was found at startup.
    pub(crate) icon_theme: Option<crate::icon_theme::IconThemeIndex>,
    /// Rasterised icon textures. Key = `icon_theme::icon_key()` result.
    pub(crate) icon_cache: HashMap<String, crate::icon_theme::IconState>,
    /// Dedicated channel for icon rasterisation results. Uses `Arc<RenderImage>` directly
    /// so GPUI types never enter atlas-model's `BgResult` enum.
    pub(crate) icon_tx: flume::Sender<(String, Option<std::sync::Arc<gpui::RenderImage>>)>,
    pub(crate) icon_rx: flume::Receiver<(String, Option<std::sync::Arc<gpui::RenderImage>>)>,
    // FASE A: central tabs/splits rendered by the gpui-component Dock. `pane_tree`
    // stays the source of truth; the Dock is rebuilt when `layout_revision` advances
    // (explicit ops) and reconciled back on interactive changes.
    pub(crate) dock: Option<Entity<gpui_component::dock::DockArea>>,
    /// Cached `FilePanel` entities keyed by pane `TileId`. Reusing entities across
    /// `rebuild_dock_center` calls keeps gpui animation state alive on existing panes
    /// so they don't flash when a new split or close restructures the layout.
    pub(crate) file_panel_cache: HashMap<TileId, Entity<crate::ui::pane_panel::FilePanel>>,
    pub(crate) sidebar_panel: Option<Entity<crate::ui::sidebar::SidebarPanel>>,
    /// MRU focus history: pane TileIds in the order they last held focus.
    /// `find_focus_after_close` pops entries until it finds a live pane,
    /// giving VS-Code-style "go to previously used pane" on Ctrl+W.
    pub(crate) focus_history: std::collections::VecDeque<TileId>,
    // The open palette hosted in its own view (see `ui::palette_host`). Cached like
    // `dock`; cleared on open/close so each open replays the panel's open animation.
    pub(crate) palette_host: Option<(
        crate::ui::palette::PaletteKind,
        Entity<crate::ui::palette_host::PaletteHost>,
    )>,
    // Set when a palette is dismissed with a fade: keeps its view mounted for one
    // exit animation, then `tick_palette_close` (driven by the 60 Hz drain) tears
    // it down.
    pub(crate) palette_closing: Option<(crate::ui::palette::PaletteKind, Instant)>,
    pub(crate) layout_revision: u64,
    pub(crate) dock_revision: u64,
    /// Short-lived visual affordances for panes that just entered through a tab
    /// create or split operation. UI-only; never persisted into AppState.
    pub(crate) pane_animations: HashMap<TileId, PaneAnimation>,
    /// One lightweight entity per pane. `PaneContentView` observes its own signal
    /// so only the affected pane re-renders on pane-specific state changes, instead
    /// of every pane re-rendering on every `App::cx.notify()`.
    pub(crate) pane_signals: HashMap<TileId, gpui::Entity<PaneSignal>>,
    /// Last painted bounds per pane, used for RAD-style FLIP layout transitions.
    /// This is intentionally separate from `pane_view_bounds`, which is cleared
    /// every frame for interaction hit-testing.
    pub(crate) pane_layout_bounds: HashMap<TileId, gpui::Bounds<gpui::Pixels>>,
    pub(crate) pane_layout_animations: HashMap<TileId, PaneLayoutAnimation>,
    pub(crate) pane_layout_frame_pending: bool,
    pub(crate) dock_split_animations: HashMap<TileId, DockSplitAnimation>,
    /// Incremental dock mutations queued by keyboard split/close actions and drained
    /// in `render` (which has `Window`/`Context`, unlike `process_actions`). Driving
    /// the live dock + reconcile this way avoids the O(N) `rebuild_dock_center` churn
    /// that recreated every `TabPanel`/`StackPanel` on each op.
    pub(crate) pending_dock_ops: Vec<DockOp>,
    /// Panes mid close-out animation: kept mounted (and in the dock) until the exit
    /// animation elapses, then a `DockOp::Remove` is queued. Mirrors `palette_closing`.
    pub(crate) pane_closing: HashMap<TileId, Instant>,
    /// Visual-only panes that were already removed from layout but are fading out.
    pub(crate) close_ghosts: Vec<CloseGhost>,
    /// Set when the dock's structure changed (programmatic op or interactive drag) and
    /// `pane_tree` needs to be resynced. Drained once per render in `ensure_dock`, so
    /// many `LayoutChanged` events coalesce into a single synchronous reconcile (no
    /// async double-render / container-id churn when creating or splitting panes).
    pub(crate) reconcile_pending: bool,
    /// When a pane's fresh listing last appeared (tab create / navigation). Views
    /// stagger-animate item rows in only while within `ITEM_APPEAR_WINDOW` of this,
    /// so scrolling later never re-triggers the appear. Set alongside `appear_seq`.
    pub(crate) pane_items_appear: HashMap<TileId, Instant>,
    // FASE C: rubber-band (select box) drag state, UI-side. `row_rects` holds the
    // window-space rect of each *visible* item, rebuilt every frame via on_prepaint;
    // the active drag intersects its rectangle against them to drive the selection.
    pub(crate) rubber: Option<RubberDrag>,
    pub(crate) row_rects: HashMap<TileId, Vec<(usize, gpui::Bounds<gpui::Pixels>)>>,
    // FASE F: pane drop-zones for the OLE drop target. Each frame's render path
    // pushes one DropZone per visible pane (via on_prepaint) into this vec; at the
    // start of the next frame the vec is committed to `dnd.zones` (Windows only) so
    // the COM vtable can resolve drop positions to the correct pane directory.
    // Kept unconditional so the helper fn compiles on all platforms without cfg noise.
    pub(crate) pending_pane_zones: Vec<crate::dnd::DropZone>,
    // Native single-line inputs (gpui-component) for the overlays. Created lazily on
    // open, cleared to `None` on close so each open starts empty. Holding the `Entity`
    // here lets the renderer read `.value()` each frame as the source of truth.
    pub(crate) goto_input: Option<Entity<InputState>>,
    pub(crate) palette_input: Option<Entity<InputState>>,
    pub(crate) ext_palette_input: Option<Entity<InputState>>,
    pub(crate) rename_input: Option<Entity<InputState>>,
    pub(crate) search_inputs: HashMap<TileId, Entity<InputState>>,
    pub(crate) batch_pattern_input: Option<Entity<InputState>>,
    pub(crate) batch_replacement_input: Option<Entity<InputState>>,
    /// Native input backing the rename-active-profile dialog.
    pub(crate) rename_profile_input: Option<Entity<InputState>>,
    /// Native input backing create/rename tag.
    pub(crate) create_tag_input: Option<Entity<InputState>>,
    pub(crate) renaming_profile_idx: Option<usize>,
    pub(crate) ext_manager_input: Option<Entity<InputState>>,
    /// Inputs for the add-remote dialog; created on open, cleared on close.
    #[cfg(feature = "cloud")]
    pub(crate) cloud_add_inputs: Option<crate::ui::overlays::add_cloud::CloudAddInputs>,
    /// Input backing the rename-cloud-remote dialog.
    #[cfg(feature = "cloud")]
    pub(crate) rename_remote_input: Option<Entity<InputState>>,
    /// In-flight inline rename (one row at a time). Holds the native input being edited.
    pub(crate) rename_edit: Option<RenameEdit>,
    // Goto bar "search in content": matching file paths from the latest grep, the
    // generation that produced them (stale batches are dropped), and the cancel flag
    // shared with the in-flight background search.
    pub(crate) content_results: Vec<PathBuf>,
    pub(crate) content_gen: u64,
    pub(crate) content_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// When set, the next render applies the active profile's theme to the global
    /// gpui-component `Theme` (deferred because applying needs `&mut App` + window,
    /// which `process_actions` lacks). Starts `true` so the restored profile theme
    /// is applied on first paint.
    pub(crate) theme_dirty: bool,
    /// When set, the next render applies `settings.sidebar_width` to the left dock
    /// (per-profile sidebar width). Starts `true` so the restored width is applied
    /// once the dock exists; set again by `switch_profile`.
    pub(crate) sidebar_width_dirty: bool,

    // ── Windows OLE drag-and-drop ──────────────────────────────────────────
    // Raw HWND value (isize). Extracted once in App::new from gpui's
    // HasWindowHandle trait. Stored as isize so it is Send (HWND is !Send).
    // Used only on the main (UI) thread.
    #[cfg(windows)]
    pub(crate) hwnd: Option<isize>,
    // DnD sub-system: drop target COM object (kept alive), zone registry
    // (updated each render frame while a drag is active), and the action queue
    // (COM vtable pushes, App drains in drain_dnd_queue).
    #[cfg(windows)]
    pub(crate) dnd: Option<crate::dnd::windows::DndState>,

    // ── Shell context menu (Atlas–style right-click) ──────────────────
    // STA worker that enumerates and invokes the native Windows shell menu
    // (Open with, Send to, 7-Zip, Properties, …). `None` if the worker failed.
    #[cfg(windows)]
    pub(crate) shell_menu: Option<crate::shell_menu::ShellMenuHandle>,
    // Open right-click menu state: Atlas items + (Windows) native shell items.
    // `None` when closed. Cross-platform; on non-Windows only Atlas items show.
    pub(crate) ctx_menu: Option<crate::ui::shell_context_menu::CtxMenuState>,
    // Search input backing the context menu. Focused via `sync_overlay_focus`.
    pub(crate) ctx_menu_input: Option<Entity<InputState>>,
    // Repaints App on every keystroke in the context-menu search (the menu is
    // painted inline by App, so typed chars need an explicit re-render — same
    // reason `PaletteHost` subscribes to its input). Dropped on close.
    pub(crate) ctx_menu_sub: Option<gpui::Subscription>,
    // Monotonic id: stale `Built` events from a previous open are ignored.
    pub(crate) ctx_menu_seq: u64,
    /// When `NewFolder` is invoked, stores (tile, path) so the FS-watcher result
    /// can auto-select that entry once it appears in the listing.
    pub(crate) pending_select: Option<(TileId, PathBuf)>,
    /// Details-view column widths, shared across all panes.
    pub(crate) col_widths: ColWidths,
    /// User-facing order for Details metadata columns. Name stays fixed first.
    pub(crate) details_column_order: [DetailsColumn; 3],
    /// Active column-resize drag state (cleared on mouse-up).
    pub(crate) col_resize: Option<ColResizeState>,
    /// Column currently being drag-reordered (for the floating header + source gap).
    /// Cleared on drop / mouse-up.
    pub(crate) col_dragging: Option<DetailsColumn>,
    /// Header-local x of the cursor during a column drag (drives the floating box).
    pub(crate) col_drag_x: f32,
    /// Slide animation that interpolates each column from its pre-swap slot to its
    /// new one, so reordered headers/rows glide instead of snapping.
    pub(crate) details_reorder: Option<DetailsReorderAnim>,
    /// Per-pane active type-category filters for the Details view.
    /// Empty set = no filter. Values are `file_type_label()` strings.
    pub(crate) type_filters: HashMap<TileId, HashSet<String>>,
    /// Per-pane active size-category filters for the Details view.
    /// Empty set = no filter.
    pub(crate) size_filters: HashMap<TileId, HashSet<crate::ui::size_filter_menu::SizeFilter>>,
    // ── Details type-filter overlay ───────────────────────────────────────
    /// State for the open type-filter overlay. `None` when closed.
    pub(crate) type_filter_menu: Option<crate::ui::type_filter_menu::TypeFilterMenuState>,
    /// Search input inside the type-filter overlay.
    pub(crate) type_filter_input: Option<Entity<InputState>>,
    /// Subscription that notifies on input changes so the overlay repaints.
    pub(crate) type_filter_sub: Option<gpui::Subscription>,
    // ── Details size-filter overlay ───────────────────────────────────────
    /// State for the open size-filter overlay. `None` when closed.
    pub(crate) size_filter_menu: Option<crate::ui::size_filter_menu::SizeFilterMenuState>,
    // ── Drag-hover navigation ─────────────────────────────────────────────
    /// Finder-style hover-to-navigate state (updated each animation frame while a drag is live).
    pub(crate) drag_hover: crate::ui::drag_hover::DragHoverState,
    /// Buffer of targets registered by renderers each frame; consumed by `tick_drag_hover`.
    pub(crate) pending_drag_hover_targets: Vec<crate::ui::drag_hover::DragHoverTarget>,
    /// Window-space bounds of each pane view, used for edge-scroll + tag-drop detection.
    pub(crate) pane_view_bounds: HashMap<TileId, gpui::Bounds<gpui::Pixels>>,
    pub(crate) dock_linear_bounds: HashMap<TileId, gpui::Bounds<gpui::Pixels>>,
    pub(crate) dock_resize: Option<DockResizeState>,
    pub(crate) sidebar_resize: Option<SidebarResizeState>,
    pub(crate) inspector_resize: Option<InspectorResizeState>,
    /// Sidebar pixel-scroll region registered each frame for drag edge-scroll.
    /// The sidebar is a separate entity (not a pane), so it can't use the
    /// row-based pane edge-scroll; this drives `ScrollHandle` by pixels instead.
    pub(crate) sidebar_scroll_region: Option<(gpui::Bounds<gpui::Pixels>, gpui::ScrollHandle)>,
    pub(crate) atlas_launcher_profiles_anchor: gpui::Point<gpui::Pixels>,
    /// Panes whose `location` changed via `navigate_to` but whose `PaneSignal`
    /// has not been notified yet. Flushed by `flush_dirty_panes` from any path
    /// that holds a `Context` after navigating (drops, drag-hover tick).
    pub(crate) dirty_panes: std::collections::HashSet<TileId>,
    pub(crate) dirty_all_panes: bool,
    // ── Command shortcut capture ──────────────────────────────────────────
    /// Non-None while the user is recording a new keyboard shortcut for a palette command.
    pub(crate) command_shortcut_capture: Option<String>,
}

/// Marker entity — one per pane. `PaneContentView` observes this instead of `App`
/// so that only the affected pane re-renders on pane-specific state changes.
pub struct PaneSignal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneAnimationKind {
    NewTab,
    SplitHorizontal,
    SplitVertical,
}

/// A deferred mutation of the live dock, applied in `render`. `Split` adds a new
/// pane next to `source`; `Remove` drops a pane. Both let `reconcile_from_dock`
/// resync `pane_tree` afterwards (same path as interactive drag/close).
#[derive(Clone, Copy, Debug)]
pub(crate) enum DockOp {
    Split {
        source: TileId,
        new_pane: TileId,
        vertical: bool,
    },
    /// Add `new_pane` as a tab into the `TabPanel` that holds `source` (a live pane).
    AddTab {
        source: TileId,
        new_pane: TileId,
    },
    Remove(TileId),
}

#[derive(Clone)]
pub(crate) struct CloseGhost {
    pub(crate) pane_id: TileId,
    pub(crate) title: SharedString,
    pub(crate) bounds: gpui::Bounds<gpui::Pixels>,
    pub(crate) dir: (f32, f32),
    pub(crate) started: Instant,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PaneAnimation {
    pub(crate) kind: PaneAnimationKind,
    pub(crate) started: Instant,
}

impl PaneAnimation {
    const TTL: Duration = DockSplitAnimation::DURATION;

    pub(crate) fn new(kind: PaneAnimationKind) -> Self {
        Self {
            kind,
            started: Instant::now(),
        }
    }

    pub(crate) fn is_alive(self) -> bool {
        self.started.elapsed() < Self::TTL
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PaneLayoutAnimation {
    pub(crate) from: gpui::Bounds<gpui::Pixels>,
    pub(crate) to: gpui::Bounds<gpui::Pixels>,
    pub(crate) started: Instant,
}

impl PaneLayoutAnimation {
    const TTL: Duration = Duration::from_millis(crate::ui::motion::PANE_MS);

    pub(crate) fn new(from: gpui::Bounds<gpui::Pixels>, to: gpui::Bounds<gpui::Pixels>) -> Self {
        Self {
            from,
            to,
            started: Instant::now(),
        }
    }

    pub(crate) fn is_alive(self) -> bool {
        self.started.elapsed() < Self::TTL
    }

    pub(crate) fn progress(self) -> f32 {
        let elapsed = self.started.elapsed().as_secs_f32();
        let total = Self::TTL.as_secs_f32().max(f32::EPSILON);
        crate::ui::motion::ease_out_quart((elapsed / total).min(1.0))
    }

    pub(crate) fn offset(self) -> (f32, f32) {
        self.offset_at(self.progress())
    }

    pub(crate) fn offset_at(self, t: f32) -> (f32, f32) {
        let dx = f32::from(self.from.origin.x) - f32::from(self.to.origin.x);
        let dy = f32::from(self.from.origin.y) - f32::from(self.to.origin.y);
        (dx * (1.0 - t), dy * (1.0 - t))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DockSplitAnimation {
    pub(crate) from: Vec<f32>,
    pub(crate) to: Vec<f32>,
    pub(crate) started: Instant,
}

impl DockSplitAnimation {
    pub(crate) const DURATION: Duration = Duration::from_millis(260);

    pub(crate) fn new(to: Vec<f32>, inserted_ix: usize) -> Self {
        if to.is_empty() {
            return Self {
                from: Vec::new(),
                to,
                started: Instant::now(),
            };
        }
        let mut from = to.clone();
        from.remove(inserted_ix.min(from.len().saturating_sub(1)));
        Self::opening(from, to, inserted_ix)
    }

    pub(crate) fn opening(mut from: Vec<f32>, to: Vec<f32>, inserted_ix: usize) -> Self {
        if from.len() + 1 == to.len() && inserted_ix <= from.len() {
            let start = 0.01;
            let delta = to
                .get(inserted_ix)
                .map(|share| (share - start).max(0.0))
                .unwrap_or(0.0);
            from.insert(inserted_ix, start);
            if let Some((ix, _)) = from
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != inserted_ix)
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
            {
                from[ix] += delta;
            }
        } else {
            from = to.clone();
        }
        Self {
            from,
            to,
            started: Instant::now(),
        }
    }

    pub(crate) fn closing(from: Vec<f32>, closing_ix: usize) -> Self {
        let mut to = from.clone();
        if closing_ix < to.len() {
            let end = 0.01;
            let delta = (to[closing_ix] - end).max(0.0);
            to[closing_ix] = end;
            if let Some((ix, _)) = to
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != closing_ix)
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
            {
                to[ix] += delta;
            }
        }
        Self {
            from,
            to,
            started: Instant::now(),
        }
    }

    pub(crate) fn shares(&self) -> Vec<f32> {
        let t = (self.started.elapsed().as_secs_f32() / Self::DURATION.as_secs_f32()).min(1.0);
        let eased = split_spring(t * Self::DURATION.as_secs_f32());
        self.from
            .iter()
            .zip(&self.to)
            .map(|(from, to)| from + (to - from) * eased)
            .collect()
    }

    pub(crate) fn done(&self) -> bool {
        self.started.elapsed() >= Self::DURATION
    }
}

fn split_spring(t: f32) -> f32 {
    let omega: f32 = 22.0;
    let zeta: f32 = 0.68;
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    let e = (-zeta * omega * t).exp();
    1.0 - e * ((wd * t).cos() + (zeta * omega / wd) * (wd * t).sin())
}

/// Widths of the fixed-width columns in the Details view (Name is flex_1, not stored).
/// Serialized into ProfilePrefs via `DetailsColumns: ProfileScoped`.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct ColWidths {
    /// Name column width (icon + text). Resizable like the rest; Explorer-style
    /// left-anchored layout, so this is a real fixed width, not flex.
    #[serde(default = "default_name_width")]
    pub(crate) name: f32,
    pub(crate) size: f32,
    pub(crate) modified: f32,
    pub(crate) type_w: f32,
}

fn default_name_width() -> f32 {
    300.
}

impl Default for ColWidths {
    fn default() -> Self {
        Self {
            name: default_name_width(),
            size: 90.,
            modified: 150.,
            type_w: 110.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum DetailsColumn {
    Size,
    Modified,
    Type,
}

impl DetailsColumn {
    pub(crate) const DEFAULT_ORDER: [Self; 3] = [Self::Size, Self::Modified, Self::Type];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Type => "type",
        }
    }
}

/// Column-reorder slide: snapshots the order *before* a live swap so the header
/// and rows can interpolate each column's x from its old slot to its new one.
/// Mirrors the `PaneLayoutAnimation` time model (`Instant` + eased `progress`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DetailsReorderAnim {
    pub(crate) from_order: [DetailsColumn; 3],
    pub(crate) started: Instant,
}

impl DetailsReorderAnim {
    const TTL: Duration = Duration::from_millis(crate::ui::motion::MICRO_MS);

    pub(crate) fn new(from_order: [DetailsColumn; 3]) -> Self {
        Self {
            from_order,
            started: Instant::now(),
        }
    }

    pub(crate) fn is_alive(self) -> bool {
        self.started.elapsed() < Self::TTL
    }

    /// Eased 0→1 progress; 1.0 once expired.
    pub(crate) fn progress(self) -> f32 {
        let elapsed = self.started.elapsed().as_secs_f32();
        let total = Self::TTL.as_secs_f32().max(f32::EPSILON);
        crate::ui::motion::ease_out_quart((elapsed / total).min(1.0))
    }
}

/// Per-profile Details column preferences: widths + visual order. Persisted as a
/// single `ProfileScoped` blob so widths and order travel together.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct DetailsColumnPrefs {
    pub(crate) widths: ColWidths,
    pub(crate) order: [DetailsColumn; 3],
}

impl Default for DetailsColumnPrefs {
    fn default() -> Self {
        Self {
            widths: ColWidths::default(),
            order: DetailsColumn::DEFAULT_ORDER,
        }
    }
}

/// Active column-resize drag state for the Details header.
pub(crate) struct ColResizeState {
    /// Which column: "size", "modified", or "type".
    pub(crate) col: &'static str,
    /// Pane the resize started on — only this pane is re-rendered live (scoped
    /// `notify_pane`); other Details panes catch up on mouse-up.
    pub(crate) pane: TileId,
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

pub(crate) struct DockResizeState {
    pub(crate) container: TileId,
    pub(crate) divider: usize,
    pub(crate) dir: crate::state::layout_tree::LinearDir,
    pub(crate) start_pos: f32,
    pub(crate) span_px: f32,
    pub(crate) start_shares: Vec<f32>,
}

pub(crate) struct SidebarResizeState {
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

pub(crate) struct InspectorResizeState {
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

/// Per-tab interaction state for the inspector image preview. `zoom` is a multiplier
/// over the fit-to-panel size (1.0 = fit). `expanded` widens the inspector to most of
/// the window (toggled with Space). Reset on each new selection (except `expanded`,
/// which persists so browsing in the big view keeps it open).
#[derive(Clone, Copy)]
pub(crate) struct PreviewView {
    pub(crate) zoom: f32,
    pub(crate) pan_x: f32,
    pub(crate) pan_y: f32,
    pub(crate) expanded: bool,
}

impl Default for PreviewView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            expanded: false,
        }
    }
}

/// Per-tab code/text editor state for the inspector preview. Holds the native
/// `InputState` in code-editor mode, the source path for saving, a dirty flag
/// (set on any change), whether the editor is in editable mode, and the language
/// token (so the toolbar can display it).
pub(crate) struct PreviewEditor {
    /// The gpui-component `InputState` in `code_editor(lang)` mode.
    pub(crate) state: Entity<InputState>,
    /// Absolute path of the file being previewed/edited.
    pub(crate) path: PathBuf,
    /// Most recent text content, kept current by the Change subscription.
    /// Used by `SavePreviewFile` without needing `cx`.
    pub(crate) cached_text: String,
    /// True when the content has changed since last save (or since load).
    pub(crate) dirty: bool,
    /// When false the editor is rendered `.disabled(true)` — read-only with highlight.
    pub(crate) editing: bool,
    /// Current line-number visibility (mirrors the profile pref). The render path
    /// syncs this into the `InputState` when `ln_needs_sync` is set.
    pub(crate) line_numbers: bool,
    /// Set by `TogglePreviewLineNumbers`; cleared by the render after syncing.
    pub(crate) ln_needs_sync: bool,
    /// Language token passed to `code_editor` (e.g. `"rust"`, `"python"`).
    pub(crate) lang: gpui::SharedString,
    /// Change subscription — kept alive as long as the editor is open.
    pub(crate) _sub: gpui::Subscription,
}

/// In-flight inline rename of a single row in the focused pane.
pub struct RenameEdit {
    pub(crate) pane: TileId,
    pub(crate) idx: usize,
    pub(crate) path: PathBuf,
    pub(crate) input: Entity<InputState>,
    /// When set, the next focus-sync selects this many chars from the start
    /// (to pre-highlight the stem; None after consumed).
    pub(crate) initial_select_chars: Option<u32>,
}

/// In-flight rubber-band selection (one pane at a time).
pub struct RubberDrag {
    pub(crate) pane: TileId,
    pub(crate) origin: gpui::Point<gpui::Pixels>,
    pub(crate) current: gpui::Point<gpui::Pixels>,
    /// Selection before the drag began (kept when Ctrl/Cmd makes the drag additive).
    pub(crate) base: HashSet<usize>,
    /// Items intersected at any point during this drag. This keeps rubber selection
    /// stable while the virtualized list scrolls and old rows leave the viewport.
    pub(crate) swept: HashSet<usize>,
    pub(crate) additive: bool,
    /// True once the cursor has moved at least 4 px from origin. Prevents a plain
    /// click from drawing a marquee or clearing the selection prematurely.
    pub(crate) started: bool,
}

// FASE5: pub(crate) so the gpui inspector module (crate::ui::inspector) can reuse
// the same debounce as the legacy egui sync path.
pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(90);
/// How often the foreground loop drains background results. Keep this out of the
/// display-refresh path: waking the UI thread at 60 Hz made window moves stutter on
/// Windows even when there was no work to paint.
const DRAIN_INTERVAL: Duration = Duration::from_millis(100);
// How long a palette's exit animation runs before its view is torn down. Must be ≥
// `palette::PALETTE_CLOSE_MS` so the fade-out completes (it reaches opacity 0 first).
const PALETTE_CLOSE_FINALIZE: Duration = Duration::from_millis(150);
// How long a pane's close-out animation runs before it's removed from the dock.
pub(crate) const PANE_CLOSE_FINALIZE: Duration = Duration::from_millis(crate::ui::motion::PANE_MS);

/// Map the user's scroll-smoothness setting (0 = instant … 1 = silky) to the gpui
/// scroll handle's (ease, boost). `ease` is the fraction of remaining distance closed
/// per frame (lower = longer glide); `boost` is per-notch travel.
pub(crate) fn scroll_smooth_params(smoothness: f32) -> (f32, f32) {
    let s = smoothness.clamp(0.0, 1.0);
    if s <= 0.0 {
        return (0.0, 1.0); // off → instant (ease 0 disables smoothing in gpui)
    }
    // s=0.01→ease~0.46 (snappy), s=1→ease 0.12 (silky); boost 1.0→3.0.
    let ease = 0.46_f32 - 0.34 * s;
    let boost = 1.0_f32 + 2.0 * s;
    (ease, boost)
}
// Window after a fresh listing appears during which item rows stagger-animate in.
// Must comfortably exceed the longest staggered row animation; past it, scrolling
// never re-triggers the appear.
pub(crate) const ITEM_APPEAR_WINDOW: Duration = Duration::from_millis(150);

pub struct PendingPreview {
    pub(crate) tab: TileId,
    pub(crate) request: PreviewRequest,
    pub(crate) ready_at: Instant,
}

impl App {
    const MAX_PANE_LAYOUT_ANIMATIONS: usize = 12;

    /// Notify only the signal entity for `pane_id`. The `PaneContentView` for that
    /// pane observes this signal, so only it re-renders — not every other pane.
    pub(crate) fn notify_pane(&self, pane_id: TileId, cx: &mut gpui::Context<Self>) {
        let _perf = crate::perf::span_detail("notify_pane", || format!("pane={}", pane_id.0));
        crate::perf::event(|| format!("notify_pane pane={}", pane_id.0));
        if let Some(signal) = self.pane_signals.get(&pane_id) {
            signal.update(cx, |_, cx| cx.notify());
        }
    }

    /// Notify all pane signals at once — used for global changes (settings, theme,
    /// icon cache updates) that affect every pane simultaneously.
    pub(crate) fn notify_all_panes(&self, cx: &mut gpui::Context<Self>) {
        let _perf = crate::perf::span_detail("notify_all_panes", || {
            format!("signals={}", self.pane_signals.len())
        });
        crate::perf::event(|| format!("notify_all_panes signals={}", self.pane_signals.len()));
        for signal in self.pane_signals.values() {
            signal.update(cx, |_, cx| cx.notify());
        }
    }

    /// Repaint panes navigated via `navigate_to` (which has no `cx`). `navigate_to`
    /// updates `pane.location` (chrome/breadcrumb show the new route) and marks the
    /// tile in `dirty_panes`; this flushes those into `notify_pane` so the file view
    /// re-renders in lockstep with the route. Also notifies App (chrome + sidebar
    /// via its `observe`). Call from any path holding a `Context` after navigating.
    pub(crate) fn flush_dirty_panes(&mut self, cx: &mut gpui::Context<Self>) {
        let _perf = crate::perf::span_detail("flush_dirty_panes", || {
            format!(
                "dirty={} dirty_all={}",
                self.dirty_panes.len(),
                self.dirty_all_panes
            )
        });
        if self.dirty_all_panes {
            self.dirty_all_panes = false;
            self.notify_all_panes(cx);
            cx.notify();
            return;
        }
        if self.dirty_panes.is_empty() {
            cx.notify();
            return;
        }
        let dirty: Vec<TileId> = self.dirty_panes.drain().collect();
        for id in dirty {
            self.notify_pane(id, cx);
        }
        cx.notify();
    }

    /// Return (creating lazily) the signal entity for a pane. `PaneContentView`
    /// calls this once at construction to get the entity to observe.
    pub(crate) fn pane_signal_for(
        &mut self,
        pane_id: TileId,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Entity<PaneSignal> {
        self.pane_signals
            .entry(pane_id)
            .or_insert_with(|| cx.new(|_| PaneSignal))
            .clone()
    }

    /// Remove the signal entity when a pane is closed (no-op if already gone).
    pub(crate) fn drop_pane_signal(&mut self, pane_id: TileId) {
        self.pane_signals.remove(&pane_id);
    }

    /// Seam direction a closing pane should slide toward, or `None` if it isn't
    /// closing. The pane retreats into the split seam (toward its surviving sibling)
    /// while fading; once removed, the neighbor expands to fill via FLIP. The fade
    /// itself is a self-driving `with_animation` tween in `FilePanel`.
    pub(crate) fn pane_closing_for(&self, pane_id: TileId) -> Option<(f32, f32)> {
        self.pane_closing
            .contains_key(&pane_id)
            .then(|| self.pane_seam_dir(pane_id))
    }

    /// Unit-ish direction (dx, dy) toward the split seam for `pane_id`: the axis of
    /// the parent Linear container and the sign toward the adjacent sibling.
    fn pane_seam_dir(&self, pane_id: TileId) -> (f32, f32) {
        use crate::state::layout_tree::{Container, LinearDir, Tile};
        // The pane lives in a Tabs container; the split axis is on its parent Linear.
        let tabs_id = self.state.find_tabs_container(pane_id).unwrap_or(pane_id);
        let Some(parent_id) = self.state.find_parent_of(tabs_id) else {
            return (0.0, 0.0);
        };
        let Some(Tile::Container(Container::Linear(lin))) =
            self.state.pane_tree.tiles.get(parent_id)
        else {
            return (0.0, 0.0);
        };
        let Some(pos) = lin.children.iter().position(|&c| c == tabs_id) else {
            return (0.0, 0.0);
        };
        // Slide toward the seam shared with the neighbor: if not first child the
        // neighbor is before us (negative), else after us (positive).
        let sign = if pos > 0 { -1.0 } else { 1.0 };
        match lin.dir {
            LinearDir::Horizontal => (sign, 0.0),
            LinearDir::Vertical => (0.0, sign),
        }
    }

    /// If `pane_id` is within its item-appear window, return the `appear_seq` to key
    /// the row animation on (changes per fresh listing, so it plays once). `None`
    /// once the window elapses, so scrolling never re-triggers the cascade.
    pub(crate) fn pane_item_appear_seq(&self, pane_id: TileId) -> Option<u64> {
        if self.state.settings.disable_animations {
            return None;
        }
        let started = self.pane_items_appear.get(&pane_id)?;
        (started.elapsed() < ITEM_APPEAR_WINDOW)
            .then(|| self.state.tab_states.get(&pane_id).map(|ts| ts.appear_seq))
            .flatten()
    }

    pub(crate) fn pane_animation_for(&self, pane_id: TileId) -> Option<PaneAnimation> {
        self.pane_animations
            .get(&pane_id)
            .copied()
            .filter(|animation| animation.is_alive())
    }

    pub(crate) fn prune_pane_animations(&mut self) {
        self.pane_animations
            .retain(|_, animation| animation.is_alive());
    }

    pub(crate) fn pane_layout_animation_for(&self, pane_id: TileId) -> Option<PaneLayoutAnimation> {
        self.pane_layout_animations
            .get(&pane_id)
            .copied()
            .filter(|animation| animation.is_alive())
    }

    pub(crate) fn prune_pane_layout_animations(&mut self) {
        self.pane_layout_animations
            .retain(|_, animation| animation.is_alive());
    }

    /// Live column-reorder slide, if still running.
    pub(crate) fn details_reorder_anim(&self) -> Option<DetailsReorderAnim> {
        self.details_reorder.filter(|a| a.is_alive())
    }

    pub(crate) fn begin_pane_layout_frame(&mut self) {
        self.pane_layout_frame_pending = false;
    }

    pub(crate) fn request_pane_layout_frame(&mut self) -> bool {
        if self.pane_layout_frame_pending {
            return false;
        }
        self.pane_layout_frame_pending = true;
        true
    }

    pub(crate) fn record_pane_layout_bounds(
        &mut self,
        pane_id: TileId,
        bounds: gpui::Bounds<gpui::Pixels>,
    ) -> bool {
        if self.dock_resize.is_some()
            || self.sidebar_resize.is_some()
            || self.inspector_resize.is_some()
        {
            self.pane_layout_bounds.insert(pane_id, bounds);
            return false;
        }

        if self
            .pane_animations
            .get(&pane_id)
            .copied()
            .is_some_and(PaneAnimation::is_alive)
            || self
                .pane_layout_animations
                .get(&pane_id)
                .copied()
                .is_some_and(PaneLayoutAnimation::is_alive)
        {
            self.pane_layout_bounds.insert(pane_id, bounds);
            return false;
        }

        let changed = self
            .pane_layout_bounds
            .get(&pane_id)
            .copied()
            .is_some_and(|old| {
                if !pane_bounds_changed(old, bounds) {
                    return false;
                }

                if !pane_bounds_moved(old, bounds)
                    || pane_bounds_resized(old, bounds)
                    || self.pane_layout_animations.len() >= Self::MAX_PANE_LAYOUT_ANIMATIONS
                {
                    return false;
                }
                self.pane_layout_animations
                    .insert(pane_id, PaneLayoutAnimation::new(old, bounds));
                true
            });
        self.pane_layout_bounds.insert(pane_id, bounds);
        changed
    }

    /// (Re)launch the goto bar's content search for the current query + directory.
    /// Cancels any in-flight search, bumps the generation so its stale results are
    /// ignored, and spawns a fresh grep that streams matches back via `tx`.
    pub(crate) fn relaunch_content_search(&mut self) {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        if let Some(c) = self.content_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.content_gen = self.content_gen.wrapping_add(1);
        self.content_results.clear();
        let query = self.state.overlay.goto_state.query.clone();
        if query.trim().is_empty() {
            return;
        }
        let root = self
            .state
            .active_tab_data()
            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        let cancel = Arc::new(AtomicBool::new(false));
        self.content_cancel = Some(cancel.clone());
        let gen = self.content_gen;
        let tx = self.tx.clone();
        crate::search::content::search_content(root, query, cancel, move |path| {
            let _ = tx.send(BgResult::ContentResults {
                gen,
                paths: vec![path],
            });
        });
    }

    /// Start an OS (OLE) drag of `paths` / archive items, with stale-source
    /// validation: files that no longer exist are excluded and reported as
    /// toasts (via the drag-and-drop event queue — this runs under a read borrow).
    #[cfg(windows)]
    pub(crate) fn trigger_os_drag(
        &self,
        paths: Vec<PathBuf>,
        archive_items: Option<Vec<(PathBuf, PathBuf)>>,
    ) {
        let Some(hwnd) = self.hwnd else { return };
        let queue = self.dnd.as_ref().map(|d| d.action_queue.clone());
        let (paths, stale): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| p.exists());
        if !stale.is_empty() {
            if let Some(q) = queue.as_ref().and_then(|q| q.lock().ok()) {
                let mut q = q;
                for p in &stale {
                    q.push(crate::dnd::windows::DndEvent::Error(
                        rust_i18n::t!(
                            "app_path_not_exists",
                            path = p.file_name().unwrap_or_default().to_string_lossy()
                        )
                        .to_string(),
                    ));
                }
            }
        }
        if paths.is_empty() && archive_items.as_ref().is_none_or(std::vec::Vec::is_empty) {
            return;
        }
        crate::dnd::windows::trigger_drag(hwnd, paths, archive_items, queue);
    }

    /// Record a pane's window-space bounds so the OLE drop target can route external
    /// drops to the correct pane directory. Called from each pane's `on_prepaint`
    /// closure via the `zone_recorder` helper in `root.rs`.
    ///
    /// No-op on non-Windows (OLE drag-and-drop not used; field always empty).
    pub(crate) fn push_pane_zone(&mut self, pane_id: TileId, bounds: gpui::Bounds<gpui::Pixels>) {
        #[cfg(windows)]
        if let Some(dest) = self
            .tab_data_for(pane_id)
            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
        {
            crate::perf::count("dnd.pending_zone_push");
            self.pending_pane_zones.push(crate::dnd::DropZone {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                w: f32::from(bounds.size.width),
                h: f32::from(bounds.size.height),
                dest,
                pane_id: pane_id.0,
            });
        }
    }

    pub(crate) fn tab_data_for(&self, tab_id: TileId) -> Option<&TabData> {
        match self.state.pane_tree.tiles.get(tab_id) {
            Some(Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }

    /// Re-list every tab touched by a finished background operation.
    pub(crate) fn refresh_tabs(&mut self, tabs: Vec<TileId>) {
        for tab_id in tabs {
            if let Some(tab_data) = self.tab_data_for(tab_id) {
                let loc = tab_data.location.clone();
                let sort_col = tab_data.sort_col.clone();
                let sort_dir = tab_data.sort_dir.clone();
                self.dispatch_list(tab_id, loc, sort_col, sort_dir);
            }
        }
    }

    /// Paths currently selected in the focused tab (for clipboard / delete shortcuts).
    pub(crate) fn selected_paths(&self) -> Vec<PathBuf> {
        self.state
            .active_tab_state()
            .map(|tab| {
                tab.selection
                    .selected
                    .iter()
                    .filter_map(|idx| tab.entries.get(*idx))
                    .map(|entry| entry.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get-or-create the virtualized-list scroll handle for a pane.
    pub(crate) fn scroll_handle_for(&mut self, tile: TileId) -> UniformListScrollHandle {
        let handle = self.scroll_handles.entry(tile).or_default().clone();
        // Apply the user's wheel-smoothness setting to the underlying scroll handle so
        // every file view scrolls with the same feel. Cheap; runs per render.
        let (ease, boost) = scroll_smooth_params(self.state.settings.scroll_smoothness);
        handle.0.borrow().base_handle.set_smooth(ease, boost);
        handle
    }

    /// Move the keyboard cursor within the focused tab's visible list. `target`
    /// maps `(current_pos, len)` to the new visible position. `extend` grows the
    /// selection from the anchor (Shift); otherwise it becomes a single selection.
    fn nav_apply(&mut self, target: impl Fn(usize, usize) -> usize, extend: bool) {
        let tile = self.state.focused_tile;
        let next_pos = {
            let Some(ts) = self.state.tab_states.get_mut(&tile) else {
                return;
            };
            let visible: Vec<usize> = ts.visible_indices().to_vec();
            if visible.is_empty() {
                return;
            }
            let len = visible.len();
            let cur = ts
                .selection
                .keyboard_cursor
                .and_then(|i| visible.iter().position(|v| *v == i))
                .unwrap_or(0);
            let np = target(cur, len).min(len - 1);
            let next_idx = visible[np];
            let anchor = ts.selection.selection_anchor.unwrap_or(next_idx);
            ts.selection.keyboard_cursor = Some(next_idx);
            ts.selection.last_clicked_index = Some(next_idx);
            if extend {
                let ap = visible.iter().position(|v| *v == anchor).unwrap_or(np);
                let (lo, hi) = (ap.min(np), ap.max(np));
                ts.selection.selected.clear();
                ts.selection
                    .selected
                    .extend(visible[lo..=hi].iter().copied());
            } else {
                ts.selection.selection_anchor = Some(next_idx);
                ts.selection.selected.clear();
                ts.selection.selected.insert(next_idx);
            }
            np
        };
        let (is_grid, cols) = self
            .tab_data_for(tile)
            .map(|td| {
                (
                    matches!(td.view_mode, crate::state::settings::ViewMode::Grid),
                    self.rubber_scroll_dims(tile).0,
                )
            })
            .unwrap_or((false, 1));
        if is_grid && self.scroll_grid_row_to(tile, next_pos / cols.max(1), ScrollStrategy::Center)
        {
            return;
        }
        if self.scroll_details_row_to(tile, next_pos, ScrollStrategy::Center) {
            return;
        }
        if let Some(handle) = self.scroll_handles.get(&tile) {
            handle.scroll_to_item(next_pos, ScrollStrategy::Center);
        }
    }

    pub(crate) fn nav_up(&mut self, extend: bool) {
        self.nav_apply(|c, _| c.saturating_sub(1), extend);
    }
    pub(crate) fn nav_down(&mut self, extend: bool) {
        self.nav_apply(|c, _| c + 1, extend);
    }
    pub(crate) fn nav_home(&mut self, extend: bool) {
        self.nav_apply(|_, _| 0, extend);
    }
    pub(crate) fn nav_end(&mut self, extend: bool) {
        self.nav_apply(|_, len| len - 1, extend);
    }
    pub(crate) fn nav_page(&mut self, down: bool, extend: bool) {
        const STEP: usize = 20;
        self.nav_apply(
            move |c, _| {
                if down {
                    c + STEP
                } else {
                    c.saturating_sub(STEP)
                }
            },
            extend,
        );
    }

    /// Open (or navigate into) the item under the keyboard cursor.
    pub(crate) fn open_cursor(&mut self) {
        let tile = self.state.focused_tile;
        let idx = self.state.tab_states.get(&tile).and_then(|ts| {
            ts.selection
                .keyboard_cursor
                .or_else(|| ts.selection.selected.iter().next().copied())
        });
        if let Some(idx) = idx {
            self.process_actions(vec![crate::input::action::Action::Activate { tile, idx }]);
        }
    }

    pub(crate) fn focus_next_tab(&mut self) {
        self.cycle_tab(1);
    }
    pub(crate) fn focus_prev_tab(&mut self) {
        self.cycle_tab(-1);
    }
    fn cycle_tab(&mut self, dir: isize) {
        let cur = self.state.focused_tile;
        let Some(container) = self.state.find_tabs_container(cur) else {
            return;
        };
        let children = match self.state.pane_tree.tiles.get(container) {
            Some(Tile::Container(Container::Tabs(tabs))) => tabs.children.clone(),
            _ => return,
        };
        if children.is_empty() {
            return;
        }
        let pos = children.iter().position(|&c| c == cur).unwrap_or(0) as isize;
        let n = children.len() as isize;
        let np = (((pos + dir) % n + n) % n) as usize;
        self.activate_tab(children[np]);
    }

    /// Persist all UI state to disk. Called on window close.
    pub fn save(&self) {
        save_settings(&self.state.settings);
        save_session(&self.state.pane_tree);
        save_profiles(&self.state.profiles);
        crate::util::config::save_ext_enabled(&self.state.ext.enabled);
        #[cfg(feature = "cloud")]
        crate::util::config::save_cloud_remotes(&self.state.cloud.remotes);
    }

    #[cfg(feature = "cloud")]
    pub fn on_exit(&mut self) {
        self.cleanup_cloud_on_exit();
    }
}

fn pane_bounds_changed(old: gpui::Bounds<gpui::Pixels>, new: gpui::Bounds<gpui::Pixels>) -> bool {
    const MIN_DELTA: f32 = 1.25;
    let dx = (f32::from(old.origin.x) - f32::from(new.origin.x)).abs();
    let dy = (f32::from(old.origin.y) - f32::from(new.origin.y)).abs();
    let dw = (f32::from(old.size.width) - f32::from(new.size.width)).abs();
    let dh = (f32::from(old.size.height) - f32::from(new.size.height)).abs();
    dx > MIN_DELTA || dy > MIN_DELTA || dw > MIN_DELTA || dh > MIN_DELTA
}

fn pane_bounds_moved(old: gpui::Bounds<gpui::Pixels>, new: gpui::Bounds<gpui::Pixels>) -> bool {
    const MIN_MOVE_DELTA: f32 = 1.25;
    let dx = (f32::from(old.origin.x) - f32::from(new.origin.x)).abs();
    let dy = (f32::from(old.origin.y) - f32::from(new.origin.y)).abs();
    dx > MIN_MOVE_DELTA || dy > MIN_MOVE_DELTA
}

fn pane_bounds_resized(old: gpui::Bounds<gpui::Pixels>, new: gpui::Bounds<gpui::Pixels>) -> bool {
    const MIN_SIZE_DELTA: f32 = 1.25;
    let dw = (f32::from(old.size.width) - f32::from(new.size.width)).abs();
    let dh = (f32::from(old.size.height) - f32::from(new.size.height)).abs();
    dw > MIN_SIZE_DELTA || dh > MIN_SIZE_DELTA
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BatchRenameField {
    Pattern,
    Replacement,
}

/// Build the one-time migration payload for a fresh tag database: every tag that
/// previously lived only in profile config (globals + each profile's own tags).
fn collect_tag_migration(
    profiles: &crate::state::profiles::ProfileManager,
) -> Vec<crate::state::tag_cache::TagDef> {
    use crate::state::tag_cache::{TagDef, TagId};
    let mut out = Vec::new();
    for tag in &profiles.global_tags {
        out.push(TagDef {
            id: TagId(0),
            name: tag.name.clone(),
            color: tag.color,
            profile: None,
        });
    }
    for profile in &profiles.profiles {
        for tag in &profile.tags {
            out.push(TagDef {
                id: TagId(0),
                name: tag.name.clone(),
                color: tag.color,
                profile: Some(profile.name.clone()),
            });
        }
    }
    out
}
