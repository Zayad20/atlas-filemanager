//! Framework-neutral layout tree for panes/tabs/splits.
//!
//! Replaces `egui_tiles::Tree` so `src/state` carries no UI-framework dependency
//! (see `the architecture notes`: state must never import the UI). The public surface mirrors
//! the slice of the `egui_tiles` API the app actually used, so call sites migrate
//! by swapping the `egui_tiles::` path for `crate::state::layout_tree::`.
//!
//! The renderer (gpui) reads this tree to lay out panes and emits `Action`s for any
//! structural change — the tree never mutates itself during rendering.

use std::collections::BTreeMap;

/// Stable identity of a tile (a pane leaf or a container). Monotonic `u64` so a
/// `BTreeMap` keeps tiles in insertion order — `all_pane_tile_ids().first()` is then
/// deterministic (oldest pane), unlike the old hash-ordered slotmap.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
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

    pub fn set_active(&mut self, id: TileId) {
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
        let restored_max = self.map.keys().last().map(|id| id.0 + 1).unwrap_or(0);
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
        for (_, tile) in self.tiles.map.iter_mut() {
            if let Tile::Container(Container::Tabs(tabs)) = tile {
                if tabs.children.contains(&target) {
                    tabs.active = Some(target);
                }
            }
        }
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
}
