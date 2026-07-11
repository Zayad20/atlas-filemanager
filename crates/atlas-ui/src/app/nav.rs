use crate::app::App;
use atlas_model::layout_tree::{Tile, TileId};
use atlas_model::location::Location;
use std::path::PathBuf;

/// Controls how `navigate_to` manipulates the history stacks.
pub(crate) enum NavKind {
    /// Push current location onto history, clear forward stack.
    Push,
    /// Caller already popped from history; push current to forward stack.
    Back,
    /// Caller already popped from forward stack; push current to history.
    Forward,
}

impl App {
    pub(crate) fn recent_folders(&self) -> Vec<PathBuf> {
        use atlas_model::profiles::RecentFoldersFeature;
        self.state
            .profiles
            .active()
            .scoped::<RecentFoldersFeature>()
    }

    fn push_recent_folder(&mut self, path: &std::path::Path) {
        use atlas_model::profiles::RecentFoldersFeature;
        let mut recent = self
            .state
            .profiles
            .active()
            .scoped::<RecentFoldersFeature>();
        recent.retain(|p| p != path);
        recent.push(path.to_path_buf());
        if recent.len() > 50 {
            recent.remove(0);
        }
        self.state
            .profiles
            .active_mut()
            .set_scoped::<RecentFoldersFeature>(&recent);
    }

    /// Single entry point for all navigation. Handles history, watcher lifecycle,
    /// recent folders, begin_navigation, focused_tile, and spawn_list.
    pub(crate) fn navigate_to(&mut self, tab_id: TileId, mut loc: Location, kind: NavKind) {
        let tab_id = match self.resolve_pane(tab_id) {
            Some(id) => id,
            None => return,
        };
        atlas_core::fs::paths::normalize_wsl_location_path(&mut loc);
        self.rubber = None;

        // Per-folder view/sort memory (F3 #14): looked up before the mutable
        // pane borrow below so it can be applied in the same pass.
        let restored_view_pref = if let Location::Directory(path) = &loc {
            use atlas_model::profiles::FolderViewFeature;
            self.state
                .profiles
                .active()
                .scoped::<FolderViewFeature>()
                .get(path)
        } else {
            None
        };

        // ── 1. History manipulation ──────────────────────────────────────────
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
            let old = pane.location.clone();
            match kind {
                NavKind::Push => {
                    if old != loc {
                        pane.history.push(old);
                        pane.history_forward.clear();
                    }
                }
                NavKind::Back => {
                    pane.history_forward.push(old);
                }
                NavKind::Forward => {
                    pane.history.push(old);
                }
            }
            pane.location = loc.clone();
            if let Some(pref) = restored_view_pref {
                pane.view_mode = pref.view_mode;
                pane.sort_col = pref.sort_col;
                pane.sort_dir = pref.sort_dir;
            }
        }

        // ── 2. Watcher lifecycle ─────────────────────────────────────────────
        // Only Directory locations get a watcher; entering an archive/tag/remote
        // drops any lingering watcher for this pane (fixes stale-watcher bug on back).
        match &loc {
            Location::Directory(path) => {
                if let Ok(handle) = atlas_core::fs::watcher::watch_path(path, self.tx.clone()) {
                    self.watchers.insert(tab_id, handle);
                }
            }
            _ => {
                self.watchers.remove(&tab_id);
            }
        }

        // ── 3. Recent folders (Directory only) ──────────────────────────────
        if let Location::Directory(path) = &loc {
            self.push_recent_folder(path);
        }

        // ── 4. Begin navigation ──────────────────────────────────────────────
        let had_entries = self
            .state
            .tab_states
            .get(&tab_id)
            .is_some_and(|ts| !ts.entries.is_empty());
        if let Some(ts) = self.state.tab_states.get_mut(&tab_id) {
            ts.begin_navigation();
        }
        // Drop the pane's filter input so its stale text can't re-apply the old
        // filter to the new directory through the bottom-bar value sync.
        self.search_inputs.remove(&tab_id);

        // ── 5. Stale drag geometry ───────────────────────────────────────────
        // Navigating mid-drag (spring-load) leaves this pane's row-level hover
        // targets and OLE zone dest pointing at the *previous* listing — a drop
        // would route to a folder that is no longer on screen. Prune the row
        // targets (the next prepaint re-registers fresh ones; meanwhile the
        // pane-bounds fallback reads the live location) and rewrite the OLE
        // zone dest in place, since the nested DoDragDrop loop may resolve a
        // drop before the next frame recommits zones.
        if self.drag_hover_maybe_active() {
            self.pending_drag_hover_targets.retain(|target| {
                !matches!(&target.id,
                    crate::ui::drag_hover::DragHoverId::Folder(pane, _) if *pane == tab_id)
            });
            self.row_rects.remove(&tab_id);
            #[cfg(windows)]
            if let Some(dnd) = &self.dnd {
                if let Ok(mut zones) = dnd.zones.lock() {
                    match loc.fs_dir() {
                        Some(dir) => zones.set_dest_for_pane(tab_id.0, dir.to_path_buf()),
                        None => zones.remove_pane(tab_id.0),
                    }
                }
            }
        }

        // ── 6. Focused tile + listing ────────────────────────────────────────
        self.state.focused_tile = tab_id;
        self.spawn_list(tab_id, loc);
        // Keep an already-painted pane mounted until BgResult atomically replaces
        // it. Repainting the empty begin_navigation state caused a visible flash.
        // New/empty panes still need their initial loading state painted.
        if !had_entries {
            self.dirty_panes.insert(tab_id);
        }
    }

    /// Persist a per-folder view/sort pref (F3 #14). No-op when `pref` is `None`
    /// (non-directory locations don't get remembered).
    pub(crate) fn save_folder_view_pref(
        &mut self,
        pref: Option<(PathBuf, atlas_model::profiles::FolderViewPref)>,
    ) {
        let Some((path, pref)) = pref else { return };
        use atlas_model::profiles::FolderViewFeature;
        let mut prefs = self.state.profiles.active().scoped::<FolderViewFeature>();
        if prefs.get(&path).as_ref() == Some(&pref) {
            return;
        }
        prefs.set(path, pref);
        self.state
            .profiles
            .active_mut()
            .set_scoped::<FolderViewFeature>(&prefs);
        atlas_core::util::config::save_profiles(&self.state.profiles);
    }

    // ── Thin public wrappers ──────────────────────────────────────────────────

    /// Navigate to a local directory (Push). Access-mode remote tabs are redirected
    /// through the cloud worker.
    pub(crate) fn navigate(&mut self, tab_id: TileId, path: PathBuf) {
        #[cfg(feature = "provider-host")]
        if let Some(Location::Remote { id, .. }) =
            self.tab_data_for(tab_id).map(|td| td.location.clone())
        {
            let rpath = path.to_string_lossy().into_owned();
            return self.navigate_remote(tab_id, id, rpath);
        }
        self.navigate_to(tab_id, Location::Directory(path), NavKind::Push);
    }

    pub(crate) fn navigate_back(&mut self, tab_id: TileId) {
        let prev = match self.state.pane_tree.tiles.get_mut(tab_id) {
            Some(Tile::Pane(pane)) => pane.history.pop(),
            _ => None,
        };
        if let Some(loc) = prev {
            self.navigate_to(tab_id, loc, NavKind::Back);
        }
    }

    pub(crate) fn navigate_forward(&mut self, tab_id: TileId) {
        let next = match self.state.pane_tree.tiles.get_mut(tab_id) {
            Some(Tile::Pane(pane)) => pane.history_forward.pop(),
            _ => None,
        };
        if let Some(loc) = next {
            self.navigate_to(tab_id, loc, NavKind::Forward);
        }
    }

    pub(crate) fn navigate_up(&mut self, tab_id: TileId) {
        let parent = self
            .tab_data_for(tab_id)
            .and_then(|td| td.location.ascend());
        if let Some(loc) = parent {
            self.navigate_to(tab_id, loc, NavKind::Push);
        }
    }

    /// Enter an archive file or navigate to a subdirectory within one already open.
    pub(crate) fn navigate_into_archive(
        &mut self,
        tab_id: TileId,
        archive: PathBuf,
        inner: PathBuf,
    ) {
        let loc = Location::Archive { archive, inner };
        self.navigate_to(tab_id, loc, NavKind::Push);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn resolve_pane(&self, tab_id: TileId) -> Option<TileId> {
        if matches!(self.state.pane_tree.tiles.get(tab_id), Some(Tile::Pane(_))) {
            return Some(tab_id);
        }
        let focused = self.state.focused_tile;
        if matches!(self.state.pane_tree.tiles.get(focused), Some(Tile::Pane(_))) {
            return Some(focused);
        }
        self.state.all_pane_tile_ids().into_iter().next()
    }

    /// Dispatch a background listing based on location type (for refresh/toggle-hidden).
    /// Does NOT manipulate history or begin_navigation; use `navigate_to` for navigation.
    pub(crate) fn dispatch_list(
        &mut self,
        tab_id: TileId,
        loc: Location,
        col: atlas_model::file_entry::SortColumn,
        dir: atlas_model::file_entry::SortDir,
    ) {
        match loc {
            Location::Directory(path) => self.spawn_list_dir(tab_id, path, col, dir),
            Location::Archive { archive, inner } => {
                self.spawn_list_archive(tab_id, archive, inner, col, dir);
            }
            Location::Tag(tag) => {
                let op_id = self.state.next_op_id();
                let ts = self.state.tab_states.entry(tab_id).or_default();
                ts.load_state = atlas_model::tab::LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = self
                    .tag_db_tx
                    .send(atlas_core::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
            }
            Location::RecycleBin => {
                self.spawn_list_recycle_bin(tab_id, col, dir);
            }
            #[cfg(feature = "provider-host")]
            Location::Remote { id, path } => {
                self.spawn_list_remote(tab_id, id, path);
            }
        }
    }
}
