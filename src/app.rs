use egui_tiles::TileId;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::fs::preview::PreviewResult;
use crate::fs::read_dir::list_dir;
use crate::input::action::Action;
use crate::input::shortcuts::handle_global_shortcuts;
use crate::state::app_state::{AppState, BgResult, DragHoverTarget};
use crate::state::file_entry::FileKind;
use crate::state::pane::TabData;
use crate::state::tab::{LoadState, TabState};
use crate::ui::layout::show_layout;
use crate::util::config::{
    load_profiles, load_session, load_settings, save_profiles, save_session, save_settings,
};

pub struct App {
    state: AppState,
    tx: flume::Sender<BgResult>,
    rx: flume::Receiver<BgResult>,
    rt: tokio::runtime::Handle,
    /// Last theme/font applied to the egui context, to avoid rebuilding the
    /// whole Style every frame (which clones a large struct needlessly).
    applied_theme: Option<crate::state::settings::Theme>,
    applied_font_size: f32,
    pending_preview: Option<PendingPreview>,
    pending_native_drag: Option<Vec<PathBuf>>,
}

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(90);

struct PendingPreview {
    tab: TileId,
    path: PathBuf,
    ready_at: Instant,
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

        // Trigger directory loads for all panes (focused first, rest on demand)
        let pane_ids = state.all_pane_tile_ids();
        let show_hidden = state.settings.show_hidden;
        for tile_id in pane_ids {
            let path = match state.pane_tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(p)) => p.path.clone(),
                _ => continue,
            };
            let sort_col = match state.pane_tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(p)) => p.sort_col.clone(),
                _ => Default::default(),
            };
            let sort_dir = match state.pane_tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(p)) => p.sort_dir.clone(),
                _ => Default::default(),
            };
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
                            ts.entries = entries;
                            ts.load_state = LoadState::Loaded;
                            ts.selected.clear();
                            ts.file_list_hit_regions = None;
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
                BgResult::PreviewReady { tab, path, result } => {
                    if self.state.preview_paths.get(&tab) != Some(&path) {
                        continue;
                    }
                    let image = match &result {
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
                        } => Some((rgba, *width, *height)),
                        _ => None,
                    };
                    if let Some((rgba, width, height)) = image {
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied([width, height], rgba);
                        self.state.preview_textures.insert(
                            tab,
                            ctx.load_texture(
                                format!("preview_img_{tab:?}"),
                                color_image,
                                egui::TextureOptions::default(),
                            ),
                        );
                    } else {
                        self.state.preview_textures.remove(&tab);
                    }
                    self.state.preview_results.insert(tab, result);
                }
                BgResult::OpComplete {
                    id,
                    result: _,
                    affected_tabs,
                } => {
                    self.state.active_ops.remove(&id);
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
                    if let Some(op) = self.state.active_ops.get_mut(&id) {
                        op.done = done;
                        op.total = total;
                        op.current_file = current_file;
                    }
                }
                _ => {}
            }
        }
    }

    fn process_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Navigate { tab, path } => self.navigate(tab, path),
                Action::NavigateBack(tab_id) => self.navigate_back(tab_id),
                Action::NavigateForward(tab_id) => self.navigate_forward(tab_id),
                Action::NavigateUp(tab_id) => self.navigate_up(tab_id),
                Action::SpringNavigate { tab, path } => self.spring_navigate(tab, path),
                Action::SpringFocusTab(tab_id) => {
                    self.state
                        .drag_hover
                        .original_focused_tile
                        .get_or_insert(self.state.focused_tile);
                    self.activate_tab(tab_id);
                }
                Action::EditLocation(tab_id) => {
                    let path = self.tab_data_for(tab_id).map(|pane| pane.path.clone());
                    if let (Some(path), Some(tab_state)) =
                        (path, self.state.tab_states.get_mut(&tab_id))
                    {
                        tab_state.path_edit_text = path.to_string_lossy().into_owned();
                        tab_state.path_editing = true;
                        tab_state.path_edit_focus = true;
                        tab_state.path_edit_error = None;
                    }
                }
                Action::NewTab(pane_id) => self.new_tab(pane_id),
                Action::NewTabFocused => {
                    let tabs_id = self
                        .state
                        .find_tabs_container(self.state.focused_tile)
                        .or_else(|| self.state.find_any_tabs_container())
                        .unwrap_or(self.state.focused_tile);
                    self.new_tab(tabs_id);
                }
                Action::CloseTab(tab_id) => self.close_tab(tab_id),
                Action::CloseFocusedTab => {
                    let tile = self.state.focused_tile;
                    self.close_tab(tile);
                }
                Action::FocusTab(tab_id) => {
                    self.activate_tab(tab_id);
                }
                Action::ToggleHidden => {
                    self.state.settings.show_hidden = !self.state.settings.show_hidden;
                    let tab_id = self.state.focused_tile;
                    if let Some(tab_data) = self.tab_data_for(tab_id) {
                        let path = tab_data.path.clone();
                        let col = tab_data.sort_col.clone();
                        let dir = tab_data.sort_dir.clone();
                        self.spawn_list_dir(tab_id, path, col, dir);
                    }
                }
                Action::ToggleInspector => {
                    if let Some(tab) = self.state.active_tab_data_mut() {
                        tab.preview_enabled = !tab.preview_enabled;
                    }
                }
                Action::ToggleSidebar => {
                    self.state.settings.show_sidebar = !self.state.settings.show_sidebar;
                }
                Action::OpenSettings => {
                    self.state.show_settings = true;
                }
                Action::ToggleDropShelf => {
                    self.state.drop_shelf_open = !self.state.drop_shelf_open;
                }
                Action::CloseOverlays => {
                    self.state.show_goto_bar = false;
                    self.state.show_palette = false;
                }
                Action::OpenGotoBar => {
                    self.state.show_goto_bar = true;
                    self.state.goto_state.reset();
                    // Rebuild index from the current pane's directory
                    self.state.nucleo.reset();
                    let root = self
                        .state
                        .active_tab_data()
                        .map(|td| td.path.clone())
                        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
                    let injector = self.state.nucleo.injector();
                    crate::fs::index::build_index(root, injector);
                }
                Action::OpenPalette => {
                    self.state.show_palette = true;
                    self.state.palette_state.reset();
                }
                Action::ClosePalette => {
                    self.state.show_palette = false;
                    self.state.show_goto_bar = false;
                }
                Action::Refresh(tab_id) => {
                    if let Some(tab_data) = self.tab_data_for(tab_id) {
                        let path = tab_data.path.clone();
                        let col = tab_data.sort_col.clone();
                        let dir = tab_data.sort_dir.clone();
                        self.spawn_list_dir(tab_id, path, col, dir);
                    }
                }
                Action::Select {
                    tile,
                    idx,
                    shift,
                    ctrl,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        let visible: Vec<usize> = ts.visible_indices().collect();
                        if ctrl {
                            if ts.selected.contains(&idx) {
                                ts.selected.remove(&idx);
                            } else {
                                ts.selected.insert(idx);
                            }
                        } else if shift {
                            let anchor = ts.last_clicked_index.unwrap_or(idx);
                            ts.selected.clear();
                            if let (Some(anchor_pos), Some(idx_pos)) = (
                                visible
                                    .iter()
                                    .position(|visible_idx| *visible_idx == anchor),
                                visible.iter().position(|visible_idx| *visible_idx == idx),
                            ) {
                                let lo = anchor_pos.min(idx_pos);
                                let hi = anchor_pos.max(idx_pos);
                                ts.selected.extend(visible[lo..=hi].iter().copied());
                            }
                        } else {
                            ts.selected.clear();
                            ts.selected.insert(idx);
                        }
                        ts.last_clicked_index = Some(idx);
                        ts.keyboard_cursor = Some(idx);
                        ts.selection_anchor = Some(idx);
                    }
                    self.state.focused_tile = tile;
                }
                Action::ClearSelection(tile) => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        ts.selected.clear();
                        ts.keyboard_cursor = None;
                        ts.selection_anchor = None;
                    }
                    self.state.focused_tile = tile;
                }
                Action::Activate { tile, idx } => {
                    let entry = self
                        .state
                        .tab_states
                        .get(&tile)
                        .and_then(|ts| ts.entries.get(idx))
                        .map(|entry| (entry.path.clone(), entry.kind.clone()));
                    if let Some((path, kind)) = entry {
                        if kind == FileKind::Dir {
                            self.navigate(tile, path);
                        } else {
                            self.spawn_open_files(vec![path]);
                        }
                    }
                }
                Action::SelectAll => {
                    if let Some(ts) = self.state.active_tab_state_mut() {
                        ts.selected = ts.visible_indices().collect();
                        ts.keyboard_cursor = ts.selected.iter().next().copied();
                    }
                }
                Action::SetSort { col, dir } => {
                    if let Some(egui_tiles::Tile::Pane(pane)) =
                        self.state.pane_tree.tiles.get_mut(self.state.focused_tile)
                    {
                        pane.sort_col = col;
                        pane.sort_dir = dir;
                        let path = pane.path.clone();
                        let sort_col = pane.sort_col.clone();
                        let sort_dir = pane.sort_dir.clone();
                        self.spawn_list_dir(self.state.focused_tile, path, sort_col, sort_dir);
                    }
                }
                Action::Open(paths) => {
                    self.spawn_open_files(paths);
                }
                Action::CopyPathToClipboard(path) => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(path.to_string_lossy().to_string());
                    }
                }
                Action::Copy(paths) => {
                    self.state.clipboard = Some(crate::state::app_state::ClipboardState {
                        paths,
                        is_cut: false,
                    });
                }
                Action::Cut(paths) => {
                    self.state.clipboard = Some(crate::state::app_state::ClipboardState {
                        paths,
                        is_cut: true,
                    });
                }
                Action::Delete { paths, to_trash } => self.delete_files(paths, to_trash),
                Action::Rename { from, to } => self.rename_file(from, to),
                Action::CreateFolder { parent, name } => {
                    let path = parent.join(&name);
                    self.create_dir(path);
                }
                Action::Paste { dest } => self.paste_files(dest),
                Action::Move { paths, dest } => {
                    self.commit_spring_navigation();
                    self.move_files(paths, dest);
                }
                Action::CopyTo { paths, dest } => {
                    self.commit_spring_navigation();
                    self.copy_files(paths, dest);
                }
                Action::Undo => self.undo(),
                Action::Redo => self.redo(),
                Action::OpenBatchRename => {
                    let tile = self.state.focused_tile;
                    let paths: Vec<PathBuf> = self
                        .state
                        .tab_states
                        .get(&tile)
                        .map(|ts| {
                            ts.selected
                                .iter()
                                .filter_map(|&i| ts.entries.get(i).map(|e| e.path.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !paths.is_empty() {
                        self.state.batch_rename =
                            Some(crate::ui::batch_rename::BatchRenameState::new(paths));
                    }
                }
                Action::SplitHorizontal(pane_id) => self.split_pane(pane_id, false),
                Action::SplitVertical(pane_id) => self.split_pane(pane_id, true),
                Action::SplitHorizontalFocused => {
                    let tabs_id = self
                        .state
                        .find_tabs_container(self.state.focused_tile)
                        .or_else(|| self.state.find_any_tabs_container())
                        .unwrap_or(self.state.focused_tile);
                    self.split_pane(tabs_id, false);
                }
                Action::SplitVerticalFocused => {
                    let tabs_id = self
                        .state
                        .find_tabs_container(self.state.focused_tile)
                        .or_else(|| self.state.find_any_tabs_container())
                        .unwrap_or(self.state.focused_tile);
                    self.split_pane(tabs_id, true);
                }
                Action::ClosePane(pane_id) => self.close_pane(pane_id),
                Action::CloseFocusedPane => {
                    let tabs_id = self
                        .state
                        .find_tabs_container(self.state.focused_tile)
                        .or_else(|| self.state.find_any_tabs_container());
                    if let Some(id) = tabs_id {
                        self.close_pane(id);
                    }
                }
                Action::InvertSelection => {
                    if let Some(ts) = self.state.active_tab_state_mut() {
                        let n = ts.entries.len();
                        let all: std::collections::HashSet<usize> = (0..n).collect();
                        ts.selected = all.difference(&ts.selected).copied().collect();
                    }
                }
                Action::FocusNextPane => {
                    let panes = self.state.all_pane_tile_ids();
                    if panes.len() > 1 {
                        let cur = self.state.focused_tile;
                        let next = panes
                            .iter()
                            .skip_while(|&&id| id != cur)
                            .nth(1)
                            .copied()
                            .unwrap_or(panes[0]);
                        self.state.focused_tile = next;
                    }
                }
                Action::SetViewMode(mode) => {
                    if let Some(tab) = self.state.active_tab_data_mut() {
                        tab.view_mode = mode;
                    }
                }
                Action::SwitchProfile(idx) => self.switch_profile(idx),
                Action::DeleteActiveProfile => self.delete_active_profile(),
            }
        }
    }

    fn switch_profile(&mut self, idx: usize) {
        if idx == self.state.profiles.active_idx || idx >= self.state.profiles.profiles.len() {
            return;
        }

        // Snapshot current tree into the active profile before leaving
        self.state.profiles.active_mut().tree = Some(self.state.pane_tree.clone());

        // Drop all FS watchers from the current session
        self.state.watchers.clear();

        self.state.profiles.active_idx = idx;
        self.state.profile_ui.adding_bookmark = false;
        self.state.profile_ui.adding_tag = false;

        // Restore the target profile's tree, or create a fresh single-tab home pane
        let new_tree = self
            .state
            .profiles
            .active()
            .tree
            .clone()
            .unwrap_or_else(|| {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                let tab_data = TabData::with_view(
                    home,
                    self.state.settings.view_mode.clone(),
                    self.state.settings.thumbnail_size,
                    self.state.settings.show_inspector,
                );
                let mut tree = egui_tiles::Tree::empty(egui::Id::new("Atlas"));
                let pane_id = tree.tiles.insert_pane(tab_data);
                let tabs_id = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(tabs_id);
                tree
            });

        self.state.pane_tree = new_tree;
        self.state.tab_states.clear();
        self.state.preview_paths.clear();
        self.state.preview_results.clear();
        self.state.preview_textures.clear();
        self.pending_preview = None;

        let pane_ids = self.state.all_pane_tile_ids();
        if let Some(&first) = pane_ids.first() {
            self.state.focused_tile = first;
        }

        // Kick off directory loads for every pane in the restored tree
        let show_hidden = self.state.settings.show_hidden;
        for tile_id in pane_ids {
            let (path, sort_col, sort_dir) = match self.state.pane_tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(p)) => {
                    (p.path.clone(), p.sort_col.clone(), p.sort_dir.clone())
                }
                _ => continue,
            };
            let op_id = self.state.next_op_id();
            let ts = self.state.tab_states.entry(tile_id).or_default();
            ts.load_state = crate::state::tab::LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = self.tx.clone();
            self.rt.spawn(async move {
                match crate::fs::read_dir::list_dir(&path, show_hidden, sort_col, sort_dir).await {
                    Ok(entries) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }
    }

    fn delete_active_profile(&mut self) {
        if self.state.profiles.profiles.len() <= 1 {
            return;
        }
        let del_idx = self.state.profiles.active_idx;
        self.state.profiles.profiles.remove(del_idx);
        let new_idx = if del_idx >= self.state.profiles.profiles.len() {
            self.state.profiles.profiles.len() - 1
        } else {
            del_idx
        };
        // active_idx must differ from new_idx to trigger a real switch,
        // so temporarily place it out of range before calling switch_profile.
        self.state.profiles.active_idx = usize::MAX;
        self.switch_profile(new_idx);
    }

    fn split_pane(&mut self, tabs_container_id: TileId, vertical: bool) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let tab_data = TabData::with_view(
            home.clone(),
            self.state.settings.view_mode.clone(),
            self.state.settings.thumbnail_size,
            self.state.settings.show_inspector,
        );
        let new_pane_id = self.state.pane_tree.tiles.insert_pane(tab_data);
        self.state
            .tab_states
            .insert(new_pane_id, TabState::default());
        let new_tabs_id = self
            .state
            .pane_tree
            .tiles
            .insert_tab_tile(vec![new_pane_id]);

        if insert_split_container(
            &mut self.state.pane_tree,
            tabs_container_id,
            new_tabs_id,
            vertical,
        )
        .is_none()
        {
            self.state.pane_tree.tiles.remove(new_tabs_id);
            self.state.pane_tree.tiles.remove(new_pane_id);
            self.state.tab_states.remove(&new_pane_id);
            return;
        }

        self.state.focused_tile = new_pane_id;
        self.navigate(new_pane_id, home);
    }

    fn close_pane(&mut self, tabs_container_id: TileId) {
        if !matches!(
            self.state.pane_tree.tiles.get(tabs_container_id),
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_)))
        ) {
            return;
        }
        let pane_ids = pane_descendants(&self.state.pane_tree, tabs_container_id);
        if self.state.all_pane_tile_ids().len() <= pane_ids.len() {
            return;
        }

        for child_id in pane_ids {
            self.remove_tab_runtime(child_id);
        }
        self.state.pane_tree.remove_recursively(tabs_container_id);
        if !self.state.tab_states.contains_key(&self.state.focused_tile) {
            if let Some(first) = self.state.all_pane_tile_ids().first().copied() {
                self.activate_tab(first);
            }
        }
    }

    fn navigate(&mut self, tab_id: TileId, path: PathBuf) {
        if !path.is_dir() {
            return;
        }
        // Validate tab_id; fall back to focused_tile, then first valid pane
        let tab_id = if matches!(
            self.state.pane_tree.tiles.get(tab_id),
            Some(egui_tiles::Tile::Pane(_))
        ) {
            tab_id
        } else {
            let focused = self.state.focused_tile;
            if matches!(
                self.state.pane_tree.tiles.get(focused),
                Some(egui_tiles::Tile::Pane(_))
            ) {
                focused
            } else {
                match self.state.all_pane_tile_ids().first() {
                    Some(&id) => id,
                    None => return,
                }
            }
        };

        let (col, dir) = if let Some(td) = self.tab_data_for(tab_id) {
            (td.sort_col.clone(), td.sort_dir.clone())
        } else {
            (Default::default(), Default::default())
        };

        // Update history and path in TabData
        if let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
            if pane.path != path {
                pane.history.push(pane.path.clone());
                pane.history_forward.clear();
                pane.path = path.clone();
            }
        }

        // Start filesystem watcher for the new directory
        if let Ok(handle) = crate::fs::watcher::watch_path(&path, self.tx.clone()) {
            self.state.watchers.insert(tab_id, handle);
        }
        if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
            tab_state.begin_navigation();
        }

        // Update recent folders
        let recent = &mut self.state.settings.recent_folders;
        recent.retain(|p| p != &path);
        recent.push(path.clone());
        if recent.len() > 50 {
            recent.remove(0);
        }

        self.spawn_list_dir(tab_id, path, col, dir);
        self.state.focused_tile = tab_id;
    }

    fn navigate_back(&mut self, tab_id: TileId) {
        if let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
            if let Some(prev) = pane.history.pop() {
                pane.history_forward.push(pane.path.clone());
                let path = prev.clone();
                let col = pane.sort_col.clone();
                let dir = pane.sort_dir.clone();
                pane.path = path.clone();
                if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
                    tab_state.begin_navigation();
                }
                self.spawn_list_dir(tab_id, path, col, dir);
            }
        }
    }

    fn spring_navigate(&mut self, tab_id: TileId, path: PathBuf) {
        if !path.is_dir() {
            return;
        }
        let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) else {
            return;
        };
        if pane.path == path {
            return;
        }
        self.state
            .drag_hover
            .original_paths
            .entry(tab_id)
            .or_insert_with(|| pane.path.clone());
        let col = pane.sort_col.clone();
        let dir = pane.sort_dir.clone();
        pane.path = path.clone();
        if let Ok(handle) = crate::fs::watcher::watch_path(&path, self.tx.clone()) {
            self.state.watchers.insert(tab_id, handle);
        }
        if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
            tab_state.begin_navigation();
        }
        self.spawn_list_dir(tab_id, path, col, dir);
        self.state.focused_tile = tab_id;
    }

    fn commit_spring_navigation(&mut self) {
        self.state.drag_hover.original_paths.clear();
        self.state.drag_hover.original_focused_tile = None;
        self.state.drag_hover.candidate = None;
        self.state.drag_hover.observed_target = None;
        self.state.drag_hover.confirmation_target = None;
        self.state.drag_hover.triggered = false;
    }

    fn cancel_spring_navigation(&mut self) {
        let original_paths = std::mem::take(&mut self.state.drag_hover.original_paths);
        for (tab_id, path) in original_paths {
            let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id)
            else {
                continue;
            };
            let col = pane.sort_col.clone();
            let dir = pane.sort_dir.clone();
            pane.path = path.clone();
            if let Ok(handle) = crate::fs::watcher::watch_path(&path, self.tx.clone()) {
                self.state.watchers.insert(tab_id, handle);
            }
            if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
                tab_state.begin_navigation();
            }
            self.spawn_list_dir(tab_id, path, col, dir);
        }
        if let Some(tab_id) = self.state.drag_hover.original_focused_tile.take() {
            self.activate_tab(tab_id);
        }
        self.state.drag_hover.candidate = None;
        self.state.drag_hover.observed_target = None;
        self.state.drag_hover.confirmation_target = None;
        self.state.drag_hover.triggered = false;
    }

    fn tick_drag_hover(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        const HOVER_DELAY: f64 = 0.65;
        const CONFIRMATION_DURATION: f64 = 0.32;

        let external_drag_active = ctx.input(|input| {
            !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
        });
        if self.state.dragged_files.is_none() && !external_drag_active {
            if !self.state.drag_hover.original_paths.is_empty()
                || self.state.drag_hover.original_focused_tile.is_some()
            {
                self.cancel_spring_navigation();
            }
            self.state.drag_hover.candidate = None;
            self.state.drag_hover.observed_target = None;
            self.state.drag_hover.confirmation_target = None;
            self.state.drag_hover.triggered = false;
            return;
        }
        let observed = self.state.drag_hover.observed_target.take();
        if observed != self.state.drag_hover.candidate {
            self.state.drag_hover.candidate = observed;
            self.state.drag_hover.candidate_since = ctx.input(|input| input.time);
            self.state.drag_hover.confirmation_target = None;
            self.state.drag_hover.triggered = false;
        }
        let Some(target) = self.state.drag_hover.candidate.clone() else {
            return;
        };
        let elapsed = ctx.input(|input| input.time) - self.state.drag_hover.candidate_since;
        if elapsed < HOVER_DELAY {
            ctx.request_repaint_after(Duration::from_secs_f64(HOVER_DELAY - elapsed));
            return;
        }
        if self.state.drag_hover.triggered {
            return;
        }
        let now = ctx.input(|input| input.time);
        if self.state.drag_hover.confirmation_target.as_ref() != Some(&target) {
            self.state.drag_hover.confirmation_target = Some(target.clone());
            self.state.drag_hover.confirmation_started_at = now;
        }
        let confirmation_elapsed = now - self.state.drag_hover.confirmation_started_at;
        if confirmation_elapsed < CONFIRMATION_DURATION {
            ctx.request_repaint();
            return;
        }
        self.state.drag_hover.confirmation_target = None;
        self.state.drag_hover.triggered = true;
        match target {
            DragHoverTarget::Folder { tab, path } | DragHoverTarget::Location { tab, path } => {
                actions.push(Action::SpringNavigate { tab, path });
            }
            DragHoverTarget::Tab(tab) => actions.push(Action::SpringFocusTab(tab)),
        }
    }

    fn navigate_forward(&mut self, tab_id: TileId) {
        if let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
            if let Some(next) = pane.history_forward.pop() {
                pane.history.push(pane.path.clone());
                let path = next.clone();
                let col = pane.sort_col.clone();
                let dir = pane.sort_dir.clone();
                pane.path = path.clone();
                if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
                    tab_state.begin_navigation();
                }
                self.spawn_list_dir(tab_id, path, col, dir);
            }
        }
    }

    fn navigate_up(&mut self, tab_id: TileId) {
        if let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
            if let Some(parent) = pane.path.parent().map(|p| p.to_path_buf()) {
                if parent != pane.path {
                    pane.history.push(pane.path.clone());
                    pane.history_forward.clear();
                    let path = parent.clone();
                    let col = pane.sort_col.clone();
                    let dir = pane.sort_dir.clone();
                    pane.path = path.clone();
                    if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
                        tab_state.begin_navigation();
                    }
                    self.spawn_list_dir(tab_id, path, col, dir);
                }
            }
        }
    }

    fn new_tab(&mut self, pane_id: TileId) {
        // Resolve to a Tabs container (the caller might pass a Pane tile by mistake)
        let tabs_id = if matches!(
            self.state.pane_tree.tiles.get(pane_id),
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_)))
        ) {
            pane_id
        } else {
            match self
                .state
                .find_tabs_container(pane_id)
                .or_else(|| self.state.find_any_tabs_container())
            {
                Some(id) => id,
                None => match self.wrap_pane_in_tabs(pane_id) {
                    Some(id) => id,
                    None => return,
                },
            }
        };

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let tab_data = TabData::with_view(
            home.clone(),
            self.state.settings.view_mode.clone(),
            self.state.settings.thumbnail_size,
            self.state.settings.show_inspector,
        );
        let new_tile_id = self.state.pane_tree.tiles.insert_pane(tab_data);
        self.state
            .tab_states
            .insert(new_tile_id, TabState::default());

        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get_mut(tabs_id)
        {
            tabs.add_child(new_tile_id);
            tabs.set_active(new_tile_id);
        }

        self.activate_tab(new_tile_id);
        self.navigate(new_tile_id, home);
    }

    fn wrap_pane_in_tabs(&mut self, pane_id: TileId) -> Option<TileId> {
        if !matches!(
            self.state.pane_tree.tiles.get(pane_id),
            Some(egui_tiles::Tile::Pane(_))
        ) {
            return None;
        }

        let is_root = self.state.pane_tree.root == Some(pane_id);
        let parent_id = if is_root {
            None
        } else {
            self.state.find_parent_of(pane_id)
        };
        let tabs_id = self.state.pane_tree.tiles.insert_tab_tile(vec![pane_id]);
        if is_root {
            self.state.pane_tree.root = Some(tabs_id);
        } else if let Some(parent_id) = parent_id {
            if let Some(egui_tiles::Tile::Container(container)) =
                self.state.pane_tree.tiles.get_mut(parent_id)
            {
                match container {
                    egui_tiles::Container::Linear(linear) => {
                        if let Some(slot) = linear.children.iter_mut().find(|id| **id == pane_id) {
                            *slot = tabs_id;
                        }
                    }
                    egui_tiles::Container::Tabs(tabs) => {
                        if let Some(slot) = tabs.children.iter_mut().find(|id| **id == pane_id) {
                            *slot = tabs_id;
                        }
                    }
                    _ => return None,
                }
            }
        } else {
            return None;
        }
        Some(tabs_id)
    }

    fn activate_tab(&mut self, tab_id: TileId) {
        if !self.state.tab_states.contains_key(&tab_id) {
            return;
        }
        self.state
            .pane_tree
            .make_active(|tile_id, _| tile_id == tab_id);
        self.state.focused_tile = tab_id;
    }

    fn close_tab(&mut self, tab_id: TileId) {
        // Never close the very last pane — keep at least one
        if self.state.all_pane_tile_ids().len() <= 1 {
            return;
        }

        self.remove_tab_runtime(tab_id);

        // Find the Tabs container and remove this tile from it
        let container_id = self.state.find_tabs_container(tab_id);
        if let Some(cid) = container_id {
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
                self.state.pane_tree.tiles.get_mut(cid)
            {
                tabs.children.retain(|&id| id != tab_id);
                if tabs.active == Some(tab_id) {
                    tabs.active = tabs.children.last().copied();
                }
                if let Some(new_active) = tabs.active {
                    self.state.focused_tile = new_active;
                }
            }
        }

        self.state.pane_tree.tiles.remove(tab_id);

        // Always guarantee focused_tile points to a live pane
        if !self.state.tab_states.contains_key(&self.state.focused_tile) {
            if let Some(first) = self.state.all_pane_tile_ids().first().copied() {
                self.state.focused_tile = first;
            }
        }
    }

    fn remove_tab_runtime(&mut self, tab_id: TileId) {
        self.state.tab_states.remove(&tab_id);
        self.state.preview_paths.remove(&tab_id);
        self.state.preview_results.remove(&tab_id);
        self.state.preview_textures.remove(&tab_id);
        self.state.watchers.remove(&tab_id);
        if self
            .pending_preview
            .as_ref()
            .is_some_and(|preview| preview.tab == tab_id)
        {
            self.pending_preview = None;
        }
    }

    fn spawn_list_dir(
        &mut self,
        tab_id: TileId,
        path: PathBuf,
        sort_col: crate::state::file_entry::SortColumn,
        sort_dir: crate::state::file_entry::SortDir,
    ) {
        let op_id = self.state.next_op_id();
        let ts = self.state.tab_states.entry(tab_id).or_default();
        ts.load_state = LoadState::Loading;
        ts.last_nav_op_id = op_id;

        let tx = self.tx.clone();
        let show_hidden = self.state.settings.show_hidden;
        self.rt.spawn(async move {
            match list_dir(&path, show_hidden, sort_col, sort_dir).await {
                Ok(entries) => {
                    let _ = tx.send(BgResult::DirListed {
                        tab: tab_id,
                        op_id,
                        entries,
                    });
                }
                Err(e) => {
                    let _ = tx.send(BgResult::DirListError {
                        tab: tab_id,
                        op_id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    fn affected_tabs_for_path(&self, path: &PathBuf) -> Vec<TileId> {
        self.state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                if let egui_tiles::Tile::Pane(p) = tile {
                    if path.starts_with(&p.path) || p.path.starts_with(path) {
                        return Some(*id);
                    }
                }
                None
            })
            .collect()
    }

    fn delete_files(&mut self, paths: Vec<PathBuf>, to_trash: bool) {
        use crate::state::undo::UndoEntry;
        let op_id = self.state.next_op_id();
        let affected = paths
            .iter()
            .flat_map(|p| self.affected_tabs_for_path(p))
            .collect();
        self.state.active_ops.insert(
            op_id,
            crate::state::app_state::OpState {
                done: 0,
                total: paths.len() as u64,
                current_file: PathBuf::new(),
            },
        );

        if to_trash {
            for path in &paths {
                self.state.undo_stack.push(UndoEntry::Delete {
                    original: path.clone(),
                    trash_path: PathBuf::new(), // trash path not easily known beforehand
                });
            }
        }

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::delete_files(paths, to_trash, op_id, tx, affected).await;
        });
    }

    fn rename_file(&mut self, from: PathBuf, to: PathBuf) {
        use crate::state::undo::UndoEntry;
        self.state.undo_stack.push(UndoEntry::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&from);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::rename_file(from, to, op_id, tx, affected).await;
        });
    }

    fn create_dir(&mut self, path: PathBuf) {
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&path);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::create_dir(path, op_id, tx, affected).await;
        });
    }

    fn paste_files(&mut self, dest: PathBuf) {
        let (paths, is_cut) = match self.state.clipboard.as_ref() {
            Some(c) => (c.paths.clone(), c.is_cut),
            None => return,
        };
        if is_cut {
            self.state.clipboard = None;
            self.move_files(paths, dest);
            return;
        }
        self.copy_files(paths, dest);
    }

    fn copy_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| {
                path.parent() != Some(dest.as_path()) && path != &dest && !dest.starts_with(path)
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&dest);
        self.state.active_ops.insert(
            op_id,
            crate::state::app_state::OpState {
                done: 0,
                total: paths.len() as u64,
                current_file: PathBuf::new(),
            },
        );
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::copy_files(paths, dest, op_id, tx, affected).await;
        });
    }

    fn move_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| {
                path.parent() != Some(dest.as_path()) && path != &dest && !dest.starts_with(path)
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        self.state
            .undo_stack
            .push(crate::state::undo::UndoEntry::Move {
                paths: paths.clone(),
                dest: dest.clone(),
            });
        self.spawn_move_files(paths, dest);
    }

    fn spawn_move_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        let op_id = self.state.next_op_id();
        let affected: std::collections::HashSet<_> = paths
            .iter()
            .flat_map(|path| self.affected_tabs_for_path(path))
            .chain(self.affected_tabs_for_path(&dest))
            .collect();
        self.state.active_ops.insert(
            op_id,
            crate::state::app_state::OpState {
                done: 0,
                total: paths.len() as u64,
                current_file: PathBuf::new(),
            },
        );
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::move_files(paths, dest, op_id, tx, affected.into_iter().collect())
                .await;
        });
    }

    fn undo(&mut self) {
        use crate::state::undo::UndoEntry;
        if let Some(entry) = self.state.undo_stack.undo.pop() {
            match &entry {
                UndoEntry::Rename { from, to } => {
                    let from = from.clone();
                    let to = to.clone();
                    let op_id = self.state.next_op_id();
                    let affected = self.affected_tabs_for_path(&to);
                    let tx = self.tx.clone();
                    // Reverse: rename 'to' back to 'from'
                    self.rt.spawn(async move {
                        crate::fs::ops::rename_file(to, from, op_id, tx, affected).await;
                    });
                }
                UndoEntry::Move { paths, dest } => {
                    if let Some(original_parent) = paths.first().and_then(|path| path.parent()) {
                        let moved_paths = paths
                            .iter()
                            .filter_map(|path| path.file_name().map(|name| dest.join(name)))
                            .collect();
                        self.spawn_move_files(moved_paths, original_parent.to_path_buf());
                    }
                }
                _ => {} // Delete undo via trash restore — complex, skip for now
            }
            self.state.undo_stack.redo.push(entry);
        }
    }

    fn redo(&mut self) {
        if let Some(entry) = self.state.undo_stack.redo.pop() {
            match &entry {
                crate::state::undo::UndoEntry::Move { paths, dest } => {
                    self.spawn_move_files(paths.clone(), dest.clone());
                }
                crate::state::undo::UndoEntry::Rename { from, to } => {
                    let op_id = self.state.next_op_id();
                    let affected = self.affected_tabs_for_path(from);
                    let tx = self.tx.clone();
                    let from = from.clone();
                    let to = to.clone();
                    self.rt.spawn(async move {
                        crate::fs::ops::rename_file(from, to, op_id, tx, affected).await;
                    });
                }
                _ => {}
            }
            self.state.undo_stack.undo.push(entry);
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context, tab: TileId, path: PathBuf) {
        if self.state.preview_paths.get(&tab).map(PathBuf::as_path) == Some(path.as_path()) {
            return; // already loaded
        }
        self.state.preview_paths.insert(tab, path.clone());
        self.state.preview_results.remove(&tab);
        self.state.preview_textures.remove(&tab);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            crate::fs::preview::load_preview(tab, path, tx).await;
            ctx.request_repaint();
        });
    }

    fn sync_active_preview(&mut self, ctx: &egui::Context) {
        let tab = self.state.focused_tile;
        if !self
            .state
            .active_tab_data()
            .is_some_and(|data| data.preview_enabled)
        {
            self.pending_preview = None;
            return;
        }
        let selected_path = self
            .state
            .tab_states
            .get(&tab)
            .and_then(|ts| ts.selected.iter().next())
            .and_then(|&idx| self.state.tab_states.get(&tab)?.entries.get(idx))
            .map(|entry| entry.path.clone());
        if let Some(path) = selected_path {
            if self.state.preview_paths.get(&tab) == Some(&path) {
                self.pending_preview = None;
                return;
            }

            let now = Instant::now();
            let is_same_pending = self
                .pending_preview
                .as_ref()
                .is_some_and(|pending| pending.tab == tab && pending.path == path);
            if !is_same_pending {
                self.state.preview_paths.remove(&tab);
                self.state.preview_results.remove(&tab);
                self.state.preview_textures.remove(&tab);
                self.pending_preview = Some(PendingPreview {
                    tab,
                    path,
                    ready_at: now + PREVIEW_DEBOUNCE,
                });
            }

            let pending = self
                .pending_preview
                .as_ref()
                .expect("pending preview was just created");
            if pending.ready_at <= now {
                let pending = self
                    .pending_preview
                    .take()
                    .expect("pending preview still exists");
                self.load_preview(ctx, pending.tab, pending.path);
            } else {
                ctx.request_repaint_after(pending.ready_at - now);
            }
        } else {
            self.pending_preview = None;
            self.state.preview_paths.remove(&tab);
            self.state.preview_results.remove(&tab);
            self.state.preview_textures.remove(&tab);
        }
    }

    fn spawn_open_files(&self, paths: Vec<PathBuf>) {
        self.rt.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                for path in paths {
                    let _ = open::that(path);
                }
            })
            .await;
        });
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
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
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

        // Tick nucleo and update results when goto bar is open
        if self.state.show_goto_bar {
            self.state.nucleo.set_pattern(&self.state.goto_state.query);
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
        crate::ui::file_list::handle_pointer_events(
            ctx,
            &mut self.state.tab_states,
            &mut pointer_actions,
        );
        self.process_actions(pointer_actions);

        let mut actions = Vec::new();
        handle_global_shortcuts(ctx, &self.state, &mut actions);

        show_layout(ctx, &mut self.state, &mut actions);
        self.tick_drag_hover(ctx, &mut actions);
        if self.state.dragged_files.is_some()
            && ctx.input(|input| input.pointer.any_down() && !input.pointer.has_pointer())
        {
            let paths = self
                .state
                .dragged_files
                .as_ref()
                .map(|drag| drag.paths.clone())
                .unwrap_or_default();
            if crate::util::native_drag::start(frame, paths) {
                self.state.dragged_files = None;
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
            let target = self.state.external_drop_target.take().or_else(|| {
                self.state.active_tab_data().map(|pane| {
                    crate::state::app_state::ExternalDropTarget {
                        tab: self.state.focused_tile,
                        dest: pane.path.clone(),
                    }
                })
            });
            if let Some(target) = target {
                self.state.focused_tile = target.tab;
                actions.push(Action::CopyTo {
                    paths: external_paths,
                    dest: target.dest,
                });
            }
        } else if ctx.input(|input| input.raw.hovered_files.is_empty()) {
            self.state.external_drop_target = None;
        }

        let pointer_released = ctx.input(|input| input.pointer.any_released());
        let internal_drop = actions
            .iter()
            .any(|action| matches!(action, Action::Move { .. }));
        let had_internal_drag = self.state.dragged_files.is_some();
        if pointer_released {
            self.state.dragged_files = None;
        }

        let had_actions = !actions.is_empty();
        self.process_actions(actions);
        let mut shelf_actions = Vec::new();
        crate::ui::drop_shelf::show_drop_shelf(
            ctx,
            &mut self.state.drop_shelf_open,
            &mut self.state.drop_shelf_files,
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

fn insert_split_container(
    tree: &mut egui_tiles::Tree<TabData>,
    tabs_container_id: TileId,
    new_tabs_id: TileId,
    vertical: bool,
) -> Option<TileId> {
    let is_root = tree.root == Some(tabs_container_id);
    let parent_id = (!is_root)
        .then(|| tree.tiles.parent_of(tabs_container_id))
        .flatten();
    let valid_target = matches!(
        tree.tiles.get(tabs_container_id),
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_)))
    ) && (is_root
        || parent_id.is_some_and(|id| {
            matches!(
                tree.tiles.get(id),
                Some(egui_tiles::Tile::Container(
                    egui_tiles::Container::Linear(_) | egui_tiles::Container::Tabs(_)
                ))
            )
        }));
    if !valid_target {
        return None;
    }

    let split_id = if vertical {
        tree.tiles
            .insert_vertical_tile(vec![tabs_container_id, new_tabs_id])
    } else {
        tree.tiles
            .insert_horizontal_tile(vec![tabs_container_id, new_tabs_id])
    };
    if is_root {
        tree.root = Some(split_id);
    } else if let Some(egui_tiles::Tile::Container(container)) = tree
        .tiles
        .get_mut(parent_id.expect("validated split parent"))
    {
        let children = match container {
            egui_tiles::Container::Linear(linear) => &mut linear.children,
            egui_tiles::Container::Tabs(tabs) => &mut tabs.children,
            _ => unreachable!("split target parent was validated"),
        };
        let slot = children
            .iter_mut()
            .find(|id| **id == tabs_container_id)
            .expect("validated split parent contains target");
        *slot = split_id;
    }
    Some(split_id)
}

fn pane_descendants(tree: &egui_tiles::Tree<TabData>, root: TileId) -> Vec<TileId> {
    let mut panes = Vec::new();
    let mut pending = vec![root];
    let mut visited = std::collections::HashSet::new();
    while let Some(tile_id) = pending.pop() {
        if !visited.insert(tile_id) {
            continue;
        }
        match tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(_)) => panes.push(tile_id),
            Some(egui_tiles::Tile::Container(container)) => {
                pending.extend(container.children().copied());
            }
            None => {}
        }
    }
    panes
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

#[cfg(test)]
mod tests {
    use super::{insert_split_container, pane_descendants};
    use crate::state::pane::TabData;
    use crate::state::settings::ViewMode;
    use std::path::PathBuf;

    fn tab(path: &str) -> TabData {
        TabData::with_view(PathBuf::from(path), ViewMode::Details, 128.0, false)
    }

    #[test]
    fn nested_split_replaces_the_original_parent_without_creating_a_cycle() {
        let mut tree = egui_tiles::Tree::empty("split_test");
        let first_pane = tree.tiles.insert_pane(tab("/first"));
        let first_tabs = tree.tiles.insert_tab_tile(vec![first_pane]);
        let second_pane = tree.tiles.insert_pane(tab("/second"));
        let second_tabs = tree.tiles.insert_tab_tile(vec![second_pane]);
        let root = tree
            .tiles
            .insert_horizontal_tile(vec![first_tabs, second_tabs]);
        tree.root = Some(root);

        let new_pane = tree.tiles.insert_pane(tab("/new"));
        let new_tabs = tree.tiles.insert_tab_tile(vec![new_pane]);
        let split =
            insert_split_container(&mut tree, first_tabs, new_tabs, true).expect("valid split");

        let egui_tiles::Tile::Container(egui_tiles::Container::Linear(root_linear)) =
            tree.tiles.get(root).expect("root")
        else {
            panic!("expected linear root");
        };
        assert_eq!(root_linear.children, vec![split, second_tabs]);

        let egui_tiles::Tile::Container(egui_tiles::Container::Linear(split_linear)) =
            tree.tiles.get(split).expect("split")
        else {
            panic!("expected linear split");
        };
        assert_eq!(split_linear.children, vec![first_tabs, new_tabs]);
        assert!(!split_linear.children.contains(&split));
    }

    #[test]
    fn pane_descendants_collect_nested_tabs() {
        let mut tree = egui_tiles::Tree::empty("descendants_test");
        let first = tree.tiles.insert_pane(tab("/first"));
        let second = tree.tiles.insert_pane(tab("/second"));
        let nested = tree.tiles.insert_tab_tile(vec![second]);
        let root = tree.tiles.insert_tab_tile(vec![first, nested]);
        tree.root = Some(root);

        let mut panes = pane_descendants(&tree, root);
        panes.sort_by_key(|id| id.0);
        assert_eq!(panes, vec![first, second]);
    }
}
