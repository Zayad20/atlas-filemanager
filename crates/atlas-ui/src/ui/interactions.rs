//! Pane interactions: rubber-band selection, inline rename, type-to-filter, ctrl-scroll.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::{App, RubberTarget};
use crate::input::action::Action;
use atlas_model::layout_tree::Tile;
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{
    canvas, Context, DispatchPhase, Entity, KeyDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};
use gpui_component::{ActiveTheme, WindowExt};

impl App {
    fn rubber_target_bounds(&self, target: RubberTarget) -> Option<gpui::Bounds<gpui::Pixels>> {
        match target {
            RubberTarget::Pane(pane) => self.rubber_pane_bounds(pane),
            RubberTarget::Dropzone => self.dropzone_bounds,
        }
    }

    fn rubber_pane_bounds(&self, pane: TabId) -> Option<gpui::Bounds<gpui::Pixels>> {
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
                const GAP: f32 = 8.0;
                const PAD: f32 = 0.0;
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

    /// FASE C: recompute the focused rubber-band selection by intersecting its
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
        // could disagree near pane edges → "descuadre".
        let origin = self.clamp_rubber_target_point(target, r.origin);
        let current = self.clamp_rubber_target_point(target, r.current);
        let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));
        let (cx_, cy) = (f32::from(current.x), f32::from(current.y));
        let (rl, rr) = (ox.min(cx_), ox.max(cx_));
        let (rt, rb) = (oy.min(cy), oy.max(cy));
        match target {
            RubberTarget::Pane(pane) => {
                let mut hits = std::collections::HashSet::new();
                if let Some(rects) = self.row_rects.get(&pane) {
                    for (idx, b) in rects {
                        let (bl, br) = (f32::from(b.left()), f32::from(b.right()));
                        let (bt, bb) = (f32::from(b.top()), f32::from(b.bottom()));
                        // AABB overlap: the marquee and the item box share any area.
                        if rl < br && bl < rr && rt < bb && bt < rb {
                            hits.insert(*idx);
                        }
                    }
                }
                let Some(r) = self.rubber.as_mut() else {
                    return;
                };
                let mut sel: std::collections::HashSet<usize> = if r.additive {
                    r.base.clone()
                } else {
                    std::collections::HashSet::new()
                };
                sel.extend(hits.iter().copied());
                if let Some(ts) = self.state.tab_states.get_mut(&pane) {
                    ts.selection.selected = sel;
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

    /// Nudge a virtualized file view while rubber-band selecting near the visible
    /// top/bottom edge. `cols` maps file positions to uniform-list rows.
    /// Returns `true` when it actually scrolled (caller requests the next animation
    /// frame only then — keeps the render loop idle while the cursor sits still in
    /// the middle of the pane).
    pub(crate) fn rubber_autoscroll_rows(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
        cols: usize,
        row_count: usize,
    ) -> bool {
        if row_count == 0
            || self
                .rubber
                .as_ref()
                .is_none_or(|r| r.target != RubberTarget::Pane(pane))
        {
            return false;
        }
        let Some(rects) = self.row_rects.get(&pane) else {
            return false;
        };
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return false;
        };

        let mut top = f32::INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        let mut top_row = usize::MAX;
        let mut bottom_row = 0usize;

        for (idx, b) in rects {
            let Some(visible_pos) = ts.visible.iter().position(|v| v == idx) else {
                continue;
            };
            let row = visible_pos / cols.max(1);
            let item_top = f32::from(b.top());
            let item_bottom = f32::from(b.bottom());
            top = top.min(item_top);
            bottom = bottom.max(item_bottom);
            top_row = top_row.min(row);
            bottom_row = bottom_row.max(row);
        }

        if !top.is_finite() || !bottom.is_finite() {
            return false;
        }

        const EDGE: f32 = 44.0;
        let y = f32::from(pos.y);
        let target = if y < top + EDGE {
            // Deeper into the zone → more rows per tick (1 at boundary, 4 at edge).
            let t = 1.0 - (y - top).clamp(0.0, EDGE) / EDGE;
            let step = (1.0 + t * 3.0).round() as usize;
            top_row.saturating_sub(step)
        } else if y > bottom - EDGE {
            let t = 1.0 - (bottom - y).clamp(0.0, EDGE) / EDGE;
            let step = (1.0 + t * 3.0).round() as usize;
            (bottom_row + step).min(row_count.saturating_sub(1))
        } else {
            return false;
        };

        if self.scroll_grid_row_to(pane, target, gpui::ScrollStrategy::Nearest) {
            return true;
        }

        if let Some(handle) = self.scroll_handles.get(&pane) {
            let strategy = if target <= top_row {
                gpui::ScrollStrategy::Top
            } else {
                gpui::ScrollStrategy::Bottom
            };
            handle.scroll_to_item_strict(target, strategy);
            return true;
        }
        false
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

        const GRID_GAP: f32 = 8.0;
        let row_pitch = pane_data.thumbnail_size * 0.62 + 32.0 + GRID_GAP;
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
        let content_h = row_count as f32 * row_pitch - GRID_GAP;
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

    /// Begin inline rename of the focused pane's cursor item: pre-fill a native input
    /// with its current name and stash it in `rename_edit`.
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.state.focused_tile;
        let Some((idx, visible_pos, path, name, initial_select_chars)) = (|| {
            let ts = self.state.tab_states.get(&pane)?;
            let idx = ts
                .selection
                .keyboard_cursor
                .or_else(|| ts.selection.selected.iter().copied().next())?;
            let visible_pos = ts.visible.iter().position(|&visible| visible == idx)?;
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
        match self
            .tab_data_for(pane)
            .map(|tab| tab.view_mode.clone())
            .unwrap_or_default()
        {
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
                self.process_actions(vec![Action::Rename {
                    from: edit.path,
                    to,
                }]);
            }
        }
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
                RubberTarget::Dropzone => {
                    self.dropzone_selected = r.dropzone_base;
                }
            }
        }
    }

    /// FASE C: start a rubber-band drag in `pane` at `pos` (window coords), capturing
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
        self.rubber = Some(crate::app::RubberDrag {
            target: RubberTarget::Pane(pane),
            origin: pos,
            current: pos,
            base,
            dropzone_base: Default::default(),
            additive,
            started: false,
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
        self.rubber = Some(crate::app::RubberDrag {
            target: RubberTarget::Dropzone,
            origin: pos,
            current: pos,
            base: Default::default(),
            dropzone_base,
            additive,
            started: false,
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
        let mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();

        let mut next_mode = mode.clone();

        if matches!(mode, ViewMode::Grid) {
            if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get_mut(pane_id) {
                let step = if dy > 0.0 { 16.0 } else { -16.0 };
                let new_size = p.thumbnail_size + step;
                if new_size < 64.0 && dy < 0.0 {
                    next_mode = ViewMode::List;
                } else if new_size > 384.0 && dy > 0.0 {
                    p.thumbnail_size = 384.0;
                    return;
                } else {
                    p.thumbnail_size = new_size;
                    return;
                }
            }
        } else if matches!(mode, ViewMode::List) {
            if dy < 0.0 {
                next_mode = ViewMode::Details;
            } else if dy > 0.0 {
                next_mode = ViewMode::Grid;
            }
        } else if matches!(mode, ViewMode::Details) {
            if dy > 0.0 {
                next_mode = ViewMode::List;
            } else {
                return;
            }
        } else {
            return; // don't cycle out of Miller
        }

        if next_mode == mode {
            return;
        }

        let next_mode_clone = next_mode.clone();
        self.state.focused_tile = pane_id;
        if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get_mut(pane_id) {
            p.view_mode = next_mode;
        }
        // Leaving Miller can leave the canonical entries empty; reload for the others.
        if next_mode_clone != ViewMode::Miller {
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
            let has_rubber = v.read(cx).rubber.is_some();
            let rubber_started = v.read(cx).rubber.as_ref().is_some_and(|r| r.started);

            // ── Crosshair cursor while drag is active ────────────────────────
            if rubber_started {
                window.set_window_cursor_style(gpui::CursorStyle::Crosshair);
            }

            // ── Escape cancels the drag, restoring the pre-drag selection ────
            let ve = v.clone();
            window.on_key_event::<KeyDownEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if ev.keystroke.key == "escape" {
                    ve.update(cx, |this, cx| {
                        if this.rubber.is_some() {
                            this.cancel_rubber();
                            cx.notify();
                        }
                    });
                }
            });

            // ── Draw the rubber band marquee rectangle ───────────────────────
            if let Some(r) = v.read(cx).rubber.as_ref() {
                let app = v.read(cx);
                let origin = app.clamp_rubber_target_point(r.target, r.origin);
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
                    fill.a = 0.15;

                    window.paint_quad(gpui::fill(rect, fill));

                    let mut border = t.accent;
                    border.a = 0.8;
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

            // ── Global move ──────────────────────────────────────────────────
            let vm = v.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vm.update(cx, |this, cx| {
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
                            (resize.start_width + dx).clamp(180.0, 520.0);
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

                    let Some(r) = this.rubber.as_mut() else {
                        return false;
                    };
                    r.current = pos;
                    if !r.started {
                        let dx = f32::from(pos.x) - f32::from(r.origin.x);
                        let dy = f32::from(pos.y) - f32::from(r.origin.y);
                        r.started = dx * dx + dy * dy >= 16.0;
                    }
                    if r.started {
                        this.apply_rubber();
                        let pane = this.rubber.as_ref().and_then(|r| match r.target {
                            RubberTarget::Pane(pane) => Some(pane),
                            RubberTarget::Dropzone => None,
                        });
                        let cur = this.rubber.as_ref().map(|r| r.current);
                        let mut scrolled = false;
                        if let (Some(pane), Some(cur)) = (pane, cur) {
                            let (cols, rows) = this.rubber_scroll_dims(pane);
                            scrolled = this.rubber_autoscroll_rows(pane, cur, cols, rows);
                        }
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
                                    let hit = this.row_rects.get(&pane).is_some_and(|rects| {
                                        rects.iter().any(|(_, b)| {
                                            pos.x >= b.left()
                                                && pos.x <= b.right()
                                                && pos.y >= b.top()
                                                && pos.y <= b.bottom()
                                        })
                                    });
                                    if !hit {
                                        if let Some(ts) = this.state.tab_states.get_mut(&pane) {
                                            ts.selection.clear();
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

            // Keep repainting while rubber is active (continuous autoscroll at edges).
            if has_rubber {
                crate::perf::event(|| "request_animation_frame rubber".to_string());
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .inset_0()
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
