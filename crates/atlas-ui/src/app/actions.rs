use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::FileKind;
use crate::state::layout_tree::Tile;
use crate::state::settings::ViewMode;
use std::path::PathBuf;
use std::time::Instant;

impl App {
    pub(crate) fn process_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Navigate { tab, path } => self.navigate(tab, path),
                Action::NavigateBack(tab_id) => self.navigate_back(tab_id),
                Action::NavigateForward(tab_id) => self.navigate_forward(tab_id),
                Action::NavigateUp(tab_id) => self.navigate_up(tab_id),
                Action::EditLocation(tab_id) => {
                    let path = self.tab_data_for(tab_id).map(|pane| pane.location.display_path());
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
                        let loc = tab_data.location.clone();
                        let col = tab_data.sort_col.clone();
                        let dir = tab_data.sort_dir.clone();
                        self.dispatch_list(tab_id, loc, col, dir);
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
                Action::CloseOverlays => {
                    // Esc with no palette open also dismisses the focused pane's
                    // local filter (the "obvious" exit key for the filter pill).
                    let any_palette = self.state.overlay.show_goto_bar
                        || self.state.overlay.show_palette
                        || self.state.overlay.show_ext_palette;
                    if !any_palette {
                        let tile = self.state.focused_tile;
                        if self
                            .state
                            .tab_states
                            .get(&tile)
                            .is_some_and(|ts| ts.search.active)
                        {
                            self.close_search_in_place(tile);
                        }
                    }
                    // Goto dismisses with a fade-out: keep its view + input mounted and
                    // start the exit animation; `tick_palette_close` drops them when it
                    // finishes. If it was already closing, leave the in-flight fade.
                    if self.state.overlay.show_goto_bar {
                        self.state.overlay.show_goto_bar = false;
                        self.palette_closing =
                            Some((crate::ui::palette::PaletteKind::Goto, Instant::now()));
                    }
                    self.state.overlay.show_palette = false;
                    self.palette_input = None;

                    self.state.overlay.show_ext_palette = false;
                    self.ext_palette_input = None;
                    if let Some(tx) = self.state.overlay.ext_palette_reply.take() {
                        let _ = tx.send(String::new()); // Unblock WASM on Esc
                    }
                    // Keep the host mounted only for the goto's exit fade; otherwise
                    // drop it so the next open remounts fresh.
                    if self.palette_closing.is_none() {
                        self.palette_host = None;
                    }

                    // Esc also collapses an expanded image preview.
                    self.collapse_preview_expanded();
                }
                Action::SubmitExtPalette(selection) => {
                    self.state.overlay.show_ext_palette = false;
                    self.ext_palette_input = None;
                    self.palette_host = None;
                    if let Some(tx) = self.state.overlay.ext_palette_reply.take() {
                        let _ = tx.send(selection);
                    }
                }
                Action::OpenGotoBar => {
                    self.state.overlay.show_goto_bar = true;
                    self.state.overlay.show_palette = false;
                    self.state.overlay.goto_state.reset();
                    // Cancel any in-flight exit fade and start fresh.
                    self.palette_closing = None;
                    self.goto_input = None;
                    self.palette_host = None;
                    self.nucleo.reset();
                    if self.goto_prefill.is_some() {
                        // Pre-filled open (Ctrl+L): the first render scopes the
                        // index from the path in the query; skip the global build.
                        self.goto_index_root = None;
                    } else {
                        // Rebuild the global index from the current pane's directory.
                        let root = self.state.active_tab_data().and_then(|td| {
                            td.location.fs_dir().map(|p| p.to_path_buf())
                        }).unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
                        let injector = self.nucleo.injector();
                        crate::fs::index::build_index(root.clone(), injector);
                        self.goto_index_root = Some((root, false));
                    }
                }
                Action::OpenPalette => {
                    self.state.overlay.show_palette = true;
                    self.state.overlay.show_goto_bar = false;
                    self.state.overlay.palette_state.reset();
                    self.palette_input = None;
                    // Hard-dismiss the goto (no fade) when switching to the palette.
                    self.palette_closing = None;
                    self.goto_input = None;
                    self.palette_host = None;
                }
                Action::ToggleCommandPin(id) => {
                    let pinned = &mut self.state.profiles.active_mut().pinned_commands;
                    if let Some(pos) = pinned.iter().position(|p| *p == id) {
                        pinned.remove(pos);
                    } else {
                        pinned.push(id);
                    }
                    crate::util::config::save_profiles(&self.state.profiles);
                }
                Action::OpenSearchInPlace => {
                    let tile = self.state.focused_tile;
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        ts.search.active = true;
                        ts.search.focus = true;
                    }
                }
                Action::CloseSearchInPlace => {
                    let tile = self.state.focused_tile;
                    self.close_search_in_place(tile);
                }
                Action::ToggleSearchInSubfolders => {
                    let tile = self.state.focused_tile;
                    if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                        ts.search.recursive = !ts.search.recursive;
                        ts.search.status = crate::state::tab::SearchStatus::Idle;
                        ts.search.error = None;
                        ts.search.focus = true;
                    }
                    self.refresh_search_in_place(tile);
                }
                Action::ToggleFoldersFirst(tile) => {
                    let sort = match self.state.pane_tree.tiles.get_mut(tile) {
                        Some(Tile::Pane(p)) => {
                            p.folders_first = !p.folders_first;
                            Some((p.sort_col.clone(), p.sort_dir.clone(), p.folders_first))
                        }
                        _ => None,
                    };
                    if let Some((col, dir, folders_first)) = sort {
                        self.resort_entries_in_place(tile, &col, &dir, folders_first);
                    }
                }
                Action::ToggleShowFiles(tile) | Action::ToggleShowFolders(tile) => {
                    let toggled = match self.state.pane_tree.tiles.get_mut(tile) {
                        Some(Tile::Pane(p)) => {
                            if matches!(action, Action::ToggleShowFiles(_)) {
                                p.show_files = !p.show_files;
                            } else {
                                p.show_folders = !p.show_folders;
                            }
                            Some((p.show_files, p.show_folders))
                        }
                        _ => None,
                    };
                    if let Some((show_files, show_folders)) = toggled {
                        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                            ts.set_visibility(show_files, show_folders);
                            // Drop selection entries that just became invisible so
                            // keyboard/file ops can't act on hidden items.
                            let entries = &ts.entries;
                            let kind_visible = |i: usize| {
                                entries.get(i).is_some_and(|e| {
                                    if matches!(e.kind, FileKind::Dir) {
                                        show_folders
                                    } else {
                                        show_files
                                    }
                                })
                            };
                            ts.selection.selected.retain(|&i| kind_visible(i));
                            if ts.selection.keyboard_cursor.is_some_and(|i| !kind_visible(i)) {
                                ts.selection.keyboard_cursor = None;
                                ts.selection.selection_anchor = None;
                            }
                        }
                    }
                }
                Action::ClosePalette => {
                    self.state.overlay.show_palette = false;
                    self.state.overlay.show_goto_bar = false;
                    self.palette_closing = None;
                    self.goto_input = None;
                    self.palette_input = None;
                    self.palette_host = None;
                }
                Action::Refresh(tab_id) => {
                    if let Some(tab_data) = self.tab_data_for(tab_id) {
                        let loc = tab_data.location.clone();
                        let col = tab_data.sort_col.clone();
                        let dir = tab_data.sort_dir.clone();
                        self.dispatch_list(tab_id, loc, col, dir);
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
                        .map(|e| (e.path.clone(), e.kind.clone(), e.archive_root.clone()));
                    if let Some((path, kind, archive_root)) = entry {
                        if let Some(archive) = archive_root {
                            // Virtual entry inside an archive.
                            if kind == FileKind::Dir {
                                self.navigate_into_archive(tile, archive, path);
                            } else {
                                // Open virtual file by extracting to a temp dir first.
                                let dest_dir = std::env::temp_dir().join("atlas-open");
                                let dest = dest_dir.join(path.file_name().map_or_else(
                                    || rust_i18n::t!("extract_fallback_name").to_string(),
                                    |n| n.to_string_lossy().into_owned(),
                                ));
                                let rt = self.rt.clone();
                                rt.spawn(async move {
                                    let _ = tokio::task::spawn_blocking(move || {
                                        #[allow(clippy::disallowed_methods)]
                                        // reason: inside spawn_blocking
                                        let _ = std::fs::create_dir_all(&dest_dir);
                                        if let Some(p) = crate::fs::archive::open(&archive) {
                                            let _ = p.extract_to(&path, &dest);
                                        }
                                        let _ = open::that(&dest);
                                    })
                                    .await;
                                });
                            }
                        } else if kind == FileKind::Dir {
                            self.navigate(tile, path);
                        } else {
                            let is_archive = crate::fs::archive::is_archive_path(&path);
                            if is_archive {
                                self.navigate_into_archive(tile, path, std::path::PathBuf::new());
                            } else {
                                self.spawn_open_files(vec![path]);
                            }
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
                        let loc = pane.location.clone();
                        let sort_col = pane.sort_col.clone();
                        let sort_dir = pane.sort_dir.clone();
                        self.dispatch_list(self.state.focused_tile, loc, sort_col, sort_dir);
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
                    self.set_clipboard(paths, false);
                }
                Action::Cut(paths) => {
                    self.set_clipboard(paths, true);
                }
                Action::Delete { paths, to_trash } => self.delete_files(paths, to_trash),
                Action::Rename { from, to } => self.rename_file(from, to),
                Action::CreateFolder { parent, name } => {
                    let path = parent.join(&name);
                    self.create_dir(path);
                }
                Action::Paste { dest } => self.paste_files(dest),
                Action::ResolveTransferConflict(policy) => self.apply_transfer_resolution(policy),
                Action::Transfer(req) => self.request_transfer(req),
                // gpui drop handlers: `Move` carries an internal drag,
                // `CopyTo` external paths dropped onto the window.
                Action::Move { paths, dest } => {
                    self.move_files(paths, dest, crate::state::ops::TransferOrigin::DndInternal);
                }
                Action::CopyTo { paths, dest } => {
                    self.copy_files(paths, dest, crate::state::ops::TransferOrigin::DndExternal);
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
                        // Fresh native inputs for this session.
                        self.batch_pattern_input = None;
                        self.batch_replacement_input = None;
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
                Action::MillerInit { tile, location } => {
                    let loc = location;
                    let already = self
                        .state
                        .tab_states
                        .get(&tile)
                        .and_then(|ts| ts.miller.columns.first())
                        .is_some_and(|c| c.location == loc);
                    if !already {
                        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                            ts.miller = crate::state::tab::MillerState::default();
                            ts.miller
                                .columns
                                .push(crate::state::tab::MillerColumn::new(loc.clone()));
                        }
                        self.spawn_miller_list(tile, loc);
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
                            if col.selection.selected.is_empty() && !col.entries.is_empty() {
                                col.selection.selected.insert(0);
                                col.selection.keyboard_cursor = Some(0);
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
                                let loc = td.location.clone();
                                let col = td.sort_col.clone();
                                let dir = td.sort_dir.clone();
                                self.dispatch_list(tab, loc, col, dir);
                            }
                        }
                    }
                }
                Action::SetTheme(name) => self.set_theme(name),
                Action::SwitchProfile(idx) => self.switch_profile(idx),
                Action::CreateProfile(name) => self.create_profile(name),
                Action::RenameActiveProfile(name) => self.rename_active_profile(name),
                Action::OpenRenameProfile => {
                    // Re-seed the input with the current name on next render.
                    self.rename_profile_input = None;
                    self.state.overlay.renaming_profile = true;
                }
                Action::DeleteActiveProfile => self.delete_active_profile(),
                Action::ToggleSidebarSection(i) => {
                    if let Some(section) =
                        self.state.profiles.active_mut().sidebar_sections.get_mut(i)
                    {
                        section.collapsed ^= true;
                    }
                }
                Action::MoveSidebarSectionUp(i) => {
                    let sections = &mut self.state.profiles.active_mut().sidebar_sections;
                    if i > 0 && i < sections.len() {
                        sections.swap(i, i - 1);
                    }
                }
                Action::MoveSidebarSectionDown(i) => {
                    let sections = &mut self.state.profiles.active_mut().sidebar_sections;
                    if i + 1 < sections.len() {
                        sections.swap(i, i + 1);
                    }
                }
                Action::SetSidebarSectionHidden { index, hidden } => {
                    if let Some(section) = self
                        .state
                        .profiles
                        .active_mut()
                        .sidebar_sections
                        .get_mut(index)
                    {
                        section.hidden = hidden;
                    }
                }
                Action::ResetSidebarSections => {
                    self.state.profiles.active_mut().sidebar_sections =
                        crate::state::profiles::default_sections();
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
                    let _ = self.tag_db_tx.send(crate::fs::tag_db::TagDbCmd::CreateTag {
                        name,
                        color,
                        profile,
                    });
                }
                Action::OpenCreateTagOverlay { global } => {
                    self.state.profile_ui.creating_tag = true;
                    self.state.profile_ui.adding_tag = true;
                    self.state.profile_ui.new_tag_global = global;
                    self.state.profile_ui.new_tag_name.clear();
                    self.state.profile_ui.new_tag_color = crate::state::profiles::PRESET_COLORS[0];
                }
                Action::CloseCreateTagOverlay => {
                    self.state.profile_ui.creating_tag = false;
                    self.state.profile_ui.adding_tag = false;
                    self.state.profile_ui.new_tag_name.clear();
                }
                Action::DeleteTag(tag) => {
                    self.state.tag_cache.defs.retain(|d| d.id != tag);
                    for tags in self.state.tag_cache.for_path.values_mut() {
                        tags.retain(|t| *t != tag);
                    }
                    let _ = self
                        .tag_db_tx
                        .send(crate::fs::tag_db::TagDbCmd::DeleteTag(tag));
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
                Action::NavigateIntoArchive {
                    tab,
                    archive,
                    inner,
                } => {
                    self.navigate_into_archive(tab, archive, inner);
                }
                Action::ExtractArchiveTo { tab, items, dest } => {
                    self.extract_archive_items(tab, items, dest);
                }
                #[cfg(feature = "cloud")]
                Action::Cloud(c) => self.handle_cloud_action(c),
                Action::Ext(e) => self.handle_ext_action(e),
            }
        }
    }
}

impl App {
    pub(crate) fn close_search_in_place(&mut self, tile: crate::state::layout_tree::TileId) {
        self.restore_search_original_entries(tile);
        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
            ts.search.active = false;
            ts.search.focus = false;
            ts.search.query.clear();
            ts.search.status = crate::state::tab::SearchStatus::Idle;
            ts.search.error = None;
            ts.filter.clear();
            ts.visible_valid = false;
            ts.selection.clear();
        }
        self.search_inputs.remove(&tile);
    }

    pub(crate) fn refresh_search_in_place(&mut self, tile: crate::state::layout_tree::TileId) {
        let Some((query, recursive)) = self.state.tab_states.get(&tile).map(|ts| {
            (
                ts.search.query.trim().to_string(),
                ts.search.recursive && ts.search.active,
            )
        }) else {
            return;
        };

        if query.is_empty() {
            self.restore_search_original_entries(tile);
            if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                ts.filter.clear();
                ts.visible_valid = false;
                ts.search.status = crate::state::tab::SearchStatus::Idle;
                ts.search.error = None;
                ts.selection.clear();
            }
            return;
        }

        if !recursive {
            self.restore_search_original_entries(tile);
            if let Some(ts) = self.state.tab_states.get_mut(&tile) {
                ts.filter.clone_from(&query);
                ts.visible_valid = false;
                select_first_visible(ts);
                ts.search.status = crate::state::tab::SearchStatus::Ready;
                ts.search.error = None;
            }
            return;
        }

        let Some(root) = self.tab_data_for(tile).and_then(|td| td.location.fs_dir().map(|p| p.to_path_buf())) else {
            return;
        };
        let show_hidden = self.state.settings.show_hidden;
        let generation = {
            let Some(ts) = self.state.tab_states.get_mut(&tile) else {
                return;
            };
            if ts.search.original_entries.is_none() {
                ts.search.original_entries = Some(ts.entries.clone());
            }
            ts.search.generation = ts.search.generation.wrapping_add(1);
            ts.search.status = crate::state::tab::SearchStatus::Searching;
            ts.search.error = None;
            ts.filter.clear();
            ts.visible_valid = false;
            ts.search.generation
        };

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match crate::search::name::search_names_recursive(root, query, show_hidden).await {
                Ok(entries) => {
                    let _ = tx.send(crate::state::app_state::BgResult::SearchInPlaceResults {
                        tab: tile,
                        gen: generation,
                        entries,
                    });
                }
                Err(err) => {
                    let _ = tx.send(crate::state::app_state::BgResult::SearchInPlaceError {
                        tab: tile,
                        gen: generation,
                        error: err.to_string(),
                    });
                }
            }
        });
    }

    fn restore_search_original_entries(&mut self, tile: crate::state::layout_tree::TileId) {
        let Some(ts) = self.state.tab_states.get_mut(&tile) else {
            return;
        };
        if let Some(entries) = ts.search.original_entries.take() {
            ts.set_entries(entries);
        }
    }
}

fn select_first_visible(ts: &mut crate::state::tab::TabState) {
    let first = ts.visible_indices().first().copied();
    ts.selection.clear();
    if let Some(idx) = first {
        ts.selection.selected.insert(idx);
        ts.selection.keyboard_cursor = Some(idx);
        ts.selection.selection_anchor = Some(idx);
        ts.selection.last_clicked_index = Some(idx);
        ts.selection.scroll_to_selected = true;
    }
}
