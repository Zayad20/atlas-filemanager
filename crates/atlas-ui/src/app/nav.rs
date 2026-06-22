use crate::app::App;
use crate::state::layout_tree::{Tile, TileId};
use crate::state::location::Location;
use std::path::PathBuf;

/// Controls how `navigate_to` manipulates the history stacks.
pub(crate) enum NavKind {
    /// Push current location onto history, clear forward stack.
    Push,
    /// Caller already popped from history; push current to forward stack.
    Back,
    /// Caller already popped from forward stack; push current to history.
    Forward,
    /// Replace current location without touching history (e.g. initial load, refresh).
    Replace,
}

impl App {
    /// Single entry point for all navigation. Handles history, watcher lifecycle,
    /// recent folders, begin_navigation, focused_tile, and spawn_list.
    pub(crate) fn navigate_to(&mut self, tab_id: TileId, loc: Location, kind: NavKind) {
        let tab_id = match self.resolve_pane(tab_id) {
            Some(id) => id,
            None => return,
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
                NavKind::Replace => {}
            }
            pane.location = loc.clone();
        }

        // ── 2. Watcher lifecycle ─────────────────────────────────────────────
        // Only Directory locations get a watcher; entering an archive/tag/remote
        // drops any lingering watcher for this pane (fixes stale-watcher bug on back).
        match &loc {
            Location::Directory(path) => {
                if let Ok(handle) = crate::fs::watcher::watch_path(path, self.tx.clone()) {
                    self.watchers.insert(tab_id, handle);
                }
            }
            _ => {
                self.watchers.remove(&tab_id);
            }
        }

        // ── 3. Recent folders (Directory only) ──────────────────────────────
        if let Location::Directory(path) = &loc {
            let recent = &mut self.state.settings.recent_folders;
            recent.retain(|p| p != path);
            recent.push(path.clone());
            if recent.len() > 50 {
                recent.remove(0);
            }
        }

        // ── 4. Begin navigation ──────────────────────────────────────────────
        if let Some(ts) = self.state.tab_states.get_mut(&tab_id) {
            ts.begin_navigation();
        }
        // Drop the pane's filter input so its stale text can't re-apply the old
        // filter to the new directory through the bottom-bar value sync.
        self.search_inputs.remove(&tab_id);

        // ── 5. Focused tile + listing ────────────────────────────────────────
        self.state.focused_tile = tab_id;
        self.spawn_list(tab_id, loc);
        // No `cx` here: mark the pane dirty so `flush_dirty_panes` (called from
        // drop handlers / the drag-hover tick) repaints the file view in lockstep
        // with the route. Without this the chrome shows the new path but the pane
        // (a separate PaneSignal entity) keeps the old listing until the async
        // BgResult arrives — the "route changes but view doesn't" desync.
        self.dirty_panes.insert(tab_id);
    }

    // ── Thin public wrappers ──────────────────────────────────────────────────

    /// Navigate to a local directory (Push). Access-mode remote tabs are redirected
    /// through the cloud worker.
    pub(crate) fn navigate(&mut self, tab_id: TileId, path: PathBuf) {
        #[cfg(feature = "cloud")]
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
        col: crate::state::file_entry::SortColumn,
        dir: crate::state::file_entry::SortDir,
    ) {
        match loc {
            Location::Directory(path) => self.spawn_list_dir(tab_id, path, col, dir),
            Location::Archive { archive, inner } => {
                self.spawn_list_archive(tab_id, archive, inner, col, dir);
            }
            Location::Tag(tag) => {
                let op_id = self.state.next_op_id();
                let ts = self.state.tab_states.entry(tab_id).or_default();
                ts.load_state = crate::state::tab::LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = self
                    .tag_db_tx
                    .send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
            }
            Location::RecycleBin => {
                self.spawn_list_recycle_bin(tab_id, col, dir);
            }
            #[cfg(feature = "cloud")]
            Location::Remote { id, path } => {
                self.spawn_list_remote(tab_id, id, path);
            }
        }
    }
}
