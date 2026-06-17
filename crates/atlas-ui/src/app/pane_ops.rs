#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::app::{App, PaneAnimation, PaneAnimationKind};
use crate::state::layout_tree::{Container, LinearDir, Tile, TileId};
use crate::state::location::Location;
use crate::state::pane::TabData;
use crate::state::settings::ViewMode;
use crate::state::tab::{LoadState, TabState};
use crate::state::tag_cache::TagId;
use std::collections::HashSet;
use std::path::PathBuf;

impl App {
    fn invalidate_dock_layout(&mut self) {
        self.dock = None;
        self.dock_revision = 0;
    }

    /// Reconcile the app's side-tables with `pane_tree` after `egui_tiles` may have
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
            .chain(self.watchers.keys())
            .copied()
            .filter(|id| !live.contains(id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        for id in dead {
            self.state.tab_states.remove(&id);
            self.watchers.remove(&id);
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
                matches!(tile, Tile::Container(Container::Tabs(_))).then_some(*id)
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

        // Inherit location + view settings from the source pane
        let source_pane_id = self
            .state
            .pane_tree
            .tiles
            .get(tabs_container_id)
            .and_then(|tile| {
                if let Tile::Container(Container::Tabs(tabs)) = tile {
                    tabs.active
                } else {
                    None
                }
            });

        let (start_path, view_mode, thumbnail_size, preview_enabled) = source_pane_id
            .and_then(|pane_id| self.state.pane_tree.tiles.get(pane_id))
            .and_then(|tile| {
                if let Tile::Pane(td) = tile {
                    Some(td)
                } else {
                    None
                }
            })
            .map(|td| {
                let path = if let Location::Directory(p) = &td.location {
                    p.clone()
                } else {
                    home.clone()
                };
                (
                    path,
                    td.view_mode.clone(),
                    td.thumbnail_size,
                    td.preview_enabled,
                )
            })
            .unwrap_or_else(|| {
                (
                    home,
                    self.state.settings.view_mode.clone(),
                    self.state.settings.thumbnail_size,
                    self.state.settings.show_inspector,
                )
            });

        let tab_data = TabData::with_view(
            start_path.clone(),
            view_mode,
            thumbnail_size,
            preview_enabled,
        );
        let new_pane_id = self.state.pane_tree.tiles.insert_pane(tab_data);
        self.state
            .tab_states
            .insert(new_pane_id, TabState::default());
        let cloned_loaded_snapshot = source_pane_id
            .and_then(|source_id| self.state.tab_states.get(&source_id))
            .and_then(|source| {
                matches!(source.load_state, LoadState::Loaded).then(|| source.entries.clone())
            })
            .is_some_and(|entries| {
                if let Some(target) = self.state.tab_states.get_mut(&new_pane_id) {
                    target.set_entries(entries);
                    target.load_state = LoadState::Loaded;
                    true
                } else {
                    false
                }
            });
        let new_tabs_id = self
            .state
            .pane_tree
            .tiles
            .insert_tab_tile(vec![new_pane_id]);

        let split_dir = if vertical {
            LinearDir::Vertical
        } else {
            LinearDir::Horizontal
        };

        let inserted_in_parent = self.state.pane_tree.insert_linear_sibling_after(
            tabs_container_id,
            new_tabs_id,
            split_dir,
        );

        if !inserted_in_parent {
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

            // Replace tabs_container_id with split_id everywhere it appears.
            if self.state.pane_tree.root == Some(tabs_container_id) {
                self.state.pane_tree.root = Some(split_id);
            } else if let Some(parent_id) = self.state.find_parent_of(tabs_container_id) {
                if let Some(Tile::Container(c)) = self.state.pane_tree.tiles.get_mut(parent_id) {
                    let children: &mut Vec<TileId> = match c {
                        Container::Linear(l) => &mut l.children,
                        Container::Tabs(t) => &mut t.children,
                    };
                    if let Some(slot) = children.iter_mut().find(|id| **id == tabs_container_id) {
                        *slot = split_id;
                    }
                }
            }
        }

        self.push_focus_history();
        self.state.focused_tile = new_pane_id;
        if !self.state.settings.disable_animations {
            self.pane_animations.insert(
                new_pane_id,
                PaneAnimation::new(if vertical {
                    PaneAnimationKind::SplitVertical
                } else {
                    PaneAnimationKind::SplitHorizontal
                }),
            );
        }
        if cloned_loaded_snapshot {
            self.state.focused_tile = new_pane_id;
        } else {
            self.navigate(new_pane_id, start_path);
        }
        self.layout_revision += 1;
    }

    pub(crate) fn close_pane(&mut self, tabs_container_id: TileId) {
        let focus_after = self.find_focus_after_close(tabs_container_id);

        if let Some(Tile::Container(Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get(tabs_container_id).cloned()
        {
            for child_id in tabs.children {
                self.file_panel_cache.remove(&child_id);
                self.state.tab_states.remove(&child_id);
                self.state.pane_tree.tiles.remove(child_id);
                self.pane_signals.remove(&child_id);
            }
        }
        self.state.pane_tree.tiles.remove(tabs_container_id);

        if let Some(focus_id) = focus_after {
            self.state.focused_tile = focus_id;
        }

        self.layout_revision += 1;
    }

    /// Push the current `focused_tile` onto the MRU history before an intentional
    /// focus change. Call this immediately before assigning `state.focused_tile`.
    pub(crate) fn push_focus_history(&mut self) {
        let cur = self.state.focused_tile;
        if self.focus_history.back() == Some(&cur) {
            return;
        }
        self.focus_history.push_back(cur);
        if self.focus_history.len() > 64 {
            self.focus_history.pop_front();
        }
    }

    /// Pop the MRU history to find the best pane to focus after closing
    /// `closing_pane_id`. Skips dead entries and the pane being closed.
    fn pop_mru_focus(&mut self, closing_pane_id: TileId) -> Option<TileId> {
        let live: HashSet<TileId> = self.state.all_pane_tile_ids().into_iter().collect();
        while let Some(id) = self.focus_history.pop_back() {
            if id != closing_pane_id && live.contains(&id) {
                return Some(id);
            }
        }
        None
    }

    /// Choose which pane should receive focus after `closing_tabs_id` is removed.
    /// First tries MRU history (VS-Code-style "previous pane"), then falls back to
    /// the layout sibling before the closed container.
    fn find_focus_after_close(&mut self, closing_tabs_id: TileId) -> Option<TileId> {
        // Collect panes in the closing container so we can skip them in history.
        let closing_panes: HashSet<TileId> = match self.state.pane_tree.tiles.get(closing_tabs_id) {
            Some(Tile::Container(Container::Tabs(t))) => t.children.iter().copied().collect(),
            _ => HashSet::new(),
        };

        // Walk MRU history backwards until a live pane outside the closing set.
        let live: HashSet<TileId> = self.state.all_pane_tile_ids().into_iter().collect();
        while let Some(&id) = self.focus_history.back() {
            if closing_panes.contains(&id) || !live.contains(&id) {
                self.focus_history.pop_back();
            } else {
                self.focus_history.pop_back();
                return Some(id);
            }
        }

        // Fallback: sibling before/after in parent Linear container.
        let parent_id = self.state.find_parent_of(closing_tabs_id)?;
        let siblings = match self.state.pane_tree.tiles.get(parent_id)? {
            Tile::Container(Container::Linear(l)) => l.children.clone(),
            _ => return None,
        };
        let pos = siblings.iter().position(|id| *id == closing_tabs_id)?;
        let target_id = if pos > 0 {
            siblings[pos - 1]
        } else {
            *siblings.get(pos + 1)?
        };
        match self.state.pane_tree.tiles.get(target_id)? {
            Tile::Container(Container::Tabs(t)) => t.active,
            Tile::Pane(_) => Some(target_id),
            _ => None,
        }
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
    /// Returns the new pane's `TileId`, or `None` if no container could be resolved.
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
        if !self.state.settings.disable_animations {
            self.pane_animations
                .insert(new_tile_id, PaneAnimation::new(PaneAnimationKind::NewTab));
        }
        self.activate_tab(new_tile_id);
        self.layout_revision += 1;
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
        tab_data.location = Location::Tag(tag);
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
        if !matches!(self.state.pane_tree.tiles.get(pane_id), Some(Tile::Pane(_))) {
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
            if let Some(Tile::Container(container)) = self.state.pane_tree.tiles.get_mut(parent_id)
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
        self.push_focus_history();
        self.state
            .pane_tree
            .make_active(|tile_id, _| tile_id == tab_id);
        self.state.focused_tile = tab_id;
        self.layout_revision += 1;
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

        self.file_panel_cache.remove(&tab_id);
        self.state.pane_tree.tiles.remove(tab_id);
        self.pane_signals.remove(&tab_id);

        // Always guarantee focused_tile points to a live pane
        if !self.state.tab_states.contains_key(&self.state.focused_tile) {
            if let Some(first) = self.state.all_pane_tile_ids().first().copied() {
                self.state.focused_tile = first;
            }
        }
        self.layout_revision += 1;
    }
}
