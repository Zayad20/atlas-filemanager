use crate::app::App;
use crate::state::layout_tree::{Tile, TileId};
use std::path::PathBuf;

impl App {
    pub(crate) fn navigate(&mut self, tab_id: TileId, path: PathBuf) {
        // Access-mode remote tabs descend by re-listing through the cloud worker,
        // not by `read_dir` — the entry path is remote-relative, not a real dir.
        #[cfg(feature = "cloud")]
        if let Some(crate::state::pane::TabSource::Remote { id, .. }) =
            self.tab_data_for(tab_id).map(|td| td.source.clone()).as_ref()
        {
            let id = id.clone();
            let rpath = path.to_string_lossy().into_owned();
            return self.navigate_remote(tab_id, id, rpath);
        }
        if !path.is_dir() {
            return;
        }
        // Validate tab_id; fall back to focused_tile, then first valid pane
        let tab_id = if matches!(
            self.state.pane_tree.tiles.get(tab_id),
            Some(Tile::Pane(_))
        ) {
            tab_id
        } else {
            let focused = self.state.focused_tile;
            if matches!(
                self.state.pane_tree.tiles.get(focused),
                Some(Tile::Pane(_))
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
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
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

    pub(crate) fn navigate_back(&mut self, tab_id: TileId) {
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
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

    pub(crate) fn spring_navigate(&mut self, tab_id: TileId, path: PathBuf) {
        self.state
            .drag_drop
            .drag_hover
            .original_paths
            .entry(tab_id)
            .or_insert_with(|| {
                if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get(tab_id) {
                    pane.path.clone()
                } else {
                    PathBuf::from("/")
                }
            });
        self.state
            .drag_drop
            .drag_hover
            .original_focused_tile
            .get_or_insert(self.state.focused_tile);
        self.navigate(tab_id, path);
    }

    pub(crate) fn navigate_forward(&mut self, tab_id: TileId) {
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
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

    pub(crate) fn navigate_up(&mut self, tab_id: TileId) {
        // A virtual tab (e.g. tag results) has no real directory to ascend from.
        if self
            .tab_data_for(tab_id)
            .is_some_and(|td| !td.source.is_directory())
        {
            return;
        }
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id) {
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
}
