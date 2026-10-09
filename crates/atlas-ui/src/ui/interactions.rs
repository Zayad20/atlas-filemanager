//! Pane interactions: rubber-band selection, inline rename, type-to-filter, ctrl-scroll.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::{App, RubberTarget};
use crate::input::action::Action;
use crate::ui::edge_autoscroll::{self, EdgeDirection};
use atlas_model::layout_tree::Tile;
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{
    canvas, Context, DispatchPhase, Entity, Focusable, MouseMoveEvent, MouseUpEvent,
    ScrollWheelEvent, Window,
};
use gpui_component::input::{IndentInline, OutdentInline};
use gpui_component::{ActiveTheme, WindowExt};
use std::time::Instant;

impl App {
    pub(crate) fn pane_view_slider_value(&self, pane_id: TabId) -> f32 {
        self.tab_data_for(pane_id).map_or(0.0, |pane| {
            Self::slider_value_for_view(&pane.view_mode, pane.thumbnail_size)
        })
    }

    pub(crate) fn slider_value_for_view(mode: &ViewMode, thumbnail_size: f32) -> f32 {
        match mode {
            ViewMode::List => 0.0,
            ViewMode::Details => 1.0,
            ViewMode::Grid => 2.0 + ((thumbnail_size - 64.0) / 16.0).round().clamp(0.0, 20.0),
            ViewMode::Miller => 1.0,
        }
    }

    pub(crate) fn set_pane_view_from_slider_value(&mut self, pane_id: TabId, value: f32) -> bool {
        let value = value.round().clamp(0.0, 22.0);
        if value <= 0.0 {
            self.set_pane_view_mode(pane_id, ViewMode::List)
        } else if value <= 1.0 {
            self.set_pane_view_mode(pane_id, ViewMode::Details)
        } else {
            let thumbnail_size = 64.0 + (value - 2.0) * 16.0;
            self.set_pane_grid_thumbnail_size(pane_id, thumbnail_size)
        }
    }

    pub(crate) fn set_pane_view_mode(&mut self, pane_id: TabId, mode: ViewMode) -> bool {
        let mut folder_pref_write = None;
        let mut changed = false;
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(pane_id) {
            if pane.view_mode != mode {
                pane.view_mode = mode.clone();
                changed = true;
            }
            if let atlas_model::location::Location::Directory(path) = &pane.location {
                folder_pref_write = Some((
                    path.clone(),
                    atlas_model::profiles::FolderViewPref {
                        view_mode: mode.clone(),
                        sort_col: pane.sort_col.clone(),
                        sort_dir: pane.sort_dir.clone(),
                    },
                ));
            }
        }

        if !changed {
            return false;
        }

        self.activate_tab(pane_id);
        self.dirty_panes.insert(pane_id);
        self.save_folder_view_pref(folder_pref_write);
        self.reload_flat_entries_if_needed(pane_id, &mode);
        true
    }

    fn set_pane_grid_thumbnail_size(&mut self, pane_id: TabId, thumbnail_size: f32) -> bool {
        let thumbnail_size = thumbnail_size.clamp(64.0, 384.0);
        let mut folder_pref_write = None;
        let mut changed = false;
        if let Some(Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(pane_id) {
            if pane.view_mode != ViewMode::Grid {
                pane.view_mode = ViewMode::Grid;
                changed = true;
            }
            if (pane.thumbnail_size - thumbnail_size).abs() > f32::EPSILON {
                pane.thumbnail_size = thumbnail_size;
                changed = true;
            }
            if let atlas_model::location::Location::Directory(path) = &pane.location {
                folder_pref_write = Some((
                    path.clone(),
                    atlas_model::profiles::FolderViewPref {
                        view_mode: ViewMode::Grid,
                        sort_col: pane.sort_col.clone(),
                        sort_dir: pane.sort_dir.clone(),
                    },
                ));
            }
        }

        if !changed {
            return false;
        }

        self.activate_tab(pane_id);
        self.dirty_panes.insert(pane_id);
        self.save_folder_view_pref(folder_pref_write);
        self.reload_flat_entries_if_needed(pane_id, &ViewMode::Grid);
        true
    }

    fn reload_flat_entries_if_needed(&mut self, pane_id: TabId, mode: &ViewMode) {
        if *mode == ViewMode::Miller {
            return;
        }
        let needs_load = self
            .state
            .tab_states
            .get(&pane_id)
            .is_some_and(|ts| ts.entries.is_empty());
        if needs_load {
            if let Some(td) = self.tab_data_for(pane_id) {
                let loc = td.location.clone();
                let col = td.sort_col.clone();
                let dir = td.sort_dir.clone();
                self.dispatch_list(pane_id, loc, col, dir);
            }
        }
    }

    fn rubber_target_bounds(&self, target: RubberTarget) -> Option<gpui::Bounds<gpui::Pixels>> {
        if let Some(rubber) = self.rubber.as_ref() {
            if rubber.target == target {
                return Some(rubber.target_bounds);
            }
        }
        match target {
            RubberTarget::Pane(pane) => self.rubber_pane_bounds(pane),
            RubberTarget::MillerColumn(pane, column) => {
                self.miller_column_bounds.get(&(pane, column)).copied()
            }
            RubberTarget::Dropzone => self.dropzone_bounds,
        }
    }

    fn rubber_pane_bounds(&self, pane: TabId) -> Option<gpui::Bounds<gpui::Pixels>> {
        // Details records the actual rows viewport. The generic pane bounds also
        // include breadcrumb/header chrome, which shifts the marquee's content
        // coordinates by that chrome height and selects the wrong rows.
        if let Some(tab) = self.tab_data_for(pane) {
            match tab.view_mode {
                ViewMode::Details => {
                    if let Some(model) = self.details_hit.get(&pane) {
                        return Some(model.bounds);
                    }
                }
                ViewMode::Grid => {
                    if let Some(model) = self.grid_hit.get(&pane) {
                        return Some(model.bounds);
                    }
                }
                ViewMode::List => {
                    if let Some(model) = self.list_hit.get(&pane) {
                        return Some(model.bounds);
                    }
                }
                _ => {}
            }
        }
        self.pane_view_bounds
            .get(&pane)
            .or_else(|| self.pane_layout_bounds.get(&pane))
            .copied()
    }

    pub(crate) fn clamp_rubber_target_point(
        &self,
        target: RubberTarget,
        pos: gpui::Point<gpui::Pixels>,
    ) -> gpui::Point<gpui::Pixels> {
        let Some(bounds) = self.rubber_target_bounds(target) else {
            return pos;
        };
        gpui::point(
            gpui::px(f32::from(pos.x).clamp(f32::from(bounds.left()), f32::from(bounds.right()))),
            gpui::px(f32::from(pos.y).clamp(f32::from(bounds.top()), f32::from(bounds.bottom()))),
        )
    }

    fn clamp_point_to_bounds(
        bounds: gpui::Bounds<gpui::Pixels>,
        pos: gpui::Point<gpui::Pixels>,
    ) -> gpui::Point<gpui::Pixels> {
        gpui::point(
            gpui::px(f32::from(pos.x).clamp(f32::from(bounds.left()), f32::from(bounds.right()))),
            gpui::px(f32::from(pos.y).clamp(f32::from(bounds.top()), f32::from(bounds.bottom()))),
        )
    }

    pub(crate) fn ensure_drag_selection(&mut self, pane: TabId, entry_idx: usize) {
        if let Some(ts) = self.state.tab_states.get_mut(&pane) {
            if !ts.selection.selected.contains(&entry_idx) {
                ts.selection.selected.clear();
                ts.selection.selected.insert(entry_idx);
            }
        }
    }

    pub(crate) fn select_context_menu_paths(
        &mut self,
        pane: TabId,
        entry_idx: usize,
    ) -> Vec<std::path::PathBuf> {
        self.ensure_drag_selection(pane, entry_idx);
        self.selected_paths()
    }

    pub(crate) fn details_entry_at(
        &self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
    ) -> Option<usize> {
        self.details_hit
            .get(&pane)
            .and_then(|model| model.hit_entry(pos))
    }

    pub(crate) fn details_contains_point(
        &self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
    ) -> bool {
        self.details_hit
            .get(&pane)
            .is_some_and(|model| model.bounds.contains(&pos))
    }

    pub(crate) fn details_row_selected(&self, pane: TabId, entry_idx: usize) -> bool {
        self.state
            .tab_states
            .get(&pane)
            .is_some_and(|ts| ts.selection.selected.contains(&entry_idx))
    }

    pub(crate) fn details_hover_at(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
    ) -> Option<(TabId, usize)> {
        self.details_entry_at(pane, pos).map(|idx| (pane, idx))
    }

    pub(crate) fn details_left_mouse_down(
        &mut self,
        pane: TabId,
        entry_idx: Option<usize>,
        pos: gpui::Point<gpui::Pixels>,
        shift: bool,
        ctrl: bool,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        if self.rename_edit.is_some() {
            self.commit_rename(cx);
        }

        let Some(entry_idx) = entry_idx else {
            self.begin_rubber(pane, pos, shift || ctrl);
            cx.notify();
            return;
        };

        if click_count >= 2 {
            self.process_actions(vec![Action::Activate {
                tile: pane,
                idx: entry_idx,
            }]);
            cx.notify();
            return;
        }

        let selected = self.details_row_selected(pane, entry_idx);
        if selected && !shift && !ctrl {
            self.activate_tab(pane);
            cx.notify();
            return;
        }

        if !selected {
            self.begin_rubber(pane, pos, shift || ctrl);
        }
        self.process_actions(vec![Action::Select {
            tile: pane,
            idx: entry_idx,
            shift,
            ctrl,
        }]);
        cx.notify();
    }

    pub(crate) fn details_right_mouse_down(
        &mut self,
        pane: TabId,
        entry_idx: Option<usize>,
        pos: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry_idx) = entry_idx {
            self.activate_tab(pane);
            let paths = self.select_context_menu_paths(pane, entry_idx);
            self.open_ctx_menu(paths, pos, window, cx);
            cx.notify();
            return;
        }

        if let Some(path) = self
            .tab_data_for(pane)
            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
        {
            self.open_background_ctx_menu(Some(path), pos, window, cx);
            cx.notify();
        }
    }

    /// Compute (cols, row_count) for autoscroll in the given pane. Uses tab_state
    /// pixel width for Grid/List; returns (1, count) for Details; (1, 0) for Miller.
    pub(crate) fn rubber_scroll_dims(&self, pane: TabId) -> (usize, usize) {
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return (1, 0);
        };
        let Some(td) = self.tab_data_for(pane) else {
            return (1, 0);
        };
        let count = ts.visible.len();
        match &td.view_mode {
            atlas_model::settings::ViewMode::Details => (1, count),
            atlas_model::settings::ViewMode::Grid => {
                let thumb = td.thumbnail_size;
                const GAP: f32 = crate::ui::views::grid::GRID_GAP;
                const PAD: f32 = crate::ui::views::grid::GRID_PAD;
                let pane_w = ts.last_pane_width.max(thumb + 1.0);
                let inner = (pane_w - 2.0 * PAD).max(thumb);
                let cols = (((inner + GAP) / (thumb + GAP)).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            atlas_model::settings::ViewMode::List => {
                const CELL_W: f32 = 220.0;
                let stride = CELL_W + 6.0;
                let avail = ts.last_pane_width.max(stride);
                let cols = ((avail / stride).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            atlas_model::settings::ViewMode::Miller => (1, 0),
        }
    }

    /// Phase C: recompute the focused rubber-band selection by intersecting its
    /// rectangle (window coords) against the recorded visible-item rects.
    ///
    /// Hit-test is a full AABB overlap (any contact counts), matching the
    /// Finder/Explorer/marquee convention: a rect that merely grazes the corner of
    /// an item selects it. The old center-point test made items "jump" into the
    /// selection only once the marquee crossed their midpoint, which read as laggy
    /// and "not marking objects in real time".
    pub(crate) fn apply_rubber(&mut self) {
        let Some(r) = self.rubber.as_ref() else {
            return;
        };
        if !r.started {
            return;
        }
        let target = r.target;
        // Clamp both endpoints to the pane so the hit-test region matches the
        // marquee rectangle that is actually drawn (which is also clamped in the
        // paint callback). Without this the visible box and the selection region
        // could disagree near pane edges → "misalignment".
        let origin = self.clamp_rubber_target_point(target, r.origin);
        let current = self.clamp_rubber_target_point(target, r.current);
        let (ox, oy, cx_, cy) = match target {
            RubberTarget::Pane(_) | RubberTarget::MillerColumn(_, _) => {
                let cur = self.rubber_content_point(target, current);
                (
                    f32::from(r.content_origin.x),
                    f32::from(r.content_origin.y),
                    f32::from(cur.x),
                    f32::from(cur.y),
                )
            }
            RubberTarget::Dropzone => (
                f32::from(origin.x),
                f32::from(origin.y),
                f32::from(current.x),
                f32::from(current.y),
            ),
        };
        let (rl, rr) = (ox.min(cx_), ox.max(cx_));
        let (rt, rb) = (oy.min(cy), oy.max(cy));
        match target {
            RubberTarget::Pane(pane) => {
                // List is laid out by AnimatedVirtualGrid. Its measured item bounds
                // are the single source of truth: the old arithmetic approximation
                // used a fixed 24px pitch, while List uses the actual line height
                // plus padding. That divergence grows one row at a time and makes a
                // marquee select rows above/below the rectangle (especially at zoom).
                let list_rect_hits = self
                    .tab_data_for(pane)
                    .is_some_and(|tab| tab.view_mode == ViewMode::List)
                    .then(|| {
                        let left = f32::from(origin.x).min(f32::from(current.x));
                        let right = f32::from(origin.x).max(f32::from(current.x));
                        let top = f32::from(origin.y).min(f32::from(current.y));
                        let bottom = f32::from(origin.y).max(f32::from(current.y));
                        self.row_rects
                            .borrow()
                            .get(&pane)
                            .filter(|rects| !rects.is_empty())
                            .map(|rects| {
                                rects
                                    .iter()
                                    .filter_map(|(idx, bounds)| {
                                        let (item_left, item_right) =
                                            (f32::from(bounds.left()), f32::from(bounds.right()));
                                        let (item_top, item_bottom) =
                                            (f32::from(bounds.top()), f32::from(bounds.bottom()));
                                        (left < item_right
                                            && item_left < right
                                            && top < item_bottom
                                            && item_top < bottom)
                                            .then_some(*idx)
                                    })
                                    .collect::<std::collections::HashSet<_>>()
                            })
                    })
                    .flatten();
                // Keep the content-space fallback for Details/Grid and for List's
                // first frame before prepaint has published its rectangles.
                let hits = list_rect_hits
                    .unwrap_or_else(|| self.rubber_content_hits_for_pane(pane, rl, rr, rt, rb));
                let Some(r) = self.rubber.as_mut() else {
                    return;
                };
                let mut sel: std::collections::HashSet<usize> = if r.additive {
                    r.base.clone()
                } else {
                    std::collections::HashSet::new()
                };
                sel.extend(hits);
                if let Some(ts) = self.state.tab_states.get_mut(&pane) {
                    ts.selection.selected = sel;
                }
            }
            RubberTarget::MillerColumn(pane, column) => {
                let Some(r) = self.rubber.as_mut() else {
                    return;
                };
                let mut sel: std::collections::HashSet<usize> = if r.additive {
                    r.base.clone()
                } else {
                    std::collections::HashSet::new()
                };
                sel.extend(self.rubber_content_hits_for_miller(pane, column, rt, rb));
                if let Some(col) = self
                    .state
                    .tab_states
                    .get_mut(&pane)
                    .and_then(|ts| ts.miller.columns.get_mut(column))
                {
                    col.selection.selected = sel;
                    col.selection.keyboard_cursor = col.selection.selected.iter().next().copied();
                }
            }
            RubberTarget::Dropzone => {
                let mut hits = std::collections::HashSet::new();
                for (path, b) in &self.dropzone_rects {
                    let (bl, br) = (f32::from(b.left()), f32::from(b.right()));
                    let (bt, bb) = (f32::from(b.top()), f32::from(b.bottom()));
                    if rl < br && bl < rr && rt < bb && bt < rb {
                        hits.insert(path.clone());
                    }
                }
                let Some(r) = self.rubber.as_ref() else {
                    return;
                };
                let mut sel: std::collections::HashSet<std::path::PathBuf> = if r.additive {
                    r.dropzone_base.clone()
                } else {
                    std::collections::HashSet::new()
                };
                sel.extend(hits);
                self.dropzone_selected = sel;
            }
        }
    }

    fn rubber_content_point(
        &self,
        target: RubberTarget,
        pos: gpui::Point<gpui::Pixels>,
    ) -> gpui::Point<gpui::Pixels> {
        let Some(bounds) = self.rubber_target_bounds(target) else {
            return pos;
        };
        let scroll_y = self
            .rubber_scroll_handle(target)
            .map(|(_, handle)| -f32::from(handle.offset().y))
            .unwrap_or(0.0);
        gpui::point(
            gpui::px(f32::from(pos.x) - f32::from(bounds.left())),
            gpui::px(f32::from(pos.y) - f32::from(bounds.top()) + scroll_y),
        )
    }

    fn rubber_window_point_from_content(
        &self,
        target: RubberTarget,
        pos: gpui::Point<gpui::Pixels>,
    ) -> gpui::Point<gpui::Pixels> {
        let Some(bounds) = self.rubber_target_bounds(target) else {
            return pos;
        };
        let scroll_y = self
            .rubber_scroll_handle(target)
            .map(|(_, handle)| -f32::from(handle.offset().y))
            .unwrap_or(0.0);
        gpui::point(
            gpui::px(f32::from(bounds.left()) + f32::from(pos.x)),
            gpui::px(f32::from(bounds.top()) + f32::from(pos.y) - scroll_y),
        )
    }

    fn rubber_content_hits_for_pane(
        &mut self,
        pane: TabId,
        rl: f32,
        rr: f32,
        rt: f32,
        rb: f32,
    ) -> std::collections::HashSet<usize> {
        let mut hits = std::collections::HashSet::new();
        let zoom = self.state.settings.global_zoom;
        let Some((mode, thumbnail_size)) = self
            .tab_data_for(pane)
            .map(|td| (td.view_mode.clone(), td.thumbnail_size))
        else {
            return hits;
        };
        let Some(ts) = self.state.tab_states.get_mut(&pane) else {
            return hits;
        };
        let visible = ts.visible_indices().to_vec();
        match mode {
            ViewMode::Details => {
                let row_h = crate::ui::views::details::DETAILS_ROW_H * zoom;
                for (pos, idx) in visible.into_iter().enumerate() {
                    let top = pos as f32 * row_h;
                    if rt < top + row_h && top < rb {
                        hits.insert(idx);
                    }
                }
            }
            ViewMode::Grid => {
                let gap = crate::ui::views::grid::GRID_GAP * zoom;
                let thumb = thumbnail_size * zoom;
                let cell_h = thumb * crate::ui::views::grid::GRID_ICON_AREA
                    + crate::ui::views::grid::GRID_LABEL_H * zoom;
                let pane_w = ts.last_pane_width.max(thumb + 1.0);
                let cols = (((pane_w + gap) / (thumb + gap)).floor() as usize).max(1);
                for (pos, idx) in visible.into_iter().enumerate() {
                    let row = pos / cols;
                    let col = pos % cols;
                    let left = col as f32 * (thumb + gap);
                    let top = row as f32 * (cell_h + gap);
                    if rl < left + thumb && left < rr && rt < top + cell_h && top < rb {
                        hits.insert(idx);
                    }
                }
            }
            ViewMode::List => {
                let cell_w = 220.0 * zoom;
                let gap = 6.0 * zoom;
                let stride = cell_w + gap;
                let avail = (ts.last_pane_width - 16.0 * zoom).max(stride);
                let cols = ((avail / stride).floor() as usize).max(1);
                let row_h = 24.0 * zoom;
                for (pos, idx) in visible.into_iter().enumerate() {
                    let row = pos / cols;
                    let col = pos % cols;
                    let left = 8.0 * zoom + col as f32 * stride;
                    let top = row as f32 * row_h;
                    if rl < left + cell_w && left < rr && rt < top + row_h && top < rb {
                        hits.insert(idx);
                    }
                }
            }
            ViewMode::Miller => {}
        }
        hits
    }

    fn rubber_content_hits_for_miller(
        &self,
        pane: TabId,
        column: usize,
        rt: f32,
        rb: f32,
    ) -> std::collections::HashSet<usize> {
        let mut hits = std::collections::HashSet::new();
        let row_h = 24.0 * self.state.settings.global_zoom;
        let Some(col) = self
            .state
            .tab_states
            .get(&pane)
            .and_then(|ts| ts.miller.columns.get(column))
        else {
            return hits;
        };
        for idx in 0..col.entries.len() {
            let top = idx as f32 * row_h;
            if rt < top + row_h && top < rb {
                hits.insert(idx);
            }
        }
        hits
    }

    fn rubber_scroll_handle(&self, target: RubberTarget) -> Option<(TabId, gpui::ScrollHandle)> {
        match target {
            RubberTarget::Pane(pane) => self
                .scroll_handles
                .get(&pane)
                .map(|handle| (pane, handle.0.borrow().base_handle.clone())),
            RubberTarget::MillerColumn(pane, column) => self
                .miller_scroll_handles
                .get(&(pane, column))
                .map(|handle| (pane, handle.0.borrow().base_handle.clone())),
            RubberTarget::Dropzone => None,
        }
    }

    fn rubber_scroll_pixels(&mut self, target: RubberTarget, delta_y: f32) -> bool {
        if delta_y.abs() < 0.5 {
            return false;
        }
        if !self.rubber_target_has_scrollable_content(target) {
            return false;
        }
        let Some((pane, handle)) = self.rubber_scroll_handle(target) else {
            return false;
        };
        let offset = handle.offset();
        let max = f32::from(handle.max_offset().y);
        let cur = f32::from(offset.y);
        let next = (cur + delta_y).clamp(-max, 0.0);
        if (next - cur).abs() < 0.5 {
            return false;
        }
        handle.set_offset(gpui::point(offset.x, gpui::px(next)));
        self.dirty_panes.insert(pane);
        true
    }

    fn rubber_target_has_scrollable_content(&self, target: RubberTarget) -> bool {
        match target {
            RubberTarget::Pane(pane) => self
                .state
                .tab_states
                .get(&pane)
                .is_some_and(|ts| !ts.visible.is_empty()),
            RubberTarget::MillerColumn(pane, column) => self
                .state
                .tab_states
                .get(&pane)
                .and_then(|ts| ts.miller.columns.get(column))
                .is_some_and(|column| !column.entries.is_empty()),
            RubberTarget::Dropzone => false,
        }
    }

    /// Rubber selection needs an explicit outward gesture. Entering an edge zone
    /// (or beginning a marquee there) is not enough to start scrolling.
    fn update_rubber_edge_intent(
        &mut self,
        previous: gpui::Point<gpui::Pixels>,
        current: gpui::Point<gpui::Pixels>,
    ) {
        const INTENT_DISTANCE: f32 = 4.0;
        let Some(rubber) = self.rubber.as_mut() else {
            return;
        };
        if !rubber.started {
            rubber.edge_intent = None;
            rubber.edge_intent_origin_y = None;
            return;
        }
        let Some(activation) = edge_autoscroll::activation_at(rubber.target_bounds, current) else {
            rubber.edge_intent = None;
            rubber.edge_intent_origin_y = None;
            return;
        };
        let direction = activation.direction;
        if rubber.edge_intent != Some(direction) && rubber.edge_intent_origin_y.is_none() {
            rubber.edge_intent_origin_y = Some(previous.y);
        }
        let origin = rubber.edge_intent_origin_y.unwrap_or(previous.y);
        let moved_from_origin = match direction {
            EdgeDirection::Up => f32::from(origin - current.y),
            EdgeDirection::Down => f32::from(current.y - origin),
        };
        let moved_this_event = match direction {
            EdgeDirection::Up => f32::from(previous.y - current.y),
            EdgeDirection::Down => f32::from(current.y - previous.y),
        };

        // A meaningful reversal disarms the stationary frame loop immediately.
        if rubber.edge_intent == Some(direction) && moved_this_event <= -INTENT_DISTANCE {
            rubber.edge_intent = None;
            rubber.edge_intent_origin_y = Some(current.y);
        } else if moved_from_origin >= INTENT_DISTANCE {
            rubber.edge_intent = Some(direction);
        } else if moved_from_origin < 0.0 {
            rubber.edge_intent = None;
            rubber.edge_intent_origin_y = Some(current.y);
        }
    }

    /// Advance one RubberSelect frame after directional intent has armed it.
    /// The selection is recomputed after every real offset change, so a stationary
    /// cursor still selects rows continuously while the viewport moves.
    fn tick_rubber_edge_autoscroll(&mut self, now: Instant, cx: &mut Context<Self>) -> bool {
        let Some((target, bounds, current, intent)) = self.rubber.as_ref().map(|rubber| {
            (
                rubber.target,
                rubber.target_bounds,
                rubber.current,
                rubber.edge_intent,
            )
        }) else {
            return false;
        };
        let Some(activation) = edge_autoscroll::activation_at(bounds, current) else {
            return false;
        };
        if intent != Some(activation.direction)
            || !self.rubber_target_has_scrollable_content(target)
        {
            return false;
        }
        let elapsed = self
            .rubber
            .as_mut()
            .and_then(|rubber| rubber.last_scroll_frame.replace(now))
            .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32());
        let Some((pane, handle)) = self.rubber_scroll_handle(target) else {
            return false;
        };
        if !edge_autoscroll::tick_handle(&handle, activation, elapsed) {
            return false;
        }
        self.dirty_panes.insert(pane);
        self.apply_rubber();
        self.notify_pane(pane, cx);
        cx.notify();
        true
    }

    fn rubber_wheel_delta(ev: &ScrollWheelEvent) -> f32 {
        match ev.delta {
            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
            gpui::ScrollDelta::Lines(l) => l.y * 32.0,
        }
    }

    pub(crate) fn scroll_grid_row_to(
        &mut self,
        pane: TabId,
        row: usize,
        strategy: gpui::ScrollStrategy,
    ) -> bool {
        let Some(Tile::Pane(pane_data)) = self.state.pane_tree.tiles.get(pane) else {
            return false;
        };
        if !matches!(pane_data.view_mode, ViewMode::Grid) {
            return false;
        }
        let Some(handle) = self.scroll_handles.get(&pane) else {
            return false;
        };

        let row_pitch = pane_data.thumbnail_size * crate::ui::views::grid::GRID_ICON_AREA
            + crate::ui::views::grid::GRID_LABEL_H
            + crate::ui::views::grid::GRID_GAP;
        let (_, row_count) = self.rubber_scroll_dims(pane);
        if row_count == 0 {
            return true;
        }

        let base = handle.0.borrow().base_handle.clone();
        let viewport_h = f32::from(base.bounds().size.height);
        if viewport_h <= 1.0 {
            return false;
        }

        let row_top = row as f32 * row_pitch;
        let row_bottom = row_top + row_pitch;
        let content_h = row_count as f32 * row_pitch - crate::ui::views::grid::GRID_GAP;
        let max_scroll = (content_h - viewport_h).max(0.0);
        let offset = base.offset();
        let current_top = (-f32::from(offset.y)).max(0.0);
        let target = match strategy {
            gpui::ScrollStrategy::Top => row_top,
            gpui::ScrollStrategy::Center => row_top - (viewport_h - row_pitch) * 0.5,
            gpui::ScrollStrategy::Bottom => row_bottom - viewport_h,
            gpui::ScrollStrategy::Nearest => {
                if row_top < current_top {
                    row_top
                } else if row_bottom > current_top + viewport_h {
                    row_bottom - viewport_h
                } else {
                    current_top
                }
            }
        }
        .clamp(0.0, max_scroll);

        base.set_offset(gpui::point(offset.x, gpui::px(-target)));
        true
    }

    pub(crate) fn scroll_details_row_to(
        &mut self,
        pane: TabId,
        row: usize,
        strategy: gpui::ScrollStrategy,
    ) -> bool {
        let Some(Tile::Pane(pane_data)) = self.state.pane_tree.tiles.get(pane) else {
            return false;
        };
        if !matches!(pane_data.view_mode, ViewMode::Details) {
            return false;
        }
        let Some(handle) = self.scroll_handles.get(&pane) else {
            return false;
        };

        let row_pitch = crate::ui::views::details::DETAILS_ROW_H;
        let row_count = self
            .state
            .tab_states
            .get(&pane)
            .map(|ts| ts.visible.len())
            .unwrap_or(0);
        if row_count == 0 {
            return true;
        }

        let base = handle.0.borrow().base_handle.clone();
        let viewport_h = f32::from(base.bounds().size.height);
        if viewport_h <= 1.0 {
            return false;
        }

        let row_top = row as f32 * row_pitch;
        let row_bottom = row_top + row_pitch;
        let content_h = row_count as f32 * row_pitch;
        let max_scroll = (content_h - viewport_h).max(0.0);
        let offset = base.offset();
        let current_top = (-f32::from(offset.y)).max(0.0);
        let target = match strategy {
            gpui::ScrollStrategy::Top => row_top,
            gpui::ScrollStrategy::Center => row_top - (viewport_h - row_pitch) * 0.5,
            gpui::ScrollStrategy::Bottom => row_bottom - viewport_h,
            gpui::ScrollStrategy::Nearest => {
                if row_top < current_top {
                    row_top
                } else if row_bottom > current_top + viewport_h {
                    row_bottom - viewport_h
                } else {
                    current_top
                }
            }
        }
        .clamp(0.0, max_scroll);

        base.set_offset(gpui::point(offset.x, gpui::px(-target)));
        true
    }

    pub(crate) fn scroll_selected_into_view(
        &mut self,
        pane: TabId,
        strategy: gpui::ScrollStrategy,
    ) -> bool {
        let Some((visible_pos, mode)) = (|| {
            let ts = self.state.tab_states.get_mut(&pane)?;
            let cursor = ts
                .selection
                .keyboard_cursor
                .or_else(|| ts.selection.selected.iter().next().copied())?;
            let visible_pos = ts.visible_indices().iter().position(|idx| *idx == cursor)?;
            let mode = self.tab_data_for(pane)?.view_mode.clone();
            Some((visible_pos, mode))
        })() else {
            return false;
        };

        match mode {
            ViewMode::Grid => {
                let (cols, _) = self.rubber_scroll_dims(pane);
                self.scroll_grid_row_to(pane, visible_pos / cols.max(1), strategy)
            }
            ViewMode::List => {
                let (cols, _) = self.rubber_scroll_dims(pane);
                if let Some(handle) = self.scroll_handles.get(&pane) {
                    handle.scroll_to_item(visible_pos / cols.max(1), strategy);
                    true
                } else {
                    false
                }
            }
            ViewMode::Details => self.scroll_details_row_to(pane, visible_pos, strategy),
            ViewMode::Miller => false,
        }
    }

    /// Begin inline rename of the focused pane's cursor item: pre-fill a native input
    /// with its current name and stash it in `rename_edit`.
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.state.focused_tile;
        let view_mode = self
            .tab_data_for(pane)
            .map(|tab| tab.view_mode.clone())
            .unwrap_or_default();
        let Some((idx, visible_pos, path, name, initial_select_chars)) = (|| {
            let ts = self.state.tab_states.get_mut(&pane)?;
            if matches!(view_mode, ViewMode::Miller) {
                let col = ts.miller.columns.get(ts.miller.focused)?;
                let idx = col
                    .selection
                    .keyboard_cursor
                    .or_else(|| col.selection.selected.iter().copied().next())?;
                let entry = col.entries.get(idx)?;
                let stem_chars = rename_select_chars(&entry.name);
                return Some((idx, idx, entry.path.clone(), entry.name.clone(), stem_chars));
            }
            let idx = ts
                .selection
                .keyboard_cursor
                .or_else(|| ts.selection.selected.iter().copied().next())?;
            let visible_pos = ts
                .visible_indices()
                .iter()
                .position(|&visible| visible == idx)?;
            let entry = ts.entries.get(idx)?;
            let stem_chars = rename_select_chars(&entry.name);
            Some((
                idx,
                visible_pos,
                entry.path.clone(),
                entry.name.clone(),
                stem_chars,
            ))
        })() else {
            return;
        };

        self.close_batch_rename();
        self.rename_visual_suppress = None;
        match view_mode {
            ViewMode::Grid => {
                let (cols, _) = self.rubber_scroll_dims(pane);
                self.scroll_grid_row_to(
                    pane,
                    visible_pos / cols.max(1),
                    gpui::ScrollStrategy::Nearest,
                );
            }
            ViewMode::Details => {
                self.scroll_details_row_to(pane, visible_pos, gpui::ScrollStrategy::Nearest);
            }
            ViewMode::List => {
                let (cols, _) = self.rubber_scroll_dims(pane);
                if let Some(handle) = self.scroll_handles.get(&pane) {
                    handle.scroll_to_item(visible_pos / cols.max(1), gpui::ScrollStrategy::Nearest);
                }
            }
            ViewMode::Miller => {}
        }

        let input = crate::ui::text_input::single_line(window, cx, "", name);
        input.update(cx, |state, cx| state.focus(window, cx));
        self.rename_edit = Some(crate::app::RenameEdit {
            pane,
            idx,
            path,
            input,
            initial_select_chars,
        });
    }

    /// Confirm the inline rename: validate the new name (cross-platform: reject path
    /// separators and the `.`/`..` specials) and dispatch the FS rename.
    pub(crate) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        self.commit_rename_inner(true, cx);
    }

    fn commit_rename_inner(&mut self, select_target: bool, cx: &mut Context<Self>) {
        let Some(edit) = self.rename_edit.take() else {
            return;
        };
        self.rename_visual_suppress = Some((edit.pane, edit.idx, 2));
        let new_name = edit.input.read(cx).value().to_string();
        let new_name = new_name.trim();
        if !atlas_model::batch_rename::name_is_valid(new_name) {
            return;
        }
        if let Some(parent) = edit.path.parent() {
            let to = parent.join(new_name);
            if to != edit.path {
                if select_target {
                    self.pending_select = Some((edit.pane, to.clone()));
                }
                self.state.focused_tile = edit.pane;
                self.process_actions(vec![Action::Rename {
                    from: edit.path,
                    to,
                }]);
            } else {
                self.state.focused_tile = edit.pane;
            }
        }
    }

    pub(crate) fn commit_rename_and_move(
        &mut self,
        forward: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pane, idx)) = self.rename_edit.as_ref().map(|edit| (edit.pane, edit.idx)) else {
            return;
        };
        let next = self.rename_neighbor(pane, idx, forward);
        self.commit_rename_inner(false, cx);
        let Some((next_idx, next_path)) = next else {
            cx.notify();
            return;
        };
        self.select_rename_target(pane, next_idx);
        self.pending_rename = Some((pane, next_path, false));
        cx.notify();
    }

    pub(crate) fn rename_next_action(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_rename_and_move(true, window, cx);
        cx.stop_propagation();
    }

    pub(crate) fn rename_previous_action(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_rename_and_move(false, window, cx);
        cx.stop_propagation();
    }

    fn rename_neighbor(
        &mut self,
        pane: TabId,
        idx: usize,
        forward: bool,
    ) -> Option<(usize, std::path::PathBuf)> {
        let view_mode = self.tab_data_for(pane)?.view_mode.clone();
        let ts = self.state.tab_states.get_mut(&pane)?;
        if matches!(view_mode, ViewMode::Miller) {
            let col = ts.miller.columns.get(ts.miller.focused)?;
            let next_idx = neighbor_index(idx, col.entries.len(), forward)?;
            let path = col.entries.get(next_idx)?.path.clone();
            return Some((next_idx, path));
        }

        let visible = ts.visible_indices().to_vec();
        let pos = visible.iter().position(|&visible_idx| visible_idx == idx)?;
        let next_pos = neighbor_index(pos, visible.len(), forward)?;
        let next_idx = visible[next_pos];
        let path = ts.entries.get(next_idx)?.path.clone();
        Some((next_idx, path))
    }

    fn select_rename_target(&mut self, pane: TabId, idx: usize) {
        let view_mode = self.tab_data_for(pane).map(|tab| tab.view_mode.clone());
        let Some(ts) = self.state.tab_states.get_mut(&pane) else {
            return;
        };
        if matches!(view_mode, Some(ViewMode::Miller)) {
            if let Some(col) = ts.miller.columns.get_mut(ts.miller.focused) {
                col.selection.selected.clear();
                col.selection.selected.insert(idx);
                col.selection.keyboard_cursor = Some(idx);
                col.selection.selection_anchor = Some(idx);
                col.selection.last_clicked_index = Some(idx);
            }
        } else {
            ts.selection.selected.clear();
            ts.selection.selected.insert(idx);
            ts.selection.keyboard_cursor = Some(idx);
            ts.selection.selection_anchor = Some(idx);
            ts.selection.last_clicked_index = Some(idx);
            ts.selection.scroll_to_selected = false;
        }
        self.activate_tab(pane);
    }

    /// Cancel the inline rename without touching the filesystem.
    pub(crate) fn cancel_rename(&mut self) {
        if let Some(edit) = self.rename_edit.take() {
            self.rename_visual_suppress = Some((edit.pane, edit.idx, 2));
        }
    }

    /// Cancel an in-flight rubber drag, restoring the pre-drag selection.
    pub(crate) fn cancel_rubber(&mut self) {
        if let Some(r) = self.rubber.take() {
            match r.target {
                RubberTarget::Pane(pane) => {
                    if let Some(ts) = self.state.tab_states.get_mut(&pane) {
                        ts.selection.selected = r.base;
                    }
                }
                RubberTarget::MillerColumn(pane, column) => {
                    if let Some(col) = self
                        .state
                        .tab_states
                        .get_mut(&pane)
                        .and_then(|ts| ts.miller.columns.get_mut(column))
                    {
                        col.selection.selected = r.base;
                    }
                }
                RubberTarget::Dropzone => {
                    self.dropzone_selected = r.dropzone_base;
                }
            }
        }
    }

    /// Phase C: start a rubber-band drag in `pane` at `pos` (window coords), capturing
    /// the current selection as the additive base. Shared by the pane's empty area and
    /// the Details "dead zone" column so the gesture begins the same way from either.
    pub(crate) fn begin_rubber(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
        additive: bool,
    ) {
        // Context menu open: mousedown belongs to the menu, not rubber.
        if self.ctx_menu.is_some() {
            return;
        }
        // Miller has per-column selection — rubber band doesn't map onto it.
        if self
            .tab_data_for(pane)
            .is_some_and(|td| matches!(td.view_mode, ViewMode::Miller))
        {
            return;
        }
        // Clicking the empty list area dismisses a filter that has no text yet
        // (the "click outside to leave the field" affordance); a filter with a
        // query stays applied so the click can refine the selection instead.
        if self
            .state
            .tab_states
            .get(&pane)
            .is_some_and(|ts| ts.search.active && ts.search.query.trim().is_empty())
        {
            self.close_search_in_place(pane);
        }
        let base = self
            .state
            .tab_states
            .get(&pane)
            .map(|ts| ts.selection.selected.clone())
            .unwrap_or_default();
        let target = RubberTarget::Pane(pane);
        let Some(target_bounds) = self.rubber_target_bounds(target) else {
            return;
        };
        let origin = Self::clamp_point_to_bounds(target_bounds, pos);
        self.rubber = Some(crate::app::RubberDrag {
            target,
            target_bounds,
            origin,
            current: origin,
            content_origin: self.rubber_content_point(target, origin),
            base,
            dropzone_base: Default::default(),
            additive,
            started: false,
            edge_intent: None,
            edge_intent_origin_y: None,
            last_scroll_frame: None,
        });
    }

    pub(crate) fn begin_miller_rubber(
        &mut self,
        pane: TabId,
        column: usize,
        pos: gpui::Point<gpui::Pixels>,
        additive: bool,
    ) {
        if self.ctx_menu.is_some() {
            return;
        }
        self.activate_tab(pane);
        let base = self
            .state
            .tab_states
            .get_mut(&pane)
            .and_then(|ts| {
                ts.miller.focused = column;
                ts.miller.columns.get(column)
            })
            .map(|col| col.selection.selected.clone())
            .unwrap_or_default();
        let target = RubberTarget::MillerColumn(pane, column);
        let Some(target_bounds) = self.rubber_target_bounds(target) else {
            return;
        };
        let origin = Self::clamp_point_to_bounds(target_bounds, pos);
        self.rubber = Some(crate::app::RubberDrag {
            target,
            target_bounds,
            origin,
            current: origin,
            content_origin: self.rubber_content_point(target, origin),
            base,
            dropzone_base: Default::default(),
            additive,
            started: false,
            edge_intent: None,
            edge_intent_origin_y: None,
            last_scroll_frame: None,
        });
    }

    pub(crate) fn begin_dropzone_rubber(&mut self, pos: gpui::Point<gpui::Pixels>, additive: bool) {
        if self.ctx_menu.is_some() {
            return;
        }
        let staged = self
            .state
            .sidebar_drop_zone
            .iter()
            .map(|item| item.path.clone())
            .collect::<std::collections::HashSet<_>>();
        self.dropzone_selected.retain(|path| staged.contains(path));
        let dropzone_base = self.dropzone_selected.clone();
        if !additive {
            self.dropzone_selected.clear();
        }
        let target = RubberTarget::Dropzone;
        let Some(target_bounds) = self.rubber_target_bounds(target) else {
            return;
        };
        let origin = Self::clamp_point_to_bounds(target_bounds, pos);
        self.rubber = Some(crate::app::RubberDrag {
            target,
            target_bounds,
            origin,
            current: origin,
            content_origin: origin,
            base: Default::default(),
            dropzone_base,
            additive,
            started: false,
            edge_intent: None,
            edge_intent_origin_y: None,
            last_scroll_frame: None,
        });
    }

    /// True when any of the app's text inputs (overlay palettes, renames, the
    /// per-pane filter) currently holds keyboard focus — typing then belongs to
    /// that field, never to type-to-filter.
    /// True when the keyboard belongs to a text field or a managed modal, so the
    /// window-level type-to-filter must not steal the key. Uses gpui-component's
    /// own focus bookkeeping (`has_focused_input`) instead of a hand-maintained
    /// list, so it automatically covers every overlay's input — the app fields,
    /// the right-click menu's search box, and `Dialog`/`Sheet` inputs alike.
    /// (The old hardcoded list missed the context-menu and dialog inputs, which
    /// let the pane filter swallow their typing.)
    pub(crate) fn any_text_input_focused(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        window.has_focused_input(cx) || window.has_active_dialog(cx) || window.has_active_sheet(cx)
    }

    /// Window-level type-to-filter: a plain character (no modifiers, not space)
    /// typed while no text field is focused opens — or extends — the focused
    /// pane's local fuzzy filter.
    pub(crate) fn handle_global_type_to_filter(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(seed) = search_seed_from_key(ev) else {
            return;
        };
        if self.any_text_input_focused(window, cx) {
            return;
        }
        let o = &self.state.overlay;
        if o.show_goto_bar
            || o.show_palette
            || o.show_settings
            || o.show_ext_palette
            || o.show_analyzer
            || o.transfer_conflict.is_some()
            || self.state.batch_rename.is_some()
            || self.rename_edit.is_some()
        {
            return;
        }
        let pane_id = self.state.focused_tile;
        if !matches!(self.state.pane_tree.tiles.get(pane_id), Some(Tile::Pane(_))) {
            return;
        }
        if self
            .state
            .tab_states
            .get(&pane_id)
            .is_some_and(|ts| ts.path_edit.editing)
        {
            return;
        }

        let active = self
            .state
            .tab_states
            .get(&pane_id)
            .is_some_and(|ts| ts.search.active);
        if active {
            // Filter open but unfocused: append the char and refocus the field.
            if let Some(input) = self.search_inputs.get(&pane_id).cloned() {
                let new_value = format!("{}{}", input.read(cx).value(), seed);
                input.update(cx, |state, cx| state.set_value(new_value, window, cx));
                let focus = input.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
                ts.search.focus = true;
            }
        } else {
            self.open_search_in_place_seeded(pane_id, seed, window, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn open_search_in_place_seeded(
        &mut self,
        pane_id: TabId,
        seed: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.focused_tile = pane_id;
        let input = self
            .search_inputs
            .entry(pane_id)
            .or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", ""))
            .clone();
        input.update(cx, |state, cx| state.set_value(seed.clone(), window, cx));
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
            ts.search.active = true;
            ts.search.focus = true;
            ts.search.query = seed;
        }
        self.refresh_search_in_place(pane_id);
    }

    /// Ctrl+scroll on a pane: in Grid it zooms the thumbnail size; in every other
    /// view it cycles to the next/previous view mode.
    pub(crate) fn pane_ctrl_scroll(&mut self, pane_id: TabId, dy: f32) {
        let Some((mode, thumbnail_size)) = self
            .tab_data_for(pane_id)
            .map(|t| (t.view_mode.clone(), t.thumbnail_size))
        else {
            return;
        };
        if mode == ViewMode::Miller {
            return; // don't cycle out of Miller
        }
        let step = if dy > 0.0 { 1.0 } else { -1.0 };
        let value = Self::slider_value_for_view(&mode, thumbnail_size);
        self.set_pane_view_from_slider_value(pane_id, value + step);
    }
}

fn rename_select_chars(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let compound_exts = [
        ".tar.gz",
        ".tar.bz2",
        ".tar.xz",
        ".tar.zst",
        ".tar.lz",
        ".tar.lzma",
        ".tar.br",
        ".tar.z",
        ".d.ts",
        ".d.mts",
        ".d.cts",
        ".js.map",
        ".css.map",
        ".min.js",
        ".min.css",
    ];
    let stem = compound_exts
        .iter()
        .find_map(|ext| name.strip_suffix(ext).or_else(|| lower.strip_suffix(ext)))
        .or_else(|| {
            let dot = name.rfind('.')?;
            (dot > 0).then(|| &name[..dot])
        })?;
    let chars = stem.chars().count() as u32;
    (chars > 0).then_some(chars)
}

#[cfg(test)]
mod tests {
    use super::rename_select_chars;

    #[test]
    fn rename_selects_name_before_extension() {
        assert_eq!(rename_select_chars("photo.png"), Some(5));
        assert_eq!(rename_select_chars("archive.tar.gz"), Some(7));
        assert_eq!(rename_select_chars("ARCHIVE.TAR.GZ"), Some(7));
        assert_eq!(rename_select_chars("types.d.ts"), Some(5));
        assert_eq!(rename_select_chars("app.min.js"), Some(3));
        assert_eq!(rename_select_chars("bundle.js.map"), Some(6));
        assert_eq!(rename_select_chars("README"), None);
        assert_eq!(rename_select_chars(".gitignore"), None);
    }

    #[test]
    fn slider_values_order_list_details_grid() {
        use atlas_model::settings::ViewMode;
        use crate::app::App;

        assert_eq!(App::slider_value_for_view(&ViewMode::List, 64.0), 0.0);
        assert_eq!(App::slider_value_for_view(&ViewMode::Details, 64.0), 1.0);
        assert_eq!(App::slider_value_for_view(&ViewMode::Grid, 64.0), 2.0);
        assert_eq!(App::slider_value_for_view(&ViewMode::Grid, 384.0), 22.0);
    }
}

/// Global rubber-band motor: a full-size invisible canvas painted last so its
/// `window.on_mouse_event` listeners fire even when the cursor leaves the pane.
/// Handles move (threshold → started, apply), up (clear on empty click), and
/// drives `request_animation_frame` while rubber is active for continuous autoscroll.
pub(crate) fn rubber_layer(view: Entity<App>) -> impl gpui::IntoElement {
    let v = view.clone();
    canvas(
        |_, _, _| {},
        move |_, (), window, cx| {
            // ── Draw the rubber band marquee rectangle ───────────────────────
            if let Some(r) = v.read(cx).rubber.as_ref() {
                let app = v.read(cx);
                let origin = match r.target {
                    RubberTarget::Pane(_) | RubberTarget::MillerColumn(_, _) => app
                        .clamp_rubber_target_point(
                            r.target,
                            app.rubber_window_point_from_content(r.target, r.content_origin),
                        ),
                    RubberTarget::Dropzone => app.clamp_rubber_target_point(r.target, r.origin),
                };
                let current = app.clamp_rubber_target_point(r.target, r.current);
                let left = f32::from(origin.x).min(f32::from(current.x));
                let top = f32::from(origin.y).min(f32::from(current.y));
                let right = f32::from(origin.x).max(f32::from(current.x));
                let bottom = f32::from(origin.y).max(f32::from(current.y));
                let w = (right - left).max(0.0);
                let h = (bottom - top).max(0.0);

                if r.started && w >= 2.0 && h >= 2.0 {
                    let rect = gpui::Bounds {
                        origin: gpui::point(gpui::px(left), gpui::px(top)),
                        size: gpui::size(gpui::px(w), gpui::px(h)),
                    };
                    let t = cx.theme();
                    let mut fill = t.accent;
                    fill.alpha = 0.15;

                    window.paint_quad(gpui::fill(rect, fill));

                    let mut border = t.accent;
                    border.alpha = 0.8;
                    let t_px = gpui::px(1.0);
                    window.paint_quad(gpui::fill(
                        gpui::Bounds {
                            origin: rect.origin,
                            size: gpui::size(rect.size.width, t_px),
                        },
                        border,
                    ));
                    window.paint_quad(gpui::fill(
                        gpui::Bounds {
                            origin: gpui::point(
                                rect.origin.x,
                                rect.origin.y + rect.size.height - t_px,
                            ),
                            size: gpui::size(rect.size.width, t_px),
                        },
                        border,
                    ));
                    window.paint_quad(gpui::fill(
                        gpui::Bounds {
                            origin: rect.origin,
                            size: gpui::size(t_px, rect.size.height),
                        },
                        border,
                    ));
                    window.paint_quad(gpui::fill(
                        gpui::Bounds {
                            origin: gpui::point(
                                rect.origin.x + rect.size.width - t_px,
                                rect.origin.y,
                            ),
                            size: gpui::size(t_px, rect.size.height),
                        },
                        border,
                    ));
                }
            }

            // A rubber drag must keep progressing while the pointer is still.
            // This runs from paint (where requesting the next frame is legal),
            // rather than relying on further MouseMove events.
            let rubber_scrolled = v.update(cx, |this, cx| {
                this.tick_rubber_edge_autoscroll(Instant::now(), cx)
            });
            if rubber_scrolled {
                crate::perf::event(|| "request_animation_frame rubber_autoscroll".to_string());
                window.request_animation_frame();
            }

            // ── Global move ──────────────────────────────────────────────────
            let vm = v.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vm.update(cx, |this, cx| {
                    if this.rubber.is_some() && !ev.dragging() {
                        this.rubber = None;
                        cx.notify();
                        return false;
                    }

                    if let Some(resize) = this.sidebar_resize.as_ref() {
                        let dx = f32::from(pos.x) - resize.start_x;
                        this.state.settings.sidebar_width =
                            (resize.start_width + dx).clamp(180.0, 420.0);
                        cx.notify();
                        return false;
                    }

                    if let Some(resize) = this.inspector_resize.as_ref() {
                        let dx = resize.start_x - f32::from(pos.x);
                        this.state.settings.inspector_width =
                            (resize.start_width + dx).clamp(180.0, 2000.0);
                        cx.notify();
                        return false;
                    }

                    if let Some(resize) = this.dock_resize.as_ref() {
                        let delta = match resize.dir {
                            atlas_model::layout_tree::LinearDir::Horizontal => {
                                f32::from(pos.x) - resize.start_pos
                            }
                            atlas_model::layout_tree::LinearDir::Vertical => {
                                f32::from(pos.y) - resize.start_pos
                            }
                        };
                        if resize.span_px > 1.0 {
                            let mut shares = resize.start_shares.clone();
                            let left = resize.divider;
                            let right = left + 1;
                            if right < shares.len() {
                                let total_share = shares[left] + shares[right];
                                let share_delta = delta / resize.span_px
                                    * shares.iter().copied().sum::<f32>().max(1.0);
                                let min_share = 0.1;
                                if total_share <= min_share * 2.0 {
                                    shares[left] = total_share / 2.0;
                                } else {
                                    shares[left] = (resize.start_shares[left] + share_delta)
                                        .clamp(min_share, total_share - min_share);
                                }
                                shares[right] = (total_share - shares[left]).max(0.0);
                                if let Some(atlas_model::layout_tree::Tile::Container(
                                    atlas_model::layout_tree::Container::Linear(linear),
                                )) = this.state.pane_tree.tiles.get_mut(resize.container)
                                {
                                    crate::app::normalize_split_shares(&mut shares);
                                    linear.shares = shares;
                                }
                                cx.notify();
                            }
                        }
                        return false;
                    }

                    // Column resize takes priority over rubber-band selection.
                    if let Some((col, pane, start_x, start_width)) = this
                        .col_resize
                        .as_ref()
                        .map(|cr| (cr.col, cr.pane, cr.start_x, cr.start_width))
                    {
                        // Left-anchored columns with the handle on the RIGHT edge:
                        // dragging right widens the column (Explorer-style).
                        let dx = f32::from(pos.x) - start_x;
                        let min_w = match col {
                            "name" => 120.0,
                            "size" => 46.0,
                            "modified" => 92.0,
                            "type" => 56.0,
                            _ => 35.0,
                        };
                        let new_w = (start_width + dx).max(min_w);
                        match col {
                            "name" => this.col_widths.name = new_w,
                            "size" => this.col_widths.size = new_w,
                            "modified" => this.col_widths.modified = new_w,
                            "type" => this.col_widths.type_w = new_w,
                            _ => {}
                        }
                        // Scoped re-render: only the dragged pane repaints per move —
                        // NOT App + sidebar + every other pane (that cascade is the lag).
                        this.notify_pane(pane, cx);
                        return false;
                    }

                    let Some((previous, started, target)) = this.rubber.as_mut().map(|r| {
                        let previous = r.current;
                        r.current = pos;
                        if !r.started {
                            let dx = f32::from(pos.x) - f32::from(r.origin.x);
                            let dy = f32::from(pos.y) - f32::from(r.origin.y);
                            r.started = dx * dx + dy * dy >= 16.0;
                        }
                        (previous, r.started, r.target)
                    }) else {
                        return false;
                    };
                    if started {
                        this.update_rubber_edge_intent(previous, pos);
                        this.apply_rubber();
                        let scrolled = this.tick_rubber_edge_autoscroll(Instant::now(), cx);
                        let pane = match target {
                            RubberTarget::Pane(pane) | RubberTarget::MillerColumn(pane, _) => {
                                Some(pane)
                            }
                            RubberTarget::Dropzone => None,
                        };
                        if let Some(pane) = pane {
                            this.notify_pane(pane, cx);
                        }
                        cx.notify();
                        return scrolled;
                    }
                    false
                });
                // Only force a full-window repaint while an interaction needs
                // cross-pane visual feedback (DnD drop zones, drag-hover, rubber-band).
                // Plain hover/move relies on gpui's own hover repaints and the scoped
                // `notify_pane` calls above; refreshing on every mouse move re-rendered
                // the entire App (all panes) and was the lag under many panes — and it
                // silently undid the scoped col/dock-resize notifies.
                let needs_full = cx.has_active_drag() || {
                    let app = vm.read(cx);
                    app.drag_hover_maybe_active() || app.rubber.is_some()
                };
                if needs_full {
                    window.refresh();
                }
            });

            let vw = v.clone();
            window.on_mouse_event::<ScrollWheelEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                {
                    let app = vw.read(cx);
                    if app.rubber.is_none() || app.state.settings.scroll_smoothness > 0.0 {
                        return;
                    }
                }
                let delta_y = App::rubber_wheel_delta(ev);
                if delta_y.abs() < 0.5 {
                    return;
                }
                vw.update(cx, |this, cx| {
                    let Some(target) = this.rubber.as_ref().map(|r| r.target) else {
                        return;
                    };
                    if this.rubber_scroll_pixels(target, delta_y) {
                        this.apply_rubber();
                        if let Some(pane) = match target {
                            RubberTarget::Pane(pane) | RubberTarget::MillerColumn(pane, _) => {
                                Some(pane)
                            }
                            RubberTarget::Dropzone => None,
                        } {
                            this.notify_pane(pane, cx);
                        }
                        cx.notify();
                        window.refresh();
                    }
                });
                cx.stop_propagation();
            });

            // ── Global up ────────────────────────────────────────────────────
            let vu = v.clone();
            window.on_mouse_event::<MouseUpEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vu.update(cx, |this, cx| {
                    if this.sidebar_resize.take().is_some() {
                        atlas_core::util::config::save_settings(&this.state.settings);
                        cx.notify();
                        return;
                    }

                    if this.inspector_resize.take().is_some() {
                        atlas_core::util::config::save_settings(&this.state.settings);
                        cx.notify();
                        return;
                    }

                    // End column resize — persist the new widths and let every pane
                    // (other Details views) catch up to the final widths.
                    if this.dock_resize.take().is_some() {
                        cx.notify();
                        return;
                    }

                    if this.col_resize.take().is_some() {
                        this.persist_details_columns();
                        this.notify_all_panes(cx);
                        return;
                    }

                    // End a column drag-reorder (fallback when the drop didn't land on
                    // the header — e.g. released over the rows or outside).
                    if this.col_dragging.is_some() {
                        this.finish_details_reorder();
                        this.notify_all_panes(cx);
                        return;
                    }

                    if let Some(r) = this.rubber.take() {
                        if !r.started && !r.additive {
                            // Plain click on empty space → clear selection.
                            // If pos hit an item rect, that item's on_click already handled it.
                            match r.target {
                                RubberTarget::Pane(pane) => {
                                    let hit = this
                                        .details_hit
                                        .get(&pane)
                                        .and_then(|model| model.hit_entry(pos))
                                        .is_some()
                                        || this.row_rects.borrow().get(&pane).is_some_and(
                                            |rects| {
                                                rects.iter().any(|(_, b)| {
                                                    pos.x >= b.left()
                                                        && pos.x <= b.right()
                                                        && pos.y >= b.top()
                                                        && pos.y <= b.bottom()
                                                })
                                            },
                                        );
                                    if !hit {
                                        if let Some(ts) = this.state.tab_states.get_mut(&pane) {
                                            ts.selection.clear();
                                        }
                                    }
                                }
                                RubberTarget::MillerColumn(pane, column) => {
                                    let hit = this
                                        .miller_row_rects
                                        .borrow()
                                        .get(&(pane, column))
                                        .is_some_and(|rects| {
                                            rects.iter().any(|(_, b)| {
                                                pos.x >= b.left()
                                                    && pos.x <= b.right()
                                                    && pos.y >= b.top()
                                                    && pos.y <= b.bottom()
                                            })
                                        });
                                    if !hit {
                                        if let Some(col) = this
                                            .state
                                            .tab_states
                                            .get_mut(&pane)
                                            .and_then(|ts| ts.miller.columns.get_mut(column))
                                        {
                                            col.selection.clear();
                                        }
                                    }
                                }
                                RubberTarget::Dropzone => {
                                    let hit = this.dropzone_rects.iter().any(|(_, b)| {
                                        pos.x >= b.left()
                                            && pos.x <= b.right()
                                            && pos.y >= b.top()
                                            && pos.y <= b.bottom()
                                    });
                                    if !hit {
                                        this.dropzone_selected.clear();
                                    }
                                }
                            }
                        }
                        cx.notify();
                    }
                });
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Capture Ctrl+wheel over file panes before GPUI's scrollables see it. Bubble
/// handlers on `uniform_list`/`overflow_y_scroll` are too late if registered
/// behind the internal scroll listener.
pub(crate) fn ctrl_scroll_layer(view: Entity<App>) -> impl gpui::IntoElement {
    canvas(
        |_, _, _| {},
        move |_, (), window, _cx| {
            let v = view.clone();
            window.on_mouse_event::<ScrollWheelEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || !(ev.modifiers.control || ev.modifiers.platform)
                {
                    return;
                }
                let dy = App::rubber_wheel_delta(ev);
                if dy == 0.0 {
                    return;
                }

                let pane = {
                    let app = v.read(cx);
                    let overlay = &app.state.overlay;
                    if overlay.show_goto_bar
                        || overlay.show_palette
                        || overlay.show_settings
                        || overlay.show_ext_palette
                        || overlay.show_analyzer
                        || overlay.transfer_conflict.is_some()
                        || app.state.batch_rename.is_some()
                        || app.rename_edit.is_some()
                    {
                        return;
                    }
                    app.pane_view_bounds.iter().find_map(|(pane, bounds)| {
                        let handles_ctrl_scroll = app
                            .tab_data_for(*pane)
                            .is_some_and(|tab| tab.view_mode != ViewMode::Miller);
                        (handles_ctrl_scroll && bounds.contains(&ev.position)).then_some(*pane)
                    })
                };

                let Some(pane) = pane else {
                    return;
                };
                v.update(cx, |this, cx| {
                    this.pane_ctrl_scroll(pane, dy);
                    this.notify_pane(pane, cx);
                    cx.notify();
                });
                window.prevent_default();
                cx.stop_propagation();
            });
        },
    )
}

fn neighbor_index(idx: usize, len: usize, forward: bool) -> Option<usize> {
    if forward {
        (idx + 1 < len).then_some(idx + 1)
    } else {
        idx.checked_sub(1)
    }
}

fn search_seed_from_key(ev: &gpui::KeyDownEvent) -> Option<String> {
    if ev.keystroke.modifiers.control
        || ev.keystroke.modifiers.alt
        || ev.keystroke.modifiers.platform
    {
        return None;
    }
    let key = ev.keystroke.key.as_str();
    if key == "space" || key.chars().count() != 1 {
        return None;
    }
    Some(key.to_string())
}
