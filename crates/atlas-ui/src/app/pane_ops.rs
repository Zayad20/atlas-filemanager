#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::app::{App, DockSplitAnimation, PaneAnimation, PaneAnimationKind};
use crate::state::layout_tree::{Container, LinearDir, Tile, TileId};
use crate::state::location::Location;
use crate::state::pane::TabData;
use crate::state::settings::ViewMode;
use crate::state::tab::{LoadState, TabState};
use crate::state::tag_cache::TagId;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

impl App {
    /// Reconcile the app's side-tables with `pane_tree` after `egui_tiles` may have
    /// mutated the tree itself (tab drag-to-split moves and prunes tiles during
    /// `Tree::ui`). Drops state for panes that no longer exist and guarantees
    /// `focused_tile` points at a live pane.
    pub(crate) fn reconcile_tree_state(&mut self) {
        let _perf = crate::perf::span_detail("pane_tree.reconcile", || {
            format!("tiles={}", self.state.pane_tree.tiles.iter().count())
        });
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
        let _perf = crate::perf::span_detail("split_pane", || {
            format!(
                "target={} vertical={} open={} pending={}",
                tabs_container_id.0,
                vertical,
                self.open_pane_count(),
                self.pane_closing.len()
            )
        });
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

        // Resolve the source to a concrete Pane leaf (Tabs -> active child,
        // Pane -> itself) to inherit settings from and to anchor the dock split.
        let source_pane_id = match self.state.pane_tree.tiles.get(tabs_container_id) {
            Some(Tile::Container(Container::Tabs(tabs))) => tabs.active,
            Some(Tile::Pane(_)) => Some(tabs_container_id),
            _ => None,
        };

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
                    // Synchronous load (cloned from source) → arm appear anim now.
                    target.appear_seq = target.appear_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            });
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

        let split_source = source_pane_id.unwrap_or(new_pane_id);
        self.insert_split_tile(split_source, new_pane_id, vertical);

        if cloned_loaded_snapshot {
            if !self.state.settings.disable_animations {
                self.pane_items_appear.insert(new_pane_id, Instant::now());
            }
        } else {
            self.navigate(new_pane_id, start_path);
        }
    }

    pub(crate) fn close_pane(&mut self, tabs_container_id: TileId) {
        let _perf = crate::perf::span_detail("close_pane", || {
            format!(
                "target={} open={} pending={}",
                tabs_container_id.0,
                self.open_pane_count(),
                self.pane_closing.len()
            )
        });
        let panes: Vec<TileId> = match self.state.pane_tree.tiles.get(tabs_container_id) {
            Some(Tile::Container(Container::Tabs(t))) => t.children.clone(),
            Some(Tile::Pane(_)) => vec![tabs_container_id],
            _ => return,
        };
        // Drop panes already animating out so a held shortcut can't re-queue them.
        let panes: Vec<TileId> = panes
            .into_iter()
            .filter(|p| !self.pane_closing.contains_key(p))
            .collect();
        if panes.is_empty() {
            return;
        }
        // Never close every still-open pane — keep at least one.
        let open_count = self.open_pane_count();
        if open_count <= panes.len() {
            return;
        }

        // Move focus to a survivor (live and not itself closing) before these animate
        // out, so the UI never lands on a dead/closing pane (the "lost focus" bug).
        if self.focus_needs_reassign(&panes) {
            if let Some(focus_id) = self.focus_after_closing(&panes) {
                self.state.focused_tile = focus_id;
            }
        }

        self.queue_pane_close(&panes);
    }

    /// Count panes that are open and not already animating closed.
    fn open_pane_count(&self) -> usize {
        self.state
            .all_pane_tile_ids()
            .into_iter()
            .filter(|p| !self.pane_closing.contains_key(p))
            .count()
    }

    /// True when the current focus is on a pane that's about to close (in `closing`)
    /// or already closing, so it must be moved to a survivor.
    fn focus_needs_reassign(&self, closing: &[TileId]) -> bool {
        let f = self.state.focused_tile;
        closing.contains(&f) || self.pane_closing.contains_key(&f)
    }

    /// Pick a pane to focus once `closing` is gone: a live pane that is neither in
    /// `closing` nor already animating out. Prefers MRU order, then any survivor.
    fn focus_after_closing(&self, closing: &[TileId]) -> Option<TileId> {
        let unavailable: HashSet<TileId> = closing
            .iter()
            .copied()
            .chain(self.pane_closing.keys().copied())
            .collect();
        let live: HashSet<TileId> = self
            .state
            .all_pane_tile_ids()
            .into_iter()
            .filter(|p| !unavailable.contains(p))
            .collect();
        if live.is_empty() {
            return None;
        }
        // VS-Code-style: most-recently-used survivor first.
        for id in self.focus_history.iter().rev() {
            if live.contains(id) {
                return Some(*id);
            }
        }
        self.state
            .all_pane_tile_ids()
            .into_iter()
            .find(|p| live.contains(p))
    }

    /// Defer the actual tree removal of `panes` while close animation plays.
    fn queue_pane_close(&mut self, panes: &[TileId]) {
        let _perf = crate::perf::span_detail("pane_close.queue", || {
            format!("new={} pending={}", panes.len(), self.pane_closing.len())
        });
        let mut pending_before = self.pane_closing.len();
        if self.state.settings.disable_animations {
            let mut remove: Vec<TileId> = self.pane_closing.keys().copied().collect();
            self.pane_closing.clear();
            remove.extend_from_slice(panes);
            crate::perf::event(|| {
                format!(
                    "pane_close.batch remove={} pending_before={pending_before}",
                    remove.len()
                )
            });
            self.remove_panes_from_tree(&remove);
        } else {
            if pending_before > 0 {
                let pending: Vec<TileId> = self.pane_closing.keys().copied().collect();
                self.pane_closing.clear();
                self.dock_split_animations.clear();
                crate::perf::event(|| {
                    format!(
                        "pane_close.compact_pending pending={} before_new={}",
                        pending.len(),
                        panes.len()
                    )
                });
                self.remove_panes_from_tree(&pending);
                pending_before = 0;
            }
            crate::perf::event(|| {
                format!(
                    "pane_close.defer new={} pending_before={pending_before} open_panes={}",
                    panes.len(),
                    self.open_pane_count()
                )
            });
            for &id in panes {
                self.start_pane_split_close(id);
                self.pane_closing.entry(id).or_insert_with(Instant::now);
            }
        }
    }

    fn split_shares_now(&self, linear_id: TileId, final_shares: &[f32]) -> Vec<f32> {
        self.dock_split_animations
            .get(&linear_id)
            .filter(|anim| anim.to.len() == final_shares.len() && !anim.done())
            .map(DockSplitAnimation::shares)
            .unwrap_or_else(|| final_shares.to_vec())
    }

    fn start_pane_split_close(&mut self, pane_id: TileId) {
        let Some(tabs_id) = self.state.find_tabs_container(pane_id) else {
            return;
        };
        let Some(parent_id) = self.state.find_parent_of(tabs_id) else {
            return;
        };
        let Some(Tile::Container(Container::Linear(linear))) =
            self.state.pane_tree.tiles.get(parent_id)
        else {
            return;
        };
        let old_len = linear.children.len();
        if old_len <= 1 {
            return;
        }
        let mut base = linear.shares.clone();
        if base.len() != old_len {
            base = vec![1.0; old_len];
        }
        let Some(pos) = linear.children.iter().position(|child| *child == tabs_id) else {
            return;
        };
        let from = self.split_shares_now(parent_id, &base);
        let anim = DockSplitAnimation::closing(from, pos);
        crate::perf::event(|| {
            format!(
                "pane_close.split_start pane={} tabs={} parent={} siblings={} pos={} from={:?} to={:?}",
                pane_id.0,
                tabs_id.0,
                parent_id.0,
                old_len,
                pos,
                anim.from,
                anim.to
            )
        });
        if let Some(Tile::Container(Container::Linear(linear))) =
            self.state.pane_tree.tiles.get_mut(parent_id)
        {
            linear.shares = anim.to.clone();
        }
        self.dock_split_animations.insert(parent_id, anim);
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
        self.push_focus_history();
        self.state.focused_tile = new_tile_id;
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
    }

    pub(crate) fn close_tab(&mut self, tab_id: TileId) {
        let _perf = crate::perf::span_detail("close_tab", || {
            format!(
                "tab={} open={} pending={}",
                tab_id.0,
                self.open_pane_count(),
                self.pane_closing.len()
            )
        });
        // Already animating out — a held Ctrl+W must not re-queue it.
        if self.pane_closing.contains_key(&tab_id) || self.tab_closing.contains_key(&tab_id) {
            return;
        }
        // Keep at least one open (non-closing) pane.
        if self.open_pane_count() <= 1 {
            return;
        }

        if let Some(tabs_id) = self.state.find_tabs_container(tab_id) {
            let tab_count = match self.state.pane_tree.tiles.get(tabs_id) {
                Some(Tile::Container(Container::Tabs(tabs))) => tabs.children.len(),
                _ => 0,
            };
            if tab_count > 1 {
                let next = self.focus_after_closing(&[tab_id]);
                if let Some(next) = next {
                    self.state
                        .pane_tree
                        .make_active(|tile_id, _| tile_id == next);
                    self.state.focused_tile = next;
                }
                crate::perf::event(|| {
                    format!(
                        "pane_close.close_inner_tab tab={} tabs={} count={}",
                        tab_id.0, tabs_id.0, tab_count
                    )
                });
                if self.state.settings.disable_animations {
                    self.remove_panes_from_tree(&[tab_id]);
                } else {
                    self.tab_closing.insert(tab_id, Instant::now());
                }
                return;
            }
        }

        // Hand focus to a live survivor before this tab animates away, so focus never
        // lands on a closing/dead pane (fixes lost focus on held Ctrl+W and the need
        // to re-click a pane afterwards).
        if self.focus_needs_reassign(&[tab_id]) {
            if let Some(next) = self.focus_after_closing(&[tab_id]) {
                self.state.focused_tile = next;
            }
        }

        self.queue_pane_close(&[tab_id]);
    }

    fn insert_split_tile(&mut self, source_pane: TileId, new_pane: TileId, vertical: bool) {
        let dir = if vertical {
            LinearDir::Vertical
        } else {
            LinearDir::Horizontal
        };
        let source_tabs = self
            .state
            .find_tabs_container(source_pane)
            .unwrap_or_else(|| self.wrap_pane_in_tabs(source_pane).unwrap_or(source_pane));
        let new_tabs = self.state.pane_tree.tiles.insert_tab_tile(vec![new_pane]);

        if let Some(parent_id) = self.state.find_parent_of(source_tabs) {
            let from = self
                .state
                .pane_tree
                .tiles
                .get(parent_id)
                .and_then(|tile| match tile {
                    Tile::Container(Container::Linear(linear)) if linear.dir == dir => {
                        let old_len = linear.children.len();
                        let shares = if linear.shares.len() == old_len {
                            linear.shares.clone()
                        } else {
                            vec![1.0; old_len]
                        };
                        Some(self.split_shares_now(parent_id, &shares))
                    }
                    _ => None,
                });
            if let Some(Tile::Container(Container::Linear(linear))) =
                self.state.pane_tree.tiles.get_mut(parent_id)
            {
                if linear.dir == dir {
                    if let Some(pos) = linear.children.iter().position(|id| *id == source_tabs) {
                        let old_len = linear.children.len();
                        if linear.shares.len() != old_len {
                            linear.shares = vec![1.0; old_len];
                        }
                        crate::app::normalize_split_shares(&mut linear.shares);
                        let sum: f32 = linear.shares.iter().copied().filter(|v| *v > 0.0).sum();
                        let new_share = if old_len == 0 || sum <= f32::EPSILON {
                            1.0
                        } else {
                            sum / old_len as f32
                        };
                        let insert_at = pos + 1;
                        linear.children.insert(insert_at, new_tabs);
                        linear.shares.insert(insert_at, new_share);
                        crate::app::normalize_split_shares(&mut linear.shares);
                        let anim = match from {
                            Some(from) => {
                                DockSplitAnimation::opening(from, linear.shares.clone(), insert_at)
                            }
                            None => DockSplitAnimation::new(linear.shares.clone(), insert_at),
                        };
                        self.dock_split_animations.insert(parent_id, anim);
                        return;
                    }
                }
            }
        }

        let linear = match dir {
            LinearDir::Horizontal => self
                .state
                .pane_tree
                .tiles
                .insert_horizontal_tile(vec![source_tabs, new_tabs]),
            LinearDir::Vertical => self
                .state
                .pane_tree
                .tiles
                .insert_vertical_tile(vec![source_tabs, new_tabs]),
        };
        if self.state.pane_tree.root == Some(source_tabs) {
            self.state.pane_tree.root = Some(linear);
            self.dock_split_animations
                .insert(linear, DockSplitAnimation::new(vec![1.0, 1.0], 1));
            return;
        }
        if let Some(parent_id) = self.state.find_parent_of(source_tabs) {
            if let Some(Tile::Container(container)) = self.state.pane_tree.tiles.get_mut(parent_id)
            {
                match container {
                    Container::Linear(l) => {
                        if let Some(slot) = l.children.iter_mut().find(|id| **id == source_tabs) {
                            *slot = linear;
                        }
                    }
                    Container::Tabs(t) => {
                        if let Some(slot) = t.children.iter_mut().find(|id| **id == source_tabs) {
                            *slot = linear;
                        }
                    }
                }
            }
        }
        self.dock_split_animations
            .insert(linear, DockSplitAnimation::new(vec![1.0, 1.0], 1));
    }

    pub(crate) fn remove_pane_from_tree(&mut self, pane_id: TileId) {
        self.remove_pane_from_tree_without_reconcile(pane_id);
        self.reconcile_tree_state();
    }

    pub(crate) fn remove_panes_from_tree(&mut self, panes: &[TileId]) {
        let tiles_before = self.state.pane_tree.tiles.iter().count();
        crate::perf::event(|| {
            format!(
                "pane_tree.remove_many panes={} tiles_before={} panels_before={} signals_before={}",
                panes.len(),
                tiles_before,
                self.file_panel_cache.len(),
                self.pane_signals.len()
            )
        });
        let _perf = crate::perf::span_detail("pane_tree.remove_many", || {
            format!("panes={} tiles_before={}", panes.len(), tiles_before)
        });
        let mut seen = HashSet::new();
        let mut removed = false;
        for &pane_id in panes {
            if seen.insert(pane_id) && self.state.pane_tree.tiles.get(pane_id).is_some() {
                self.remove_pane_from_tree_without_reconcile(pane_id);
                removed = true;
            }
        }
        if removed {
            self.reconcile_tree_state();
        }
        crate::perf::event(|| {
            format!(
                "pane_tree.remove_many_done removed={} tiles_after={} panels_after={} signals_after={}",
                removed,
                self.state.pane_tree.tiles.iter().count(),
                self.file_panel_cache.len(),
                self.pane_signals.len()
            )
        });
    }

    fn remove_pane_from_tree_without_reconcile(&mut self, pane_id: TileId) {
        let _perf =
            crate::perf::span_detail("pane_tree.remove_one", || format!("pane={}", pane_id.0));
        let tabs_id = self.state.find_tabs_container(pane_id);
        if let Some(tabs_id) = tabs_id {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
                self.state.pane_tree.tiles.get_mut(tabs_id)
            {
                tabs.children.retain(|id| *id != pane_id);
                if tabs.active == Some(pane_id) {
                    tabs.active = tabs.children.last().copied();
                }
            }
            let empty_tabs = matches!(
                self.state.pane_tree.tiles.get(tabs_id),
                Some(Tile::Container(Container::Tabs(tabs))) if tabs.children.is_empty()
            );
            if empty_tabs {
                self.remove_container_from_parent(tabs_id);
                self.state.pane_tree.tiles.remove(tabs_id);
            }
        }

        self.state.pane_tree.tiles.remove(pane_id);
        self.state.tab_states.remove(&pane_id);
        self.watchers.remove(&pane_id);
        self.state.clear_preview(pane_id);
        self.drop_pane_signal(pane_id);
        self.file_panel_cache.remove(&pane_id);
        self.tab_closing.remove(&pane_id);
        self.pane_items_appear.remove(&pane_id);
        self.pane_animations.remove(&pane_id);
        self.pane_layout_bounds.remove(&pane_id);
        self.pane_layout_animations.remove(&pane_id);
    }

    fn remove_container_from_parent(&mut self, id: TileId) {
        if self.state.pane_tree.root == Some(id) {
            self.state.pane_tree.root = self.state.all_pane_tile_ids().first().copied();
            return;
        }
        let Some(parent_id) = self.state.find_parent_of(id) else {
            return;
        };
        if let Some(Tile::Container(container)) = self.state.pane_tree.tiles.get_mut(parent_id) {
            match container {
                Container::Linear(l) => {
                    if let Some(pos) = l.children.iter().position(|child| *child == id) {
                        l.children.remove(pos);
                        if pos < l.shares.len() {
                            l.shares.remove(pos);
                        }
                    }
                }
                Container::Tabs(t) => {
                    t.children.retain(|child| *child != id);
                    if t.active == Some(id) {
                        t.active = t.children.first().copied();
                    }
                }
            }
        }
    }

    pub(crate) fn move_tab_before(&mut self, dragged: TileId, target: TileId) {
        if dragged == target {
            return;
        }
        let Some(target_tabs) = self.state.find_tabs_container(target) else {
            return;
        };
        self.detach_tab_for_move(dragged);
        if let Some(Tile::Container(Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get_mut(target_tabs)
        {
            tabs.children.retain(|id| *id != dragged);
            let pos = tabs
                .children
                .iter()
                .position(|id| *id == target)
                .unwrap_or(tabs.children.len());
            tabs.children.insert(pos, dragged);
            tabs.active = Some(dragged);
        }
        self.push_focus_history();
        self.state.focused_tile = dragged;
        self.cleanup_empty_tabs();
    }

    pub(crate) fn split_existing_tab(
        &mut self,
        target: TileId,
        dragged: TileId,
        vertical: bool,
        before: bool,
    ) {
        let Some(target_tabs) = self.state.find_tabs_container(target) else {
            return;
        };
        if dragged == target {
            match self.state.pane_tree.tiles.get(target_tabs) {
                Some(Tile::Container(Container::Tabs(tabs))) if tabs.children.len() > 1 => {}
                _ => return,
            }
        }
        self.detach_tab_for_move(dragged);
        let new_tabs = self.state.pane_tree.tiles.insert_tab_tile(vec![dragged]);
        self.insert_split_container(target_tabs, new_tabs, vertical, before);
        self.push_focus_history();
        self.state.focused_tile = dragged;
        self.cleanup_empty_tabs();
    }

    fn detach_tab_for_move(&mut self, pane_id: TileId) {
        let Some(source_tabs) = self.state.find_tabs_container(pane_id) else {
            return;
        };
        if let Some(Tile::Container(Container::Tabs(tabs))) =
            self.state.pane_tree.tiles.get_mut(source_tabs)
        {
            tabs.children.retain(|id| *id != pane_id);
            if tabs.active == Some(pane_id) {
                tabs.active = tabs.children.last().copied();
            }
        }
    }

    fn insert_split_container(
        &mut self,
        target_tabs: TileId,
        new_tabs: TileId,
        vertical: bool,
        before: bool,
    ) {
        let dir = if vertical {
            LinearDir::Vertical
        } else {
            LinearDir::Horizontal
        };
        if let Some(parent_id) = self.state.find_parent_of(target_tabs) {
            if let Some(Tile::Container(Container::Linear(linear))) =
                self.state.pane_tree.tiles.get_mut(parent_id)
            {
                if linear.dir == dir {
                    if let Some(pos) = linear.children.iter().position(|id| *id == target_tabs) {
                        let insert_at = if before { pos } else { pos + 1 };
                        let old_len = linear.children.len();
                        if linear.shares.len() != old_len {
                            linear.shares = vec![1.0; old_len];
                        }
                        crate::app::normalize_split_shares(&mut linear.shares);
                        let sum: f32 = linear.shares.iter().copied().filter(|v| *v > 0.0).sum();
                        let new_share = if old_len == 0 || sum <= f32::EPSILON {
                            1.0
                        } else {
                            sum / old_len as f32
                        };
                        linear.children.insert(insert_at, new_tabs);
                        linear.shares.insert(insert_at, new_share);
                        crate::app::normalize_split_shares(&mut linear.shares);
                        return;
                    }
                }
            }
        }

        let children = if before {
            vec![new_tabs, target_tabs]
        } else {
            vec![target_tabs, new_tabs]
        };
        let linear = match dir {
            LinearDir::Horizontal => self.state.pane_tree.tiles.insert_horizontal_tile(children),
            LinearDir::Vertical => self.state.pane_tree.tiles.insert_vertical_tile(children),
        };
        if self.state.pane_tree.root == Some(target_tabs) {
            self.state.pane_tree.root = Some(linear);
            return;
        }
        if let Some(parent_id) = self.state.find_parent_of(target_tabs) {
            if let Some(Tile::Container(container)) = self.state.pane_tree.tiles.get_mut(parent_id)
            {
                match container {
                    Container::Linear(l) => {
                        if let Some(slot) = l.children.iter_mut().find(|id| **id == target_tabs) {
                            *slot = linear;
                        }
                    }
                    Container::Tabs(t) => {
                        if let Some(slot) = t.children.iter_mut().find(|id| **id == target_tabs) {
                            *slot = linear;
                        }
                    }
                }
            }
        }
    }

    fn cleanup_empty_tabs(&mut self) {
        let empty: Vec<TileId> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                matches!(tile, Tile::Container(Container::Tabs(tabs)) if tabs.children.is_empty())
                    .then_some(*id)
            })
            .collect();
        for id in empty {
            self.remove_container_from_parent(id);
            self.state.pane_tree.tiles.remove(id);
        }
        self.reconcile_tree_state();
    }
}
