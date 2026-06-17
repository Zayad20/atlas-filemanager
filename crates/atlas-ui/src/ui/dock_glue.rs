//! FASE A: glue between the neutral `layout_tree` and the `gpui_component` Dock.
//!
//! The Dock (`DockArea`) is the interactive owner of the central layout: it
//! provides tab drag/reorder, drag-to-split and close out of the box. Our
//! `layout_tree` (`AppState::pane_tree`) stays the serializable source of truth
//! that the rest of the app (nav, cycle, session save) reads, so the two are kept
//! in sync:
//!
//! * **tree → dock**: `build_center` projects `pane_tree` into a `DockItem`. We
//!   rebuild it whenever an explicit op (new tab, close, split, activate) bumps
//!   `layout_revision`.
//! * **dock → tree**: on `DockEvent::LayoutChanged` (any interactive change) we
//!   `reconcile_from_dock`, rebuilding the container structure of `pane_tree` from
//!   the dumped panel tree while preserving pane leaves (and their `TabData`) by id.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{px, Axis, Context, Entity, WeakEntity, Window};
use gpui_component::dock::{
    DockArea, DockEvent, DockItem, PanelInfo, PanelState, PanelStyle, PanelView,
};

use crate::app::App;
use crate::state::layout_tree::{Container, LinearDir, Tile, TileId};
use crate::ui::pane_panel::FilePanel;

impl App {
    /// Return the central `DockArea`, creating it (or rebuilding its center when the
    /// layout changed via an explicit op) as needed. Called from `render`.
    pub(crate) fn ensure_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<DockArea> {
        if let Some(dock) = self.dock.clone() {
            if self.dock_revision != self.layout_revision {
                self.rebuild_dock_center(window, cx);
                self.dock_revision = self.layout_revision;
            }
            dock
        } else {
            let dock = self.build_dock(window, cx);
            self.dock = Some(dock.clone());
            self.dock_revision = self.layout_revision;
            dock
        }
    }

    fn build_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<DockArea> {
        // `TabBar` style: every pane shows the segmented pill tab strip (even with a
        // single tab), matching the Figma "Tab selector". Panes that opt out (sidebar)
        // hide it via `Panel::show_title_bar`.
        let dock = cx.new(|cx| {
            DockArea::new("atlas-dock", Some(1), window, cx).panel_style(PanelStyle::TabBar)
        });
        let weak = dock.downgrade();
        let center = self.build_center(&weak, window, cx);
        dock.update(cx, |da, cx| da.set_center(center, window, cx));

        // FASE B: left sidebar lives in the Dock's left dock.
        let app = cx.entity();
        let profile_names: Vec<String> = self
            .state
            .profiles
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let active_profile = self.state.profiles.active_idx;
        let sidebar = cx.new(|cx| {
            crate::ui::sidebar::SidebarPanel::new(app, profile_names, active_profile, window, cx)
        });
        let left = DockItem::tabs(
            vec![Arc::new(sidebar) as Arc<dyn PanelView>],
            &weak,
            window,
            cx,
        );
        let sidebar_width = self.state.settings.sidebar_width;
        dock.update(cx, |da, cx| {
            da.set_left_dock(left, Some(px(sidebar_width)), true, window, cx);
            // Hide the in-tab dock toggle button (the panel glyph in the tab bar).
            // The sidebar is toggled via the ToggleSidebar keyboard action instead.
            da.set_toggle_button_visible(false, cx);
        });

        cx.subscribe_in(&dock, window, |this, _dock, ev: &DockEvent, window, cx| {
            if matches!(ev, DockEvent::LayoutChanged) {
                this.reconcile_from_dock(window, cx);
                cx.notify();
            }
        })
        .detach();
        dock
    }

    fn rebuild_dock_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dock) = self.dock.clone() else {
            return;
        };
        let weak = dock.downgrade();
        let center = self.build_center(&weak, window, cx);
        dock.update(cx, |da, cx| da.set_center(center, window, cx));
    }

    /// Project `pane_tree` into the Dock's center `DockItem`. Always wrapped in a
    /// top-level `Split` so the Dock's `subscribe_item` forwards `LayoutChanged`
    /// (a bare top-level `Tabs` is not subscribed by the library).
    fn build_center(
        &mut self,
        weak: &WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DockItem {
        let item = self
            .state
            .pane_tree
            .root
            .and_then(|root| self.build_item(root, weak, window, cx));
        match item {
            Some(split @ DockItem::Split { .. }) => split,
            Some(other) => DockItem::split(Axis::Horizontal, vec![other], weak, window, cx),
            None => DockItem::split(Axis::Horizontal, vec![], weak, window, cx),
        }
    }

    fn get_or_make_file_panel(
        &mut self,
        id: TileId,
        cx: &mut Context<Self>,
    ) -> Entity<crate::ui::pane_panel::FilePanel> {
        if let Some(panel) = self.file_panel_cache.get(&id) {
            return panel.clone();
        }
        let panel = make_file_panel(self, id, cx);
        self.file_panel_cache.insert(id, panel.clone());
        panel
    }

    fn build_item(
        &mut self,
        id: TileId,
        weak: &WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DockItem> {
        match self.state.pane_tree.tiles.get(id) {
            Some(Tile::Pane(_)) => {
                let panel = self.get_or_make_file_panel(id, cx);
                Some(DockItem::tab(panel, weak, window, cx))
            }
            Some(Tile::Container(Container::Tabs(tabs))) => {
                let children = tabs.children.clone();
                let active = tabs.active;
                // Separate immutable filter pass from the mutable make_file_panel pass
                // to satisfy the borrow checker.
                let pane_ids: Vec<TileId> = children
                    .iter()
                    .filter(|&&c| matches!(self.state.pane_tree.tiles.get(c), Some(Tile::Pane(_))))
                    .copied()
                    .collect();
                let panels: Vec<Arc<dyn PanelView>> = pane_ids
                    .iter()
                    .map(|&c| Arc::new(self.get_or_make_file_panel(c, cx)) as Arc<dyn PanelView>)
                    .collect();
                if panels.is_empty() {
                    return None;
                }
                let active_ix = active
                    .and_then(|a| children.iter().position(|&c| c == a))
                    .unwrap_or(0);
                Some(DockItem::tabs(panels, weak, window, cx).active_index(active_ix, cx))
            }
            Some(Tile::Container(Container::Linear(lin))) => {
                let children = lin.children.clone();
                let vertical = matches!(lin.dir, LinearDir::Vertical);
                let items: Vec<DockItem> = children
                    .iter()
                    .filter_map(|&c| self.build_item(c, weak, window, cx))
                    .collect();
                if items.is_empty() {
                    return None;
                }
                let axis = if vertical {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                };
                Some(DockItem::split(axis, items, weak, window, cx))
            }
            None => None,
        }
    }

    /// Rebuild `pane_tree`'s container structure from the Dock's dumped layout,
    /// preserving pane leaves (and their `TabData`) by id. Triggered by interactive
    /// Dock changes (drag/reorder/split/close/select).
    pub(crate) fn reconcile_from_dock(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(dock) = self.dock.clone() else {
            return;
        };
        let state = dock.read(cx).dump(cx);

        // Old container tiles to drop after rebuilding (pane leaves are reused).
        let old_containers: Vec<TileId> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, t)| matches!(t, Tile::Container(_)).then_some(*id))
            .collect();

        let Some(new_root) = self.install_panelstate(&state.center) else {
            return;
        };
        for cid in old_containers {
            self.state.pane_tree.tiles.remove(cid);
        }
        self.state.pane_tree.root = Some(new_root);

        // Drop panes no longer reachable (closed / dragged out), plus their state.
        let live: HashSet<TileId> = self.state.all_pane_tile_ids().into_iter().collect();
        let dead_panes: Vec<TileId> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, t)| (matches!(t, Tile::Pane(_)) && !live.contains(id)).then_some(*id))
            .collect();
        for id in dead_panes {
            self.state.pane_tree.tiles.remove(id);
        }

        self.reconcile_tree_state();

        // Prune cache entries for panes that no longer exist.
        let live: std::collections::HashSet<TileId> =
            self.state.all_pane_tile_ids().into_iter().collect();
        self.file_panel_cache.retain(|id, _| live.contains(id));
    }

    /// Recursively rebuild a `pane_tree` node from a dumped `PanelState`. Returns the
    /// `TileId` of the (re)created node, or `None` to skip empty/unknown nodes.
    fn install_panelstate(&mut self, ps: &PanelState) -> Option<TileId> {
        match ps.panel_name.as_str() {
            "FilePanel" => {
                let pane_id = match &ps.info {
                    PanelInfo::Panel(v) => v.get("pane_id").and_then(serde_json::Value::as_u64),
                    _ => None,
                }
                .map(TileId)?;
                matches!(self.state.pane_tree.tiles.get(pane_id), Some(Tile::Pane(_)))
                    .then_some(pane_id)
            }
            "TabPanel" => {
                let kids: Vec<TileId> = ps
                    .children
                    .iter()
                    .filter_map(|c| self.install_panelstate(c))
                    .collect();
                if kids.is_empty() {
                    return None;
                }
                let active_ix = ps.info.active_index().unwrap_or(0);
                let id = self.state.pane_tree.tiles.insert_tab_tile(kids.clone());
                if let Some(Tile::Container(Container::Tabs(tabs))) =
                    self.state.pane_tree.tiles.get_mut(id)
                {
                    tabs.active = kids
                        .get(active_ix)
                        .copied()
                        .or_else(|| kids.first().copied());
                }
                Some(id)
            }
            "StackPanel" => {
                let kids: Vec<TileId> = ps
                    .children
                    .iter()
                    .filter_map(|c| self.install_panelstate(c))
                    .collect();
                match kids.len() {
                    0 => None,
                    // Collapse a single-child stack: avoids growing nested wrappers.
                    1 => Some(kids[0]),
                    _ => {
                        let vertical = matches!(ps.info.axis(), Some(Axis::Vertical));
                        let id = if vertical {
                            self.state
                                .pane_tree
                                .tiles
                                .insert_vertical_tile(kids.clone())
                        } else {
                            self.state
                                .pane_tree
                                .tiles
                                .insert_horizontal_tile(kids.clone())
                        };
                        if let Some(sizes) = ps.info.sizes() {
                            if sizes.len() == kids.len() {
                                if let Some(Tile::Container(Container::Linear(lin))) =
                                    self.state.pane_tree.tiles.get_mut(id)
                                {
                                    lin.shares = sizes.iter().map(|p| f32::from(*p)).collect();
                                }
                            }
                        }
                        Some(id)
                    }
                }
            }
            _ => None,
        }
    }
}

/// Create a `FilePanel` entity for `id`, handing it a handle back to the app.
fn make_file_panel(this: &mut App, id: TileId, cx: &mut Context<App>) -> Entity<FilePanel> {
    let app = cx.entity();
    // Compute the pane signal here (App context available) so FilePanel::new
    // doesn't need to call app.update(cx, ...) while App is already leased.
    let signal = this.pane_signal_for(id, cx);
    cx.new(|cx| FilePanel::new(app, id, signal, cx))
}
