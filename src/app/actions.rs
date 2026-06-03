use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::FileKind;
use crate::state::layout_tree::Tile;
use crate::state::settings::ViewMode;
use std::path::PathBuf;

impl App {
    pub(crate) fn process_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Navigate { tab, path } => self.navigate(tab, path),
                Action::NavigateBack(tab_id) => self.navigate_back(tab_id),
                Action::NavigateForward(tab_id) => self.navigate_forward(tab_id),
                Action::NavigateUp(tab_id) => self.navigate_up(tab_id),
                Action::SpringNavigate { tab, path } => self.spring_navigate(tab, path),
                Action::SpringFocusTab(tab_id) => {
                    self.state
                        .drag_drop
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
                        tab_state.path_edit.text = path.to_string_lossy().into_owned();
                        tab_state.path_edit.editing = true;
                        tab_state.path_edit.focus = true;
                        tab_state.path_edit.error = None;
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
                Action::NewTabWithPath { pane, path } => self.new_tab_with_path(pane, path),
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
                    self.state.overlay.show_settings = true;
                }
                Action::ToggleAnalyzer => {
                    self.state.overlay.show_analyzer = !self.state.overlay.show_analyzer;
                }
                Action::ToggleDropShelf => {
                    self.state.drag_drop.drop_shelf_open = !self.state.drag_drop.drop_shelf_open;
                }
                Action::CloseOverlays => {
                    self.state.overlay.show_goto_bar = false;
                    self.state.overlay.show_palette = false;
                }
                Action::OpenGotoBar => {
                    self.state.overlay.show_goto_bar = true;
                    self.state.overlay.show_palette = false;
                    self.state.overlay.goto_state.reset();
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
                    self.state.overlay.show_palette = true;
                    self.state.overlay.show_goto_bar = false;
                    self.state.overlay.palette_state.reset();
                }
                Action::ClosePalette => {
                    self.state.overlay.show_palette = false;
                    self.state.overlay.show_goto_bar = false;
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
                        let visible: Vec<usize> = ts.visible_indices().to_vec();
                        if ctrl {
                            if ts.selection.selected.contains(&idx) {
                                ts.selection.selected.remove(&idx);
                            } else {
                                ts.selection.selected.insert(idx);
                            }
                        } else if shift {
                            let anchor = ts.selection.last_clicked_index.unwrap_or(idx);
                            ts.selection.selected.clear();
                            if let (Some(anchor_pos), Some(idx_pos)) = (
                                visible
                                    .iter()
                                    .position(|visible_idx| *visible_idx == anchor),
                                visible.iter().position(|visible_idx| *visible_idx == idx),
                            ) {
                                let lo = anchor_pos.min(idx_pos);
                                let hi = anchor_pos.max(idx_pos);
                                ts.selection
                                    .selected
                                    .extend(visible[lo..=hi].iter().copied());
                            }
                        } else {
                            ts.selection.selected.clear();
                            ts.selection.selected.insert(idx);
                        }
                        ts.selection.last_clicked_index = Some(idx);
                        ts.selection.keyboard_cursor = Some(idx);
                        ts.selection.selection_anchor = Some(idx);
                    }
                    self.state.focused_tile = tile;
                }
                Action::ClearSelection(tile) => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        ts.selection.selected.clear();
                        ts.selection.keyboard_cursor = None;
                        ts.selection.selection_anchor = None;
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
                        ts.selection.selected = ts.visible_indices().iter().copied().collect();
                        ts.selection.keyboard_cursor = ts.selection.selected.iter().next().copied();
                    }
                }
                Action::SetSort { col, dir } => {
                    if let Some(Tile::Pane(pane)) =
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
                    #[cfg(feature = "cloud")]
                    if self.open_remote_paths(&paths) {
                        continue;
                    }
                    self.spawn_open_files(paths);
                }
                Action::CopyPathToClipboard(path) => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(path.to_string_lossy().to_string());
                    }
                }
                Action::Copy(paths) => {
                    self.state.ops.clipboard = Some(crate::state::ops::ClipboardState {
                        paths,
                        is_cut: false,
                    });
                }
                Action::Cut(paths) => {
                    self.state.ops.clipboard = Some(crate::state::ops::ClipboardState {
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
                Action::ResolvePasteConflict(resolution) => self.apply_paste_resolution(resolution),
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
                            ts.selection
                                .selected
                                .iter()
                                .filter_map(|&i| ts.entries.get(i).map(|e| e.path.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !paths.is_empty() {
                        self.state.batch_rename =
                            Some(crate::state::batch_rename::BatchRenameState::new(paths));
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
                        ts.selection.selected =
                            all.difference(&ts.selection.selected).copied().collect();
                    }
                }
                Action::MillerInit { tile, path } => {
                    let already = self
                        .state
                        .tab_states
                        .get(&tile)
                        .and_then(|ts| ts.miller.columns.first())
                        .is_some_and(|c| c.path == path);
                    if !already {
                        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                            ts.miller = crate::state::tab::MillerState::default();
                            ts.miller
                                .columns
                                .push(crate::state::tab::MillerColumn::new(path.clone()));
                        }
                        self.spawn_miller_list(tile, path);
                    }
                }
                Action::MillerSelect { tile, column, idx } => {
                    self.miller_select(tile, column, idx);
                }
                Action::MillerActivate { tile, column, idx } => {
                    let info = self
                        .state
                        .tab_states
                        .get(&tile)
                        .and_then(|ts| ts.miller.columns.get(column))
                        .and_then(|c| c.entries.get(idx))
                        .map(|e| (e.path.clone(), e.kind.clone()));
                    if let Some((path, kind)) = info {
                        if kind == FileKind::Dir {
                            self.miller_select(tile, column, idx);
                        } else {
                            self.spawn_open_files(vec![path]);
                        }
                    }
                }
                Action::MillerFocusColumn { tile, column } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        if let Some(col) = ts.miller.columns.get_mut(column) {
                            if col.selected.is_none() && !col.entries.is_empty() {
                                col.selected = Some(0);
                            }
                            ts.miller.focused = column;
                            ts.selection.scroll_to_selected = true;
                        }
                    }
                    self.state.focused_tile = tile;
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
                    let tab = self.state.focused_tile;
                    if let Some(data) = self.state.active_tab_data_mut() {
                        data.view_mode = mode.clone();
                    }
                    // Leaving Miller can leave the canonical entries empty (Miller keeps
                    // its data in its own columns); reload so the other views render.
                    if mode != ViewMode::Miller {
                        let needs_load = self
                            .state
                            .tab_states
                            .get(&tab)
                            .is_some_and(|ts| ts.entries.is_empty());
                        if needs_load {
                            if let Some(td) = self.tab_data_for(tab) {
                                let path = td.path.clone();
                                let col = td.sort_col.clone();
                                let dir = td.sort_dir.clone();
                                self.spawn_list_dir(tab, path, col, dir);
                            }
                        }
                    }
                }
                Action::SwitchProfile(idx) => self.switch_profile(idx),
                Action::DeleteActiveProfile => self.delete_active_profile(),
                Action::ToggleSidebarSection(i) => {
                    self.state.profiles.active_mut().sidebar_sections[i].collapsed ^= true;
                }
                Action::MoveSidebarSectionUp(i) => {
                    self.state
                        .profiles
                        .active_mut()
                        .sidebar_sections
                        .swap(i, i - 1);
                }
                Action::MoveSidebarSectionDown(i) => {
                    self.state
                        .profiles
                        .active_mut()
                        .sidebar_sections
                        .swap(i, i + 1);
                }
                Action::RemoveBookmark(idx) => {
                    self.state.profiles.active_mut().bookmarks.remove(idx);
                }
                Action::AddBookmark { name, path } => {
                    self.state
                        .profiles
                        .active_mut()
                        .bookmarks
                        .push(crate::state::profiles::Bookmark { name, path });
                }
                Action::CreateTag {
                    name,
                    color,
                    profile,
                } => {
                    let _ = self
                        .tag_db_tx
                        .send(crate::fs::tag_db::TagDbCmd::CreateTag {
                            name,
                            color,
                            profile,
                        });
                }
                Action::DeleteTag(tag) => {
                    self.state.tag_cache.defs.retain(|d| d.id != tag);
                    for tags in self.state.tag_cache.for_path.values_mut() {
                        tags.retain(|t| *t != tag);
                    }
                    let _ = self.tag_db_tx.send(crate::fs::tag_db::TagDbCmd::DeleteTag(tag));
                }
                Action::AssignTagToFiles { tag, paths } => {
                    self.state.tag_cache.assign_local(tag, &paths);
                    let _ = self
                        .tag_db_tx
                        .send(crate::fs::tag_db::TagDbCmd::AssignTag { tag, paths });
                }
                Action::UnassignTagFromFiles { tag, paths } => {
                    self.state.tag_cache.unassign_local(tag, &paths);
                    let _ = self
                        .tag_db_tx
                        .send(crate::fs::tag_db::TagDbCmd::UnassignTag { tag, paths });
                }
                Action::OpenTagResults { pane, tag } => self.open_tag_results(pane, tag),
                Action::MountDevice(dev) => self.mount_device(dev),
                Action::UnmountDevice(dev) => self.unmount_device(dev),
                #[cfg(feature = "cloud")]
                Action::Cloud(c) => self.handle_cloud_action(c),
                Action::Ext(e) => self.handle_ext_action(e),
            }
        }
    }
}
