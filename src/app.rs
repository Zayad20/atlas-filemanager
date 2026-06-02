use egui_tiles::TileId;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::fs::list::list_dir;
use crate::fs::preview::PreviewResult;
use crate::input::action::Action;
use crate::input::shortcuts::handle_global_shortcuts;
use crate::state::app_state::{AppState, BgResult};
use crate::state::pane::TabData;
use crate::state::preview_cache::PreviewRequest;
use crate::state::tab::LoadState;
use crate::ui::layout::show_layout;
use crate::util::config::{
    load_profiles, load_session, load_settings, save_profiles, save_session, save_settings,
};

pub mod actions;
pub mod drag;
pub mod fs_sync;
pub mod nav;
pub mod ops;
pub mod pane_ops;
pub mod profiles;

pub struct App {
    pub(crate) state: AppState,
    pub(crate) tx: flume::Sender<BgResult>,
    pub(crate) rx: flume::Receiver<BgResult>,
    pub(crate) rt: tokio::runtime::Handle,
    /// Last theme/font applied to the egui context, to avoid rebuilding the
    /// whole Style every frame (which clones a large struct needlessly).
    pub(crate) applied_theme: Option<crate::state::settings::Theme>,
    pub(crate) applied_font_size: f32,
    pub(crate) pending_preview: Option<PendingPreview>,
    pub(crate) pending_native_drag: Option<Vec<PathBuf>>,
    /// Channel to the SQLite tag-db worker (writes fire-and-forget, reads reply via `tx`).
    pub(crate) tag_db_tx: flume::Sender<crate::fs::tag_db::TagDbCmd>,
}

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(90);

pub(crate) struct PendingPreview {
    pub(crate) tab: TileId,
    pub(crate) request: PreviewRequest,
    pub(crate) ready_at: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        #[cfg(target_os = "linux")]
        crate::util::linux_blur::request_blur(cc);
        let settings = cc.storage.map(load_settings).unwrap_or_default();
        let profiles = cc.storage.map(load_profiles).unwrap_or_default();

        let (tx, rx) = flume::unbounded();

        // Restore previous session tree if available, else create fresh state
        let restored_tree = cc.storage.and_then(load_session);
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
            let (path, sort_col, sort_dir, source) = match state.pane_tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(p)) => (
                    p.path.clone(),
                    p.sort_col.clone(),
                    p.sort_dir.clone(),
                    p.source.clone(),
                ),
                _ => continue,
            };
            // A restored virtual tag tab re-queries its paths instead of listing a dir.
            if let crate::state::pane::TabSource::Tag(tag) = source {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = tag_db_tx.send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
                continue;
            }
            let op_id = state.next_op_id();
            let ts = state.tab_states.entry(tile_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = tx.clone();
            rt.spawn(async move {
                match list_dir(&path, show_hidden, sort_col, sort_dir).await {
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

        Self {
            state,
            tx,
            rx,
            rt,
            applied_theme: None,
            applied_font_size: 0.0,
            pending_preview: None,
            pending_native_drag: None,
            tag_db_tx,
        }
    }

    fn drain_bg_results(&mut self, ctx: &egui::Context) {
        let results: Vec<BgResult> = self.rx.try_iter().collect();
        for result in results {
            match result {
                BgResult::DirListed {
                    tab,
                    op_id,
                    entries,
                } => {
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
                            ts.file_list_hit_regions = None;
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
                }
                BgResult::MillerListed {
                    tab,
                    path,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if let Some(c) = ts
                            .miller
                            .columns
                            .iter_mut()
                            .find(|c| c.path == path && c.op_id == op_id)
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
                    let parent = changed_path.parent().map(|p| p.to_path_buf());
                    let to_refresh: Vec<TileId> = self
                        .state
                        .watchers
                        .iter()
                        .filter(|(_, handle)| {
                            Some(&handle.watched_path) == parent.as_ref()
                                || handle.watched_path == changed_path
                        })
                        .map(|(&id, _)| id)
                        .collect();
                    for tile_id in to_refresh {
                        if let Some(egui_tiles::Tile::Pane(p)) =
                            self.state.pane_tree.tiles.get(tile_id)
                        {
                            let path = p.path.clone();
                            let col = p.sort_col.clone();
                            let dir = p.sort_dir.clone();
                            self.spawn_list_dir(tile_id, path, col, dir);
                        }
                    }
                }
                BgResult::PreviewReady {
                    tab,
                    path,
                    mut result,
                } => {
                    if self.state.preview.paths.get(&tab) != Some(&path) {
                        continue;
                    }
                    match &mut result {
                        PreviewResult::Image {
                            rgba,
                            width,
                            height,
                            ..
                        }
                        | PreviewResult::Pdf {
                            rgba,
                            width,
                            height,
                        } => {
                            let color_image =
                                egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba);
                            self.state.preview.textures.insert(
                                tab,
                                ctx.load_texture(
                                    format!("preview_img_{tab:?}"),
                                    color_image,
                                    egui::TextureOptions::default(),
                                ),
                            );
                            // The pixels now live in the GPU texture and the inspector
                            // only reads width/height/exif — drop the CPU-side copy so a
                            // 4K preview doesn't sit in RAM duplicated for the tab's life.
                            *rgba = Vec::new();
                        }
                        _ => {
                            self.state.preview.textures.remove(&tab);
                        }
                    }
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
                        || multi.textures.iter().any(|(loaded, _)| loaded == &path)
                    {
                        continue;
                    }
                    let color_image =
                        egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    multi.textures.push((
                        path,
                        ctx.load_texture(
                            format!("multi_preview_img_{tab:?}_{}", multi.textures.len()),
                            color_image,
                            egui::TextureOptions::default(),
                        ),
                    ));
                }
                BgResult::OpComplete {
                    id,
                    result,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    if let Err(e) = result {
                        tracing::error!("Background operation failed: {}", e);
                    }
                    for tab_id in affected_tabs {
                        if let Some(tab_data) = self.tab_data_for(tab_id) {
                            let path = tab_data.path.clone();
                            let sort_col = tab_data.sort_col.clone();
                            let sort_dir = tab_data.sort_dir.clone();
                            self.spawn_list_dir(tab_id, path, sort_col, sort_dir);
                        }
                    }
                }
                BgResult::OpProgress {
                    id,
                    done,
                    total,
                    current_file,
                } => {
                    if let Some(op) = self.state.ops.active_ops.get_mut(&id) {
                        op.done = done;
                        op.total = total;
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
            }
        }
    }

    fn tab_data_for(&self, tab_id: TileId) -> Option<&TabData> {
        match self.state.pane_tree.tiles.get(tab_id) {
            Some(egui_tiles::Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Matches the window: transparent framebuffer only where the window itself is
        // transparent (Linux). Elsewhere paint opaque to avoid a see-through background.
        #[cfg(target_os = "linux")]
        {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        }
        #[cfg(not(target_os = "linux"))]
        {
            _visuals.window_fill().to_normalized_gamma_f32()
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Only touch the egui style when theme/font actually changed — otherwise we
        // clone the whole Style every frame for nothing.
        let theme = self.state.profiles.active().theme.clone();
        if self.applied_theme.as_ref() != Some(&theme) {
            crate::util::themes::apply_theme(ctx, &theme);
            self.applied_theme = Some(theme);
        }
        let font_size = self.state.settings.font_size;
        if (self.applied_font_size - font_size).abs() > f32::EPSILON {
            apply_font_size(ctx, font_size);
            self.applied_font_size = font_size;
        }
        // Sanitize focused_tile so all shortcuts and UI operate on a valid pane
        if !self.state.tab_states.contains_key(&self.state.focused_tile) {
            if let Some(first) = self.state.all_pane_tile_ids().first().copied() {
                self.state.focused_tile = first;
            }
        }

        self.drain_bg_results(ctx);
        self.sync_selection_tags();

        // Tick nucleo and update results when goto bar is open
        if self.state.overlay.show_goto_bar {
            self.state
                .nucleo
                .set_pattern(&self.state.overlay.goto_state.query);
            self.state.nucleo.tick();
            self.state.nucleo_results = self.state.nucleo.results(200);
            let current = self.state.active_tab_data().map(|tab| tab.path.clone());
            crate::fs::index::prioritize_folders(
                &mut self.state.nucleo_results,
                current.as_deref(),
            );
            ctx.request_repaint();
        }

        let mut pointer_actions = Vec::new();
        crate::ui::file_view_common::handle_pointer_events(
            ctx,
            &mut self.state.tab_states,
            &mut pointer_actions,
        );
        self.process_actions(pointer_actions);

        let mut actions = Vec::new();
        handle_global_shortcuts(ctx, &self.state, &mut actions);

        show_layout(ctx, &mut self.state, &mut actions);
        // egui_tiles may have restructured the tree during show_layout (tab
        // drag-to-split moves/prunes tiles). Reconcile side-tables and focus
        // before any action runs against a stale tile.
        self.reconcile_tree_state();
        // egui_tiles relocates/prunes tiles at the very end of its own `ui` pass when a
        // tab drag is dropped, without emitting any Action. Without a follow-up repaint
        // the restructured layout would not paint until the next input event, leaving the
        // new pane blank. `dragged_id` is also set on the frame the drag stops, so this
        // keeps painting through the drag and forces the settling frame to render.
        if self.state.pane_tree.dragged_id(ctx).is_some() {
            ctx.request_repaint();
        }
        self.tick_drag_hover(ctx, &mut actions);
        if self.state.drag_drop.dragged_files.is_some()
            && ctx.input(|input| input.pointer.any_down() && !input.pointer.has_pointer())
        {
            let paths = self
                .state
                .drag_drop
                .dragged_files
                .as_ref()
                .map(|drag| drag.paths.clone())
                .unwrap_or_default();
            if crate::util::native_drag::start(frame, paths) {
                self.state.drag_drop.dragged_files = None;
            }
        }

        let external_paths: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !external_paths.is_empty() {
            // Folders dropped on the tab strip open as new tabs instead of being copied.
            let strip_dirs: Vec<PathBuf> = match self.state.drag_drop.tab_strip_drop.take() {
                Some(container) => external_paths
                    .iter()
                    .filter(|p| p.is_dir())
                    .map(|path| (container, path.clone()))
                    .map(|(pane, path)| {
                        actions.push(Action::NewTabWithPath {
                            pane,
                            path: path.clone(),
                        });
                        path
                    })
                    .collect(),
                None => Vec::new(),
            };
            // Anything not opened as a tab falls back to a normal copy into the drop target.
            let remaining: Vec<PathBuf> = external_paths
                .into_iter()
                .filter(|p| !strip_dirs.contains(p))
                .collect();
            if !remaining.is_empty() {
                let target = self
                    .state
                    .drag_drop
                    .external_drop_target
                    .take()
                    .or_else(|| {
                        self.state.active_tab_data().map(|pane| {
                            crate::state::drag::ExternalDropTarget {
                                tab: self.state.focused_tile,
                                dest: pane.path.clone(),
                            }
                        })
                    });
                if let Some(target) = target {
                    self.state.focused_tile = target.tab;
                    actions.push(Action::CopyTo {
                        paths: remaining,
                        dest: target.dest,
                    });
                }
            }
        } else if ctx.input(|input| input.raw.hovered_files.is_empty()) {
            self.state.drag_drop.external_drop_target = None;
        }

        let pointer_released = ctx.input(|input| input.pointer.any_released());
        let internal_drop = actions
            .iter()
            .any(|action| matches!(action, Action::Move { .. }));
        let had_internal_drag = self.state.drag_drop.dragged_files.is_some();
        if pointer_released {
            self.state.drag_drop.dragged_files = None;
            // A sidebar item dropped over the view opens in a new tab in the active
            // tabs container (a plain click never starts a drag, so this is safe).
            if let Some(item) = self.state.drag_drop.sidebar_drag.take() {
                if let Some(pane) = self
                    .state
                    .find_tabs_container(self.state.focused_tile)
                    .or_else(|| self.state.find_any_tabs_container())
                {
                    match item {
                        crate::state::drag::SidebarDragItem::Path(path) => {
                            actions.push(Action::NewTabWithPath { pane, path });
                        }
                        crate::state::drag::SidebarDragItem::Tag(tag) => {
                            actions.push(Action::OpenTagResults { pane, tag });
                        }
                    }
                }
            }
        }

        let had_actions = !actions.is_empty();
        self.process_actions(actions);
        let mut shelf_actions = Vec::new();
        crate::ui::drop_shelf::show_drop_shelf(
            ctx,
            &mut self.state.drag_drop.drop_shelf_open,
            &mut self.state.drag_drop.drop_shelf_files,
            &mut self.pending_native_drag,
            &mut shelf_actions,
        );
        if let Some(paths) = self.pending_native_drag.take() {
            crate::util::native_drag::start(frame, paths);
        }
        self.process_actions(shelf_actions);
        if pointer_released && had_internal_drag && !internal_drop {
            self.cancel_spring_navigation();
        }
        self.sync_active_preview(ctx);

        // Actions mutate state after the frame was built (e.g. selection). Force one more
        // frame so the visual result shows immediately instead of waiting for the next
        // input event or egui going idle — this is what makes rapid clicking feel instant.
        if had_actions || self.state.any_loading() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        save_settings(storage, &self.state.settings);
        save_session(storage, &self.state.pane_tree);
        save_profiles(storage, &self.state.profiles);
    }
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

fn apply_font_size(ctx: &egui::Context, size: f32) {
    ctx.style_mut(|style| {
        for (text_style, font_id) in &mut style.text_styles {
            font_id.size = match text_style {
                egui::TextStyle::Small => size - 2.0,
                egui::TextStyle::Heading => size + 5.0,
                _ => size,
            };
        }
    });
}
