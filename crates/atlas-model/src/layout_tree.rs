#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
//! Framework-neutral layout tree for panes/tabs/splits.
//!
//! Replaces `egui_tiles::Tree` so `src/state` carries no UI-framework dependency
//! (see `ARCHITECTURE.md`: state must never import the UI). The public surface mirrors
//! the slice of the `egui_tiles` API the app actually used, so call sites migrate
//! by swapping the `egui_tiles::` path for `crate::layout_tree::`.
//!
//! The renderer (gpui) reads this tree to lay out panes and emits `Action`s for any
//! structural change — the tree never mutates itself during rendering.

use std::collections::{BTreeMap, HashSet};

/// Stable identity of a tile (a pane leaf or a container). Monotonic `u64` so a
/// `BTreeMap` keeps tiles in insertion order — `all_pane_tile_ids().first()` is then
/// deterministic (oldest pane), unlike the old hash-ordered slotmap.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct TileId(pub u64);

/// Orientation of a `Linear` split container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LinearDir {
    Horizontal,
    Vertical,
}

/// A row/column of children, each sized by its share. `shares` is parallel to
/// `children`; the renderer normalizes them into ratios.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Linear {
    pub children: Vec<TileId>,
    pub dir: LinearDir,
    #[serde(default)]
    pub shares: Vec<f32>,
}

impl Linear {
    pub fn new(dir: LinearDir, children: Vec<TileId>) -> Self {
        let shares = vec![1.0; children.len()];
        Self {
            children,
            dir,
            shares,
        }
    }
}

/// A stack of tabbed children with one active at a time.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Tabs {
    pub children: Vec<TileId>,
    pub active: Option<TileId>,
}

impl Tabs {
    pub fn new(children: Vec<TileId>) -> Self {
        let active = children.first().copied();
        Self { children, active }
    }

    pub fn add_child(&mut self, id: TileId) {
        self.children.push(id);
    }

    pub const fn set_active(&mut self, id: TileId) {
        self.active = Some(id);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Container {
    Tabs(Tabs),
    Linear(Linear),
}

/// A node in the tree: either a content pane carrying `Pane` data, or a container
/// arranging other tiles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Tile<Pane> {
    Pane(Pane),
    Container(Container),
}

/// Slotmap of tiles keyed by a monotonic `TileId`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tiles<Pane> {
    map: BTreeMap<TileId, Tile<Pane>>,
    next_id: u64,
}

impl<Pane> Default for Tiles<Pane> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
            next_id: 0,
        }
    }
}

impl<Pane> Tiles<Pane> {
    fn alloc_id(&mut self) -> TileId {
        // Never collide with ids restored from a persisted session.
        let restored_max = self.map.keys().last().map_or(0, |id| id.0 + 1);
        let id = TileId(self.next_id.max(restored_max));
        self.next_id = id.0 + 1;
        id
    }

    pub fn get(&self, id: TileId) -> Option<&Tile<Pane>> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: TileId) -> Option<&mut Tile<Pane>> {
        self.map.get_mut(&id)
    }

    /// Iterate tiles in ascending id order (insertion order).
    pub fn iter(&self) -> impl Iterator<Item = (&TileId, &Tile<Pane>)> {
        self.map.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&TileId, &mut Tile<Pane>)> {
        self.map.iter_mut()
    }

    pub fn insert_pane(&mut self, pane: Pane) -> TileId {
        let id = self.alloc_id();
        self.map.insert(id, Tile::Pane(pane));
        id
    }

    pub fn insert_tab_tile(&mut self, children: Vec<TileId>) -> TileId {
        let id = self.alloc_id();
        self.map
            .insert(id, Tile::Container(Container::Tabs(Tabs::new(children))));
        id
    }

    pub fn insert_horizontal_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_linear(LinearDir::Horizontal, children)
    }

    pub fn insert_vertical_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_linear(LinearDir::Vertical, children)
    }

    fn insert_linear(&mut self, dir: LinearDir, children: Vec<TileId>) -> TileId {
        let id = self.alloc_id();
        self.map.insert(
            id,
            Tile::Container(Container::Linear(Linear::new(dir, children))),
        );
        id
    }

    pub fn remove(&mut self, id: TileId) -> Option<Tile<Pane>> {
        self.map.remove(&id)
    }

    pub fn get_pane(&self, id: &TileId) -> Option<&Pane> {
        match self.map.get(id) {
            Some(Tile::Pane(pane)) => Some(pane),
            _ => None,
        }
    }
}

/// The pane/tab/split tree. `root` is the top container (or a lone pane).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Tree<Pane> {
    pub root: Option<TileId>,
    pub tiles: Tiles<Pane>,
}

impl<Pane> Tree<Pane> {
    pub fn empty() -> Self {
        Self {
            root: None,
            tiles: Tiles::default(),
        }
    }

    /// Make the first tile matching `predicate` the active tab of every `Tabs`
    /// container that directly holds it. Mirrors `egui_tiles::Tree::make_active`
    /// for the single use site (focusing a tab by id).
    pub fn make_active(&mut self, mut predicate: impl FnMut(TileId, &Tile<Pane>) -> bool) {
        // Resolve the matching id first to avoid borrowing `tiles` twice.
        let target = self
            .tiles
            .iter()
            .find(|(id, tile)| predicate(**id, tile))
            .map(|(id, _)| *id);
        let Some(target) = target else { return };
        for tile in self.tiles.map.values_mut() {
            if let Tile::Container(Container::Tabs(tabs)) = tile {
                if tabs.children.contains(&target) {
                    tabs.active = Some(target);
                }
            }
        }
    }

    /// Repair a restored tree before the UI reads it.
    ///
    /// Older builds could persist empty split containers after aggressive pane
    /// churn. Those nodes are legal to deserialize but not useful to render: a
    /// branch with no pane can still consume split space and leave the real pane
    /// collapsed off-screen. Keep only branches that lead to panes, collapse
    /// one-child split chains, and replace invalid shares with sane weights.
    pub fn sanitize(&mut self)
    where
        Pane: Clone,
    {
        let Some(root) = self.root else {
            self.tiles.map.clear();
            return;
        };

        let mut visiting = HashSet::new();
        self.root = self.sanitize_node(root, &mut visiting);

        let mut reachable = HashSet::new();
        if let Some(root) = self.root {
            self.collect_reachable(root, &mut reachable);
        }
        self.tiles.map.retain(|id, _| reachable.contains(id));
    }

    fn sanitize_node(&mut self, id: TileId, visiting: &mut HashSet<TileId>) -> Option<TileId>
    where
        Pane: Clone,
    {
        if !self.tiles.map.contains_key(&id) || !visiting.insert(id) {
            return None;
        }

        let tile = self.tiles.map.get(&id)?.clone();
        let result = match tile {
            Tile::Pane(_) => Some(id),
            Tile::Container(Container::Tabs(tabs)) => {
                let mut children = Vec::new();
                let mut seen = HashSet::new();
                for child in tabs.children {
                    if let Some(child) = self.sanitize_node(child, visiting) {
                        if seen.insert(child) {
                            children.push(child);
                        }
                    }
                }

                match children.len() {
                    0 => None,
                    1 => Some(children[0]),
                    _ => {
                        let active = tabs
                            .active
                            .and_then(|active| children.contains(&active).then_some(active))
                            .or_else(|| children.first().copied());
                        if let Some(Tile::Container(Container::Tabs(tabs))) =
                            self.tiles.map.get_mut(&id)
                        {
                            tabs.children = children;
                            tabs.active = active;
                        }
                        Some(id)
                    }
                }
            }
            Tile::Container(Container::Linear(linear)) => {
                let mut children = Vec::new();
                let mut shares = Vec::new();
                let mut seen = HashSet::new();
                for (ix, child) in linear.children.into_iter().enumerate() {
                    let Some(child) = self.sanitize_node(child, visiting) else {
                        continue;
                    };
                    if !seen.insert(child) {
                        continue;
                    }
                    children.push(child);
                    let share = linear.shares.get(ix).copied().unwrap_or(1.0);
                    shares.push(if share.is_finite() && share > 0.0 {
                        share
                    } else {
                        1.0
                    });
                }

                match children.len() {
                    0 => None,
                    1 => Some(children[0]),
                    _ => {
                        if let Some(Tile::Container(Container::Linear(linear))) =
                            self.tiles.map.get_mut(&id)
                        {
                            linear.children = children;
                            linear.shares = shares;
                        }
                        Some(id)
                    }
                }
            }
        };

        visiting.remove(&id);
        result
    }

    fn collect_reachable(&self, id: TileId, reachable: &mut HashSet<TileId>) {
        if !reachable.insert(id) {
            return;
        }
        let Some(tile) = self.tiles.map.get(&id) else {
            return;
        };
        match tile {
            Tile::Pane(_) => {}
            Tile::Container(Container::Tabs(tabs)) => {
                for child in &tabs.children {
                    self.collect_reachable(*child, reachable);
                }
            }
            Tile::Container(Container::Linear(linear)) => {
                for child in &linear.children {
                    self.collect_reachable(*child, reachable);
                }
            }
        }
    }

    /// Insert `new_id` immediately after `target_id` when the target already lives
    /// in a `Linear` container with the requested direction. Returns `true` when
    /// the insertion happened, or `false` when callers should wrap `target_id` in a
    /// new two-child split instead.
    pub fn insert_linear_sibling_after(
        &mut self,
        target_id: TileId,
        new_id: TileId,
        dir: LinearDir,
    ) -> bool {
        for tile in self.tiles.map.values_mut() {
            let Tile::Container(Container::Linear(linear)) = tile else {
                continue;
            };
            if linear.dir != dir {
                continue;
            }
            let Some(pos) = linear.children.iter().position(|id| *id == target_id) else {
                continue;
            };

            let old_len = linear.children.len();
            if linear.shares.len() != old_len {
                linear.shares = vec![1.0; old_len];
            }

            let sum: f32 = linear.shares.iter().copied().filter(|v| *v > 0.0).sum();
            let new_share = if old_len == 0 || sum <= f32::EPSILON {
                1.0
            } else {
                sum / old_len as f32
            };

            linear.children.insert(pos + 1, new_id);
            linear.shares.insert(pos + 1, new_share);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_ordered() {
        let mut tiles: Tiles<&str> = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        assert_eq!(a, TileId(0));
        assert_eq!(b, TileId(1));
        let order: Vec<_> = tiles.iter().map(|(id, _)| *id).collect();
        assert_eq!(order, vec![a, b]);
    }

    #[test]
    fn tab_tile_activates_first_child() {
        let mut tiles: Tiles<&str> = Tiles::default();
        let p = tiles.insert_pane("p");
        let tabs_id = tiles.insert_tab_tile(vec![p]);
        match tiles.get(tabs_id) {
            Some(Tile::Container(Container::Tabs(t))) => assert_eq!(t.active, Some(p)),
            _ => panic!("expected tabs container"),
        }
    }

    #[test]
    fn make_active_sets_parent_tabs() {
        let mut tree: Tree<&str> = Tree::empty();
        let p1 = tree.tiles.insert_pane("1");
        let p2 = tree.tiles.insert_pane("2");
        let tabs = tree.tiles.insert_tab_tile(vec![p1, p2]);
        tree.root = Some(tabs);
        tree.make_active(|id, _| id == p2);
        match tree.tiles.get(tabs) {
            Some(Tile::Container(Container::Tabs(t))) => assert_eq!(t.active, Some(p2)),
            _ => panic!("expected tabs container"),
        }
    }

    #[test]
    fn alloc_id_avoids_restored_collisions() {
        // Simulate a tree restored from session with high ids but next_id=0.
        let mut tiles: Tiles<&str> = Tiles::default();
        tiles.map.insert(TileId(5), Tile::Pane("restored"));
        let fresh = tiles.insert_pane("fresh");
        assert_eq!(fresh, TileId(6));
    }

    #[test]
    fn same_axis_split_inserts_sibling_instead_of_nesting() {
        let mut tree: Tree<&str> = Tree::empty();
        let p1 = tree.tiles.insert_pane("1");
        let p2 = tree.tiles.insert_pane("2");
        let p3 = tree.tiles.insert_pane("3");
        let t1 = tree.tiles.insert_tab_tile(vec![p1]);
        let t2 = tree.tiles.insert_tab_tile(vec![p2]);
        let t3 = tree.tiles.insert_tab_tile(vec![p3]);
        let root = tree.tiles.insert_horizontal_tile(vec![t1, t2]);
        tree.root = Some(root);

        assert!(tree.insert_linear_sibling_after(t2, t3, LinearDir::Horizontal));

        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) => {
                assert_eq!(linear.children, vec![t1, t2, t3]);
                assert_eq!(linear.shares, vec![1.0, 1.0, 1.0]);
            }
            _ => panic!("expected linear root"),
        }
    }

    #[test]
    fn same_axis_split_preserves_existing_share_ratios() {
        let mut tree: Tree<&str> = Tree::empty();
        let p1 = tree.tiles.insert_pane("1");
        let p2 = tree.tiles.insert_pane("2");
        let p3 = tree.tiles.insert_pane("3");
        let t1 = tree.tiles.insert_tab_tile(vec![p1]);
        let t2 = tree.tiles.insert_tab_tile(vec![p2]);
        let t3 = tree.tiles.insert_tab_tile(vec![p3]);
        let root = tree.tiles.insert_horizontal_tile(vec![t1, t2]);
        if let Some(Tile::Container(Container::Linear(linear))) = tree.tiles.get_mut(root) {
            linear.shares = vec![2.0, 1.0];
        }

        assert!(tree.insert_linear_sibling_after(t1, t3, LinearDir::Horizontal));

        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) => {
                assert_eq!(linear.children, vec![t1, t3, t2]);
                assert_eq!(linear.shares, vec![2.0, 1.5, 1.0]);
            }
            _ => panic!("expected linear root"),
        }
    }

    #[test]
    fn different_axis_split_reports_no_insert() {
        let mut tree: Tree<&str> = Tree::empty();
        let p1 = tree.tiles.insert_pane("1");
        let p2 = tree.tiles.insert_pane("2");
        let p3 = tree.tiles.insert_pane("3");
        let t1 = tree.tiles.insert_tab_tile(vec![p1]);
        let t2 = tree.tiles.insert_tab_tile(vec![p2]);
        let t3 = tree.tiles.insert_tab_tile(vec![p3]);
        let root = tree.tiles.insert_horizontal_tile(vec![t1, t2]);
        tree.root = Some(root);

        assert!(!tree.insert_linear_sibling_after(t2, t3, LinearDir::Vertical));

        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) => {
                assert_eq!(linear.children, vec![t1, t2]);
            }
            _ => panic!("expected linear root"),
        }
    }

    #[test]
    fn sanitize_removes_empty_split_branches() {
        let mut tree = Tree::empty();
        let pane = tree.tiles.insert_pane("pane");
        let tabs = tree.tiles.insert_tab_tile(vec![pane]);
        let empty = tree.tiles.insert_horizontal_tile(vec![]);
        let root = tree.tiles.insert_vertical_tile(vec![empty, tabs]);
        tree.root = Some(root);

        tree.sanitize();

        assert_eq!(tree.root, Some(pane));
        assert_eq!(tree.tiles.iter().count(), 1);
        assert!(matches!(tree.tiles.get(pane), Some(Tile::Pane(_))));
    }

    #[test]
    fn sanitize_replaces_invalid_split_shares() {
        let mut tree = Tree::empty();
        let a = tree.tiles.insert_pane("a");
        let b = tree.tiles.insert_pane("b");
        let root = tree.tiles.insert_horizontal_tile(vec![a, b]);
        if let Some(Tile::Container(Container::Linear(linear))) = tree.tiles.get_mut(root) {
            linear.shares = vec![f32::INFINITY, -10.0];
        }
        tree.root = Some(root);

        tree.sanitize();

        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) => {
                assert_eq!(linear.children, vec![a, b]);
                assert_eq!(linear.shares, vec![1.0, 1.0]);
            }
            other => panic!("expected linear root, got {other:?}"),
        }
    }
}
