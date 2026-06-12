use crate::app::App;
use crate::state::pane::{TabData, TabSource};
use crate::state::settings::ViewMode;
use crate::state::tab::{LoadState, TabState};
use crate::state::tag_cache::TagId;
use crate::state::layout_tree::{Container, Tile, TileId};
use std::collections::HashSet;
use std::path::PathBuf;

impl App {
    /// Reconcile the app's side-tables with `pane_tree` after egui_tiles may have
    /// mutated the tree itself (tab drag-to-split moves and prunes tiles during
    /// `Tree::ui`). Drops state for panes that no longer exist and guarantees
    /// `focused_tile` points at a live pane.
    pub(crate) fn reconcile_tree_state(&mut self) {
        let live: HashSet<TileId> = self.state.all_pane_tile_ids().into_iter().collect();

        // Tiles that had per-pane state but were pruned away by egui_tiles.
        let dead: Vec<TileId> = self
            .state
            .tab_states
            .keys()
            .chain(self.state.watchers.keys())
            .copied()
            .filter(|id| !live.contains(id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        for id in dead {
            self.state.tab_states.remove(&id);
            self.state.watchers.remove(&id);
            self.state.clear_preview(id);
        }

        // egui_tiles' `ensure_active` only checks tile *visibility*, not child
        // membership, so when a tab is dragged out of a container the source
        // container can keep `active` pointing at the moved-away tile (still alive
        // in another container) — leaving its pane blank. Repair any Tabs container
        // whose active tab is no longer one of its own children.
        let tabs_ids: Vec<TileId> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                matches!(
                    tile,
                    Tile::Container(Container::Tabs(_))
                )
                .then_some(*id)
            })
            .collect();
        for id in tabs_ids {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
                self.state.pane_tree.tiles.get_mut(id)
            {
                let active_is_child = tabs
                    .active
                    .is_some_and(|active| tabs.children.contains(&active));
                if !active_is_child {
                    tabs.active = tabs.children.last().copied();
                }
            }
        }

        if !live.contains(&self.state.focused_tile) {
            if let Some(first) = self.state.all_pane_tile_ids().first().copied() {
                self.state.focused_tile = first;
            }
        }
    }

    pub(crate) fn split_pane(&mut self, tabs_container_id: TileId, vertical: bool) {
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

        let split_id = if vertical {
            self.state
                .pane_tree
                .tiles
                .insert_vertical_tile(vec![tabs_container_id, new_tabs_id])
        } else {
            self.state
                .pane_tree
                .tiles
                .insert_horizontal_tile(vec![tabs_container_id, new_tabs_id])
        };

        // Replace tabs_container_id with split_id everywhere it appears
        if self.state.pane_tree.root == Some(tabs_container_id) {
            self.state.pane_tree.root = Some(split_id);
        } else if let Some(parent_id) = self.state.find_parent_of(tabs_container_id) {
            if let Some(Tile::Container(c)) =
                self.state.pane_tree.tiles.get_mut(parent_id)
            {
                let children: &mut Vec<TileId> = match c {
                    Container::Linear(l) => &mut l.children,
                    Container::Tabs(t) => &mut t.children,
                    _ => {
                        self.state.pane_tree.root = Some(split_id);
                        return;
                    }
                };
                if let Some(slot) = children.iter_mut().find(|id| **id == tabs_container_id) {
                    *slot = split_id;
                }
            }
        }

        self.state.focused_tile = new_pane_id;
        self.navigate(new_pane_id, home);
    }

    pub(crate) fn close_pane(&mut self, tabs_container_id: TileId) {
        if let Some(Tile::Container(Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get(tabs_container_id).cloned()
        {
            for child_id in tabs.children {
                self.state.tab_states.remove(&child_id);
                self.state.pane_tree.tiles.remove(child_id);
            }
        }
        self.state.pane_tree.tiles.remove(tabs_container_id);
    }

    pub(crate) fn new_tab(&mut self, pane_id: TileId) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        self.open_new_tab(pane_id, home);
    }

    pub(crate) fn new_tab_with_path(&mut self, pane_id: TileId, path: PathBuf) {
        self.open_new_tab(pane_id, path);
    }

    /// Resolve `pane_id` (a Pane leaf or a Tabs container) to the Tabs container that
    /// should receive a new tab, wrapping a lone pane if needed.
    fn resolve_tabs_container(&mut self, pane_id: TileId) -> Option<TileId> {
        if matches!(
            self.state.pane_tree.tiles.get(pane_id),
            Some(Tile::Container(Container::Tabs(_)))
        ) {
            return Some(pane_id);
        }
        self.state
            .find_tabs_container(pane_id)
            .or_else(|| self.state.find_any_tabs_container())
            .or_else(|| self.wrap_pane_in_tabs(pane_id))
    }

    /// Insert `tab_data` as a new focused tab in the Tabs container owning `pane_id`.
    /// Returns the new pane's TileId, or `None` if no container could be resolved.
    pub(crate) fn insert_tab(&mut self, pane_id: TileId, tab_data: TabData) -> Option<TileId> {
        let tabs_id = self.resolve_tabs_container(pane_id)?;
        let new_tile_id = self.state.pane_tree.tiles.insert_pane(tab_data);
        self.state
            .tab_states
            .insert(new_tile_id, TabState::default());
        if let Some(Tile::Container(Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get_mut(tabs_id)
        {
            tabs.add_child(new_tile_id);
            tabs.set_active(new_tile_id);
        }
        self.activate_tab(new_tile_id);
        Some(new_tile_id)
    }

    /// Insert a new pane into the Tabs container that owns `pane_id`, navigate it to
    /// `path`, and focus it. `pane_id` may be a Pane leaf or a Tabs container.
    fn open_new_tab(&mut self, pane_id: TileId, path: PathBuf) {
        let tab_data = TabData::with_view(
            path.clone(),
            self.state.settings.view_mode.clone(),
            self.state.settings.thumbnail_size,
            self.state.settings.show_inspector,
        );
        if let Some(new_tile_id) = self.insert_tab(pane_id, tab_data) {
            self.navigate(new_tile_id, path);
        }
    }

    /// Open a virtual tab showing every file carrying `tag`. Unlike a directory tab it
    /// never lists the filesystem — its entries are filled in asynchronously when the
    /// tag-db worker answers `QueryPathsForTag` (see `populate_tag_tabs`).
    pub(crate) fn open_tag_results(&mut self, pane_id: TileId, tag: TagId) {
        // Miller assumes a navigable directory tree; tag results are a flat set.
        let view_mode = match self.state.settings.view_mode.clone() {
            ViewMode::Miller => ViewMode::Details,
            other => other,
        };
        let mut tab_data = TabData::with_view(
            PathBuf::from(format!("tag:{}", tag.0)),
            view_mode,
            self.state.settings.thumbnail_size,
            self.state.settings.show_inspector,
        );
        tab_data.source = TabSource::Tag(tag);
        if let Some(new_tile_id) = self.insert_tab(pane_id, tab_data) {
            if let Some(ts) = self.state.tab_states.get_mut(&new_tile_id) {
                ts.load_state = LoadState::Loading;
            }
            let _ = self
                .tag_db_tx
                .send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
        }
    }

    pub(crate) fn wrap_pane_in_tabs(&mut self, pane_id: TileId) -> Option<TileId> {
        if !matches!(
            self.state.pane_tree.tiles.get(pane_id),
            Some(Tile::Pane(_))
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
            if let Some(Tile::Container(container)) =
                self.state.pane_tree.tiles.get_mut(parent_id)
            {
                match container {
                    Container::Linear(linear) => {
                        if let Some(slot) = linear.children.iter_mut().find(|id| **id == pane_id) {
                            *slot = tabs_id;
                        }
                    }
                    Container::Tabs(tabs) => {
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

    pub(crate) fn activate_tab(&mut self, tab_id: TileId) {
        if !self.state.tab_states.contains_key(&tab_id) {
            return;
        }
        self.state
            .pane_tree
            .make_active(|tile_id, _| tile_id == tab_id);
        self.state.focused_tile = tab_id;
    }

    pub(crate) fn close_tab(&mut self, tab_id: TileId) {
        // Never close the very last pane — keep at least one
        if self.state.all_pane_tile_ids().len() <= 1 {
            return;
        }

        self.state.tab_states.remove(&tab_id);
        self.state.clear_preview(tab_id);

        // Find the Tabs container and remove this tile from it
        let container_id = self.state.find_tabs_container(tab_id);
        if let Some(cid) = container_id {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
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
}
