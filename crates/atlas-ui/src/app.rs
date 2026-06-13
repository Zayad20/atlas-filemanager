#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::state::layout_tree::{Container, Tile, TileId};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Context, Entity, FocusHandle, ScrollStrategy, UniformListScrollHandle, Window};
use gpui_component::input::InputState;
use smol::Timer;

use crate::fs::list::list_dir;
use crate::search::nucleo_pool::NucleoPool;
use crate::state::app_state::{AppState, BgResult};
use crate::state::pane::TabData;
use crate::state::preview_cache::{ImageData, PreviewRequest};
use crate::state::tab::LoadState;
use crate::util::config::{
    load_profiles, load_session, load_settings, save_profiles, save_session, save_settings,
};

pub mod actions;
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
pub mod theme;

pub struct App {
    pub(crate) state: AppState,
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
    // FASE5: keyboard focus for the active overlay (fuzzy bars / modals capture keys here).
    pub(crate) overlay_focus: Option<FocusHandle>,
    pub(crate) batch_rename_field: BatchRenameField,
    // FASE4: UI-side cache of downscaled grid thumbnails, keyed by file path.
    pub(crate) thumb_textures: HashMap<PathBuf, crate::ui::thumbs::ThumbState>,
    // FASE6: paths with an in-flight *extension* thumbnailer process. Caps how many
    // external processes (ffmpeg/magick) run at once so a big folder can't spawn
    // hundreds. Native image decodes are CPU-only and not throttled here.
    pub(crate) ext_thumb_inflight: std::collections::HashSet<PathBuf>,
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

/// In-flight inline rename of a single row in the focused pane.
pub struct RenameEdit {
    pub(crate) pane: TileId,
    pub(crate) idx: usize,
    pub(crate) path: PathBuf,
    pub(crate) input: Entity<InputState>,
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

pub struct PendingPreview {
    pub(crate) tab: TileId,
    pub(crate) request: PreviewRequest,
    pub(crate) ready_at: Instant,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let settings = load_settings();
        let profiles = load_profiles();

        let (tx, rx) = flume::unbounded();
        let (icon_tx, icon_rx) = flume::unbounded();

        // Restore previous session tree if available, else create fresh state
        let restored_tree = load_session();
        let (mut state, _initial_tile_id) = if let Some(tree) = restored_tree {
            let (s, id) = AppState::from_tree(tree, settings);
            // Discard a corrupted/empty session and start fresh
            if s.all_pane_tile_ids().is_empty() {
                AppState::new(s.settings)
            } else {
                (s, id)
            }
        } else {
            AppState::new(settings)
        };
        state.profiles = profiles;
        // Profiles saved before the cloud feature lack the Network section, which
        // would hide configured remotes — backfill it.
        #[cfg(feature = "cloud")]
        state.profiles.ensure_cloud_section();

        // Resolve the startup icon theme from the active profile's saved selection
        // (official per-profile store), falling back to VSCode auto-discovery.
        let icon_theme = {
            let map = state
                .profiles
                .active()
                .scoped::<crate::app::profiles::IconSelection>();
            if map.is_empty() {
                crate::icon_theme::find_vscode_theme()
            } else {
                let paths = map
                    .into_iter()
                    .map(|(ext, path)| (ext, std::path::PathBuf::from(path)))
                    .collect();
                crate::icon_theme::IconThemeIndex::from_mappings(paths)
            }
        };

        // Spawn the SQLite tag-db worker. On a brand-new database it imports the tags
        // that previously lived only in profile config (global + per-profile).
        let migration = collect_tag_migration(&state.profiles);
        let tag_db_tx = crate::fs::tag_db::spawn(
            &rt,
            crate::util::config::data_dir().join("tags.db"),
            migration,
            tx.clone(),
        );

        // Trigger directory loads for all panes (focused first, rest on demand)
        let pane_ids = state.all_pane_tile_ids();
        let show_hidden = state.settings.show_hidden;
        for tile_id in pane_ids {
            let (location, sort_col, sort_dir, folders_first) =
                match state.pane_tree.tiles.get(tile_id) {
                    Some(Tile::Pane(p)) => (
                        p.location.clone(),
                        p.sort_col.clone(),
                        p.sort_dir.clone(),
                        p.folders_first,
                    ),
                    _ => continue,
                };
            // A restored Access-mode remote tab can't reconnect before rclone is up.
            #[cfg(feature = "cloud")]
            if matches!(location, crate::state::location::Location::Remote { .. }) {
                continue;
            }
            // A restored virtual tag tab re-queries its paths instead of listing a dir.
            if let crate::state::location::Location::Tag(tag) = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = tag_db_tx.send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
                continue;
            }
            // A restored archive tab lists its inner directory on startup.
            if let crate::state::location::Location::Archive { archive, inner } = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let tx_clone = tx.clone();
                let archive2 = archive.clone();
                let inner2 = inner.clone();
                rt.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let provider = crate::fs::archive::open(&archive)
                            .ok_or_else(|| rust_i18n::t!("app_archive_unsupported").to_string())?;
                        provider.list(&inner).map_err(|e| e.to_string())
                    })
                    .await;
                    let send_result = match result {
                        Ok(Ok(entries)) => BgResult::ArchiveListed {
                            tab: tile_id,
                            op_id,
                            archive: archive2,
                            inner: inner2,
                            entries,
                        },
                        Ok(Err(e)) => BgResult::ArchiveListError {
                            tab: tile_id,
                            op_id,
                            error: e,
                        },
                        Err(e) => BgResult::ArchiveListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        },
                    };
                    let _ = tx_clone.send(send_result);
                });
                continue;
            }
            let path = match location.fs_dir() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };
            let op_id = state.next_op_id();
            let ts = state.tab_states.entry(tile_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = tx.clone();
            rt.spawn(async move {
                match list_dir(&path, show_hidden, sort_col, sort_dir, folders_first).await {
                    Ok(entries) => {
                        let _ = tx_clone.send(BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }

        // ── Windows OLE drag-and-drop setup ────────────────────────────────
        // Extract the HWND from gpui's HasWindowHandle impl. Stored as isize
        // so it is Send/clone-able; reconstructed to HWND only in dnd::windows.
        #[cfg(windows)]
        let (hwnd_opt, dnd_opt) = {
            use raw_window_handle::HasWindowHandle;
            let hwnd_raw = window.window_handle().ok().and_then(|h| {
                if let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw() {
                    Some(w.hwnd.get())
                } else {
                    None
                }
            });
            let dnd = hwnd_raw.and_then(|h| {
                // HiDPI scale from gpui (no cx needed for scale_factor).
                let scale = window.scale_factor();
                crate::dnd::windows::setup_dnd(h, scale)
            });
            (hwnd_raw, dnd)
        };

        // Pre-warm the shell-menu STA worker so the first right-click is instant.
        #[cfg(windows)]
        let shell_menu = Some(crate::shell_menu::spawn());

        // Extensions: seed bundled ones, load enabled overrides, build the registry.
        let _ =
            crate::ext::install::ensure_bundled("file-tools", crate::app::ext::FILE_TOOLS_MANIFEST);
        let _ = crate::ext::install::ensure_bundled(
            "media-thumbs",
            crate::app::ext::MEDIA_THUMBS_MANIFEST,
        );
        state.ext.enabled = crate::util::config::load_ext_enabled();
        let ext_registry = crate::ext::Registry::load(&state.ext.enabled);

        // Spawn the cloud worker (rclone sidecar is started lazily on first use)
        // and restore previously-configured remotes for the sidebar.
        #[cfg(feature = "cloud")]
        let cloud_tx = crate::cloud::spawn(&rt, tx.clone());
        #[cfg(feature = "cloud")]
        {
            state.cloud.remotes = crate::util::config::load_cloud_remotes();
            // Auto-mount remotes the user flagged. Not added to `navigate_on_mount`,
            // so they mount silently without hijacking the startup tab.
            for r in &state.cloud.remotes {
                if r.auto_mount && r.access == crate::cloud::AccessKind::Mount {
                    let _ = cloud_tx.send(crate::cloud::CloudCmd::Mount { id: r.id.clone() });
                }
            }
        }

        // Drain background results on a ~60 Hz foreground loop, repainting only
        // when the drain actually changed state. Breaks when the entity is gone.
        cx.spawn(async move |this, cx| loop {
            Timer::after(DRAIN_INTERVAL).await;
            let alive = this
                .update(cx, |app, cx| {
                    let mut dirty = app.drain_bg_results();
                    // Drain Windows OLE drop payloads enqueued by our IDropTarget.
                    #[cfg(windows)]
                    {
                        dirty |= app.drain_dnd_queue();
                        dirty |= app.drain_shell_menu_events(cx);
                    }
                    // Finalize a palette's exit animation once its fade-out has elapsed.
                    dirty |= app.tick_palette_close();
                    if dirty {
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();

        Self {
            state,
            tx,
            rx,
            rt,
            focus_handle,
            scroll_handles: HashMap::new(),
            watchers: HashMap::new(),
            nucleo: NucleoPool::new(),
            nucleo_results: Vec::new(),
            goto_index_root: None,
            goto_prefill: None,
            fuzzy_scorer: crate::search::fuzzy::FuzzyScorer::new(),
            palette_scroll: gpui::ScrollHandle::new(),
            ext_registry,
            pending_preview: None,
            tag_db_tx,
            #[cfg(feature = "cloud")]
            cloud_tx,
            // FASE5: preview texture caches + overlay focus start empty.
            preview_textures: HashMap::new(),
            multi_textures: HashMap::new(),

            preview_view: HashMap::new(),
            preview_drag: None,
            overlay_focus: None,
            batch_rename_field: BatchRenameField::Pattern,
            // FASE4: grid thumbnail cache starts empty.
            thumb_textures: HashMap::new(),
            ext_thumb_inflight: HashSet::new(),
            // Icon theme resolved above from the active profile (selection or
            // VSCode auto-discovery).
            icon_theme,
            icon_cache: HashMap::new(),
            icon_tx,
            icon_rx,
            // FASE A: dock built lazily on first render from the restored pane_tree.
            dock: None,
            palette_host: None,
            palette_closing: None,
            layout_revision: 0,
            dock_revision: 0,
            // FASE C: rubber-band selection state.
            rubber: None,
            row_rects: HashMap::new(),
            pending_pane_zones: Vec::new(),
            goto_input: None,
            palette_input: None,
            ext_palette_input: None,
            rename_input: None,
            search_inputs: HashMap::new(),
            batch_pattern_input: None,
            batch_replacement_input: None,
            rename_profile_input: None,
            rename_edit: None,
            content_results: Vec::new(),
            content_gen: 0,
            content_cancel: None,
            theme_dirty: true,
            sidebar_width_dirty: true,
            #[cfg(windows)]
            hwnd: hwnd_opt,
            #[cfg(windows)]
            dnd: dnd_opt,
            #[cfg(windows)]
            shell_menu,
            ctx_menu: None,
            ctx_menu_input: None,
            ctx_menu_sub: None,
            ctx_menu_seq: 0,
        }
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

    /// Tear down a dismissed palette once its exit animation has elapsed. Returns
    /// `true` when it finalized (so the caller repaints to drop the now-hidden view).
    pub(crate) fn tick_palette_close(&mut self) -> bool {
        if let Some((kind, started)) = self.palette_closing {
            if started.elapsed() >= PALETTE_CLOSE_FINALIZE {
                self.palette_closing = None;
                match kind {
                    crate::ui::palette::PaletteKind::Goto => self.goto_input = None,
                    crate::ui::palette::PaletteKind::Commands => self.palette_input = None,
                    crate::ui::palette::PaletteKind::Ext => self.ext_palette_input = None,
                }
                self.palette_host = None;
                return true;
            }
        }
        false
    }

    /// Apply every queued background result to state. Returns `true` if at least
    /// one result was processed (i.e. a repaint is warranted).
    /// Drain the Windows OLE drop-target action queue and route payloads to
    /// `process_actions`. Called in the foreground drain loop so drops appear
    /// on the next render after the COM vtable returns.
    #[cfg(windows)]
    pub(crate) fn drain_dnd_queue(&mut self) -> bool {
        use crate::dnd::windows::DndEvent;
        use crate::input::action::Action;
        use crate::state::ops::{ConflictPolicy, TransferKind, TransferOrigin, TransferRequest};

        let Some(dnd) = &self.dnd else { return false };
        let events = {
            let Ok(mut q) = dnd.action_queue.lock() else {
                return false;
            };
            std::mem::take(&mut *q)
        };
        if events.is_empty() {
            return false;
        }
        let mut had = false;
        let mut actions = Vec::with_capacity(events.len());
        for event in events {
            let p = match event {
                DndEvent::DropIn(p) => p,
                DndEvent::DragOutMoved { paths } => {
                    // Drag out of Atlas accepted as MOVE by the target: the
                    // sources go to the trash (recoverable). Files already gone
                    // surface as visible toasts via delete_files.
                    had = true;
                    self.delete_files(paths, true);
                    continue;
                }
                DndEvent::Error(msg) => {
                    had = true;
                    self.toast_error(msg);
                    continue;
                }
            };
            // Resolve destination: use the App's focused dir as fallback.
            let dest = if p.x >= 0.0 && p.y >= 0.0 {
                // Drop target already computed a zone-based dest inside the COM vtable;
                // we trust it. If it is empty, fall back to focused tab dir.
                if p.paths.is_empty() {
                    continue;
                }
                // Re-check zones: the payload carries the logical drop position.
                let dest = self.dnd.as_ref().and_then(|d| {
                    d.zones
                        .lock()
                        .ok()
                        .and_then(|z| z.hit(p.x, p.y).map(|z| z.dest.clone()))
                });
                dest.or_else(|| self.state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())))
            } else {
                self.state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
            };
            let Some(dest) = dest else { continue };
            // Vtable always enqueues Copy; decide move vs copy here by volume.
            // Conflicts always go through the dialog (policy `Ask`) — a drop
            // must never overwrite or vanish silently.
            let vol_effect = crate::dnd::default_effect_for_paths(&p.paths, &dest);
            actions.push(Action::Transfer(TransferRequest {
                sources: p.paths,
                dest,
                kind: if vol_effect.is_move() {
                    TransferKind::Move
                } else {
                    TransferKind::Copy
                },
                policy: ConflictPolicy::Ask,
                origin: if p.from_self {
                    TransferOrigin::DndInternal
                } else {
                    TransferOrigin::DndExternal
                },
            }));
        }
        let had = had || !actions.is_empty();
        self.process_actions(actions);
        had
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
                    q.push(crate::dnd::windows::DndEvent::Error(rust_i18n::t!(
                        "app_path_not_exists",
                        path = p.file_name().unwrap_or_default().to_string_lossy()
                    ).to_string()));
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
        if let Some(dest) = self.tab_data_for(pane_id).and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
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

    pub(crate) fn drain_bg_results(&mut self) -> bool {
        // Drain icon channel first (dedicated channel, Arc<RenderImage> direct).
        let icon_results: Vec<_> = self.icon_rx.try_iter().collect();
        let mut changed = !icon_results.is_empty();
        for (key, result) in icon_results {
            let state = match result {
                Some(tex) => crate::icon_theme::IconState::Ready(tex),
                None => crate::icon_theme::IconState::Missing,
            };
            self.icon_cache.insert(key, state);
        }

        let results: Vec<BgResult> = self.rx.try_iter().collect();
        changed |= !results.is_empty();
        for result in results {
            match result {
                BgResult::DirListed {
                    tab,
                    op_id,
                    entries,
                } => {
                    // Collect icon keys before moving `entries` so we can warm
                    // the icon cache for the new listing.
                    let icon_keys: Vec<String> = if self.icon_theme.is_some() {
                        crate::icon_theme::collect_keys(&entries)
                    } else {
                        vec![]
                    };
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        // Ignore stale results from a superseded navigation
                        if op_id == ts.last_nav_op_id {
                            // Preserve selection across refreshes (e.g. fs-watcher
                            // events) by remapping the previously-selected paths onto
                            // the new listing. A real navigation already cleared the
                            // selection via begin_navigation, so nothing is preserved
                            // there.
                            let prev_selected: HashSet<PathBuf> = ts
                                .selection
                                .selected
                                .iter()
                                .filter_map(|&i| ts.entries.get(i).map(|e| e.path.clone()))
                                .collect();
                            let prev_cursor = ts
                                .selection
                                .keyboard_cursor
                                .and_then(|i| ts.entries.get(i))
                                .map(|e| e.path.clone());
                            ts.set_entries(entries);
                            ts.load_state = LoadState::Loaded;
                            ts.selection.selected = ts
                                .entries
                                .iter()
                                .enumerate()
                                .filter(|(_, e)| prev_selected.contains(&e.path))
                                .map(|(i, _)| i)
                                .collect();
                            ts.selection.keyboard_cursor = prev_cursor
                                .and_then(|p| ts.entries.iter().position(|e| e.path == p));
                            ts.selection.selection_anchor = ts.selection.keyboard_cursor;
                            ts.selection.last_clicked_index = ts.selection.keyboard_cursor;
                        }
                    }
                    // Borrow of tab_states released — safe to request icons.
                    for key in icon_keys {
                        self.request_icon(key);
                    }
                }
                BgResult::MillerListed {
                    tab,
                    location,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if let Some(c) = ts
                            .miller
                            .columns
                            .iter_mut()
                            .find(|c| c.location == location && c.op_id == op_id)
                        {
                            c.entries = entries;
                            c.load_state = LoadState::Loaded;
                        }
                    }
                }
                BgResult::DirListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                }
                BgResult::FsEvent(changed_path) => {
                    // Refresh any pane watching the directory containing the changed path
                    let parent = changed_path.parent().map(std::path::Path::to_path_buf);
                    let to_refresh: Vec<TileId> = self
                        .watchers
                        .iter()
                        .filter(|(_, handle)| {
                            Some(&handle.watched_path) == parent.as_ref()
                                || handle.watched_path == changed_path
                        })
                        .map(|(&id, _)| id)
                        .collect();
                    for tile_id in to_refresh {
                        if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get(tile_id) {
                            if let Some(path) = p.location.fs_dir().map(|p| p.to_path_buf()) {
                                let col = p.sort_col.clone();
                                let dir = p.sort_dir.clone();
                                self.spawn_list_dir(tile_id, path, col, dir);
                            }
                        }
                    }
                }
                BgResult::PreviewReady { tab, path, result } => {
                    if self.state.preview.paths.get(&tab) != Some(&path) {
                        continue;
                    }
                    // The decoded RGBA stays in `result`; the UI layer uploads it to a
                    // GPU texture and caches that handle on its own side (state holds no
                    // renderer texture).
                    self.state.preview.results.insert(tab, result);
                }
                BgResult::MultiPreviewThumbnailReady {
                    tab,
                    key,
                    path,
                    rgba,
                    width,
                    height,
                } => {
                    let Some(multi) = self.state.preview.multi.get_mut(&tab) else {
                        continue;
                    };
                    if multi.key != key
                        || !multi.paths.contains(&path)
                        || multi.images.iter().any(|(loaded, _)| loaded == &path)
                    {
                        continue;
                    }
                    multi.images.push((
                        path,
                        ImageData {
                            rgba,
                            width,
                            height,
                        },
                    ));
                }
                BgResult::GridThumbnailReady {
                    path,
                    rgba,
                    width,
                    height,
                } => {
                    self.apply_grid_thumbnail(path, rgba, width, height);
                }

                BgResult::SearchInPlaceResults { tab, gen, entries } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen
                            && ts.search.active
                            && ts.search.recursive
                            && ts.search.is_filtering()
                        {
                            ts.set_entries(entries);
                            ts.filter.clear();
                            ts.search.status = crate::state::tab::SearchStatus::Ready;
                            ts.search.error = None;
                            ts.selection.clear();
                            if !ts.entries.is_empty() {
                                ts.selection.selected.insert(0);
                                ts.selection.keyboard_cursor = Some(0);
                                ts.selection.selection_anchor = Some(0);
                                ts.selection.last_clicked_index = Some(0);
                                ts.selection.scroll_to_selected = true;
                            }
                        }
                    }
                }
                BgResult::SearchInPlaceError { tab, gen, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen && ts.search.active {
                            ts.search.status = crate::state::tab::SearchStatus::Error;
                            ts.search.error = Some(error);
                        }
                    }
                }
                BgResult::OpComplete {
                    id,
                    result,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    if let Err(e) = result {
                        tracing::error!("Background operation failed: {}", e);
                        // Multi-line payloads are one error per item; toast each.
                        for line in e.lines().filter(|l| !l.trim().is_empty()) {
                            self.toast_error(line.to_string());
                        }
                    }
                    self.refresh_tabs(affected_tabs);
                }
                BgResult::ClipboardPerformed { effect, paths } => {
                    // A paste target consumed our clipboard cut/copy.
                    // effect 2 (MOVE) = unoptimized move: we must trash sources.
                    // effect 0 (NONE) = optimized move: target already moved them.
                    // effect 1 (COPY) = plain copy: nothing to do.
                    if effect == 2 || effect == 0 {
                        let was_cut = self.state.ops.clipboard.as_ref().is_some_and(|c| c.is_cut);
                        if was_cut {
                            if effect == 2 {
                                // Quietly skip already-gone files: an optimized
                                // mover may still report MOVE for some targets.
                                let existing: Vec<_> =
                                    paths.iter().filter(|p| p.exists()).cloned().collect();
                                if !existing.is_empty() {
                                    self.delete_files(existing, true);
                                }
                            }
                            self.state.ops.clipboard = None;
                            #[cfg(windows)]
                            crate::dnd::windows::ole_clipboard::clear();
                            let affected: std::collections::HashSet<_> = paths
                                .iter()
                                .flat_map(|p| self.affected_tabs_for_path(p))
                                .collect();
                            self.refresh_tabs(affected.into_iter().collect());
                        }
                    }
                }
                BgResult::TransferComplete {
                    id,
                    kind,
                    origin,
                    report,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    for err in &report.errors {
                        tracing::error!("transfer: {}", err);
                        self.toast_error(err.to_string());
                    }
                    // Files cut in another app and pasted here: report the move
                    // back so the source clears its cut state — only when
                    // everything actually moved.
                    #[cfg(windows)]
                    if origin == crate::state::ops::TransferOrigin::PasteExternalCut
                        && report.errors.is_empty()
                        && !report.completed.is_empty()
                    {
                        crate::dnd::windows::ole_clipboard::acknowledge_external_cut();
                    }
                    if report.cancelled {
                        self.toast_error(format!(
                            "Operación cancelada ({} completados)",
                            report.completed.len()
                        ));
                    }
                    if !report.completed.is_empty() {
                        // Tags follow files moved within the app (real pairs,
                        // so this survives conflict renames).
                        if kind == crate::state::ops::TransferKind::Move {
                            for (src, dst) in &report.completed {
                                let _ =
                                    self.tag_db_tx
                                        .send(crate::fs::tag_db::TagDbCmd::RenamePath {
                                            old: src.clone(),
                                            new: dst.clone(),
                                        });
                            }
                        }
                        // Undo records what actually happened; undo/redo runs
                        // don't re-record themselves.
                        if origin != crate::state::ops::TransferOrigin::Undo {
                            let entry = match kind {
                                crate::state::ops::TransferKind::Move => {
                                    crate::state::undo::UndoEntry::Move {
                                        pairs: report.completed.clone(),
                                    }
                                }
                                crate::state::ops::TransferKind::Copy => {
                                    crate::state::undo::UndoEntry::Copy {
                                        pairs: report.completed.clone(),
                                    }
                                }
                            };
                            self.state.ops.undo_stack.push(entry);
                        }
                    }
                    self.refresh_tabs(affected_tabs);
                }
                BgResult::OpProgress {
                    id,
                    done,
                    total,
                    bytes_done,
                    bytes_total,
                    current_file,
                } => {
                    if let Some(op) = self.state.ops.active_ops.get_mut(&id) {
                        op.done = done;
                        op.total = total;
                        op.bytes_done = bytes_done;
                        op.bytes_total = bytes_total;
                        op.current_file = current_file;
                    }
                }
                BgResult::TagDefinitions(defs) => {
                    self.state.tag_cache.apply_definitions(defs);
                }
                BgResult::TagsForPaths { tags } => {
                    self.state.tag_cache.apply_for_paths(tags);
                }
                BgResult::PathsForTag { tag, paths } => {
                    self.populate_tag_tabs(tag, paths);
                }
                BgResult::ContentResults { gen, mut paths } => {
                    // Drop stale batches from a superseded query generation.
                    if gen == self.content_gen {
                        self.content_results.append(&mut paths);
                    }
                }
                BgResult::DeviceMounted(path) => {
                    crate::fs::udisks::invalidate();
                    self.navigate(self.state.focused_tile, path);
                }
                BgResult::DeviceActionFailed(error) => {
                    tracing::warn!("device action failed: {error}");
                }
                BgResult::DevicesChanged => {
                    crate::fs::udisks::invalidate();
                }
                BgResult::ExtCommandDone {
                    ext_id,
                    error,
                    dirs,
                } => {
                    self.handle_ext_bg(ext_id, error, dirs);
                }
                BgResult::ExtShowQuickPick { items, reply_tx } => {
                    self.state.overlay.show_ext_palette = true;
                    self.state.overlay.ext_palette_items = items;
                    self.state.overlay.ext_palette_reply = Some(reply_tx);
                    self.state.overlay.ext_palette_state.reset();
                    // Fresh input + host so the pick opens empty, re-subscribed
                    // and with the open animation (mirrors OpenPalette/Goto).
                    self.ext_palette_input = None;
                    self.palette_host = None;
                    self.palette_closing = None;
                    changed = true;
                }
                BgResult::ArchiveListed {
                    tab,
                    op_id,
                    entries,
                    ..
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.set_entries(entries);
                            ts.load_state = LoadState::Loaded;
                        }
                    }
                }
                BgResult::ArchiveListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                }
                #[cfg(feature = "cloud")]
                other => self.handle_cloud_bg(other),
            }
        }
        changed
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
        self.scroll_handles.entry(tile).or_default().clone()
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
        if let Some(h) = self.scroll_handles.get(&tile) {
            h.scroll_to_item(next_pos, ScrollStrategy::Center);
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
