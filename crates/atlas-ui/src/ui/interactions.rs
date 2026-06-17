//! Pane interactions: rubber-band selection, inline rename, type-to-filter, ctrl-scroll.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::layout_tree::Tile;
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use gpui::prelude::*;
use gpui::{canvas, Context, DispatchPhase, Entity, MouseMoveEvent, MouseUpEvent, Window};
use gpui_component::WindowExt;

impl App {
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
            crate::state::settings::ViewMode::Details => (1, count),
            crate::state::settings::ViewMode::Grid => {
                let thumb = td.thumbnail_size;
                const GAP: f32 = 8.0;
                const PAD: f32 = 0.0;
                let pane_w = ts.last_pane_width.max(thumb + 1.0);
                let inner = (pane_w - 2.0 * PAD).max(thumb);
                let cols = (((inner + GAP) / (thumb + GAP)).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            crate::state::settings::ViewMode::List => {
                const CELL_W: f32 = 220.0;
                let stride = CELL_W + 6.0;
                let avail = ts.last_pane_width.max(stride);
                let cols = ((avail / stride).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            crate::state::settings::ViewMode::Miller => (1, 0),
        }
    }

    /// FASE C: recompute the focused rubber-band selection by intersecting its
    /// rectangle (window coords) against the recorded visible-item rects.
    pub(crate) fn apply_rubber(&mut self) {
        let Some(r) = self.rubber.as_ref() else {
            return;
        };
        if !r.started {
            return;
        }
        let pane = r.pane;
        let (ox, oy) = (f32::from(r.origin.x), f32::from(r.origin.y));
        let (cx_, cy) = (f32::from(r.current.x), f32::from(r.current.y));
        let (rl, rr) = (ox.min(cx_), ox.max(cx_));
        let (rt, rb) = (oy.min(cy), oy.max(cy));
        let mut hits = std::collections::HashSet::new();
        let mut visible = std::collections::HashSet::new();
        if let Some(rects) = self.row_rects.get(&pane) {
            for (idx, b) in rects {
                visible.insert(*idx);
                let (bl, br) = (f32::from(b.left()), f32::from(b.right()));
                let (bt, bb) = (f32::from(b.top()), f32::from(b.bottom()));
                if rl < br && bl < rr && rt < bb && bt < rb {
                    hits.insert(*idx);
                }
            }
        }
        let Some(r) = self.rubber.as_mut() else {
            return;
        };
        r.swept.extend(hits.iter().copied());
        // Items visible but no longer under the rect get deselected; items that
        // scrolled off-screen stay in swept (can't re-check their position).
        r.swept
            .retain(|idx| !visible.contains(idx) || hits.contains(idx));
        let mut sel: std::collections::HashSet<usize> = if r.additive {
            r.base.clone()
        } else {
            std::collections::HashSet::new()
        };
        sel.extend(r.swept.iter().copied());
        if let Some(ts) = self.state.tab_states.get_mut(&pane) {
            ts.selection.selected = sel;
        }
    }

    /// Nudge a virtualized file view while rubber-band selecting near the visible
    /// top/bottom edge. `cols` maps file positions to uniform-list rows.
    pub(crate) fn rubber_autoscroll_rows(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
        cols: usize,
        row_count: usize,
    ) {
        if row_count == 0 || self.rubber.as_ref().is_none_or(|r| r.pane != pane) {
            return;
        }
        let Some(rects) = self.row_rects.get(&pane) else {
            return;
        };
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return;
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
            return;
        }

        const EDGE: f32 = 44.0;
        let y = f32::from(pos.y);
        let target = if y < top + EDGE {
            top_row.saturating_sub(1)
        } else if y > bottom - EDGE {
            (bottom_row + 1).min(row_count.saturating_sub(1))
        } else {
            return;
        };

        if self.scroll_grid_row_to(pane, target, gpui::ScrollStrategy::Nearest) {
            return;
        }

        if let Some(handle) = self.scroll_handles.get(&pane) {
            let strategy = if target <= top_row {
                gpui::ScrollStrategy::Top
            } else {
                gpui::ScrollStrategy::Bottom
            };
            handle.scroll_to_item_strict(target, strategy);
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

    /// Begin inline rename of the focused pane's cursor item: pre-fill a native input
    /// with its current name and stash it in `rename_edit` (rendered by `render_row`).
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.state.focused_tile;
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return;
        };
        let idx = ts
            .selection
            .keyboard_cursor
            .or_else(|| ts.selection.selected.iter().copied().next());
        let Some(idx) = idx else { return };
        let Some(entry) = ts.entries.get(idx) else {
            return;
        };
        let path = entry.path.clone();
        let name = entry.name.clone();
        let input = crate::ui::text_input::single_line(window, cx, "", name);
        self.rename_edit = Some(crate::app::RenameEdit {
            pane,
            idx,
            path,
            input,
            initial_select_chars: None,
        });
    }

    /// Confirm the inline rename: validate the new name (cross-platform: reject path
    /// separators and the `.`/`..` specials) and dispatch the FS rename.
    pub(crate) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.rename_edit.take() else {
            return;
        };
        let new_name = edit.input.read(cx).value().to_string();
        let new_name = new_name.trim();
        if new_name.is_empty()
            || new_name == "."
            || new_name == ".."
            || new_name.contains('/')
            || new_name.contains('\\')
        {
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
        self.rename_edit = None;
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
            pane,
            origin: pos,
            current: pos,
            base,
            swept: std::collections::HashSet::new(),
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

            // ── Global move ──────────────────────────────────────────────────
            let vm = v.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vm.update(cx, |this, cx| {
                    // Column resize takes priority over rubber-band selection.
                    if let Some((col, start_x, start_width)) = this
                        .col_resize
                        .as_ref()
                        .map(|cr| (cr.col, cr.start_x, cr.start_width))
                    {
                        let dx = start_x - f32::from(pos.x);
                        let min_w = match col {
                            "size" => 46.0,
                            "modified" => 92.0,
                            "type" => 56.0,
                            _ => 35.0,
                        };
                        let new_w = (start_width + dx).max(min_w);
                        match col {
                            "size" => this.col_widths.size = new_w,
                            "modified" => this.col_widths.modified = new_w,
                            "type" => this.col_widths.type_w = new_w,
                            _ => {}
                        }
                        cx.notify();
                        return;
                    }

                    let Some(r) = this.rubber.as_mut() else {
                        return;
                    };
                    r.current = pos;
                    if !r.started {
                        let dx = f32::from(pos.x) - f32::from(r.origin.x);
                        let dy = f32::from(pos.y) - f32::from(r.origin.y);
                        r.started = dx * dx + dy * dy >= 16.0;
                    }
                    if r.started {
                        this.apply_rubber();
                        let pane = this.rubber.as_ref().map(|r| r.pane);
                        let cur = this.rubber.as_ref().map(|r| r.current);
                        if let (Some(pane), Some(cur)) = (pane, cur) {
                            let (cols, rows) = this.rubber_scroll_dims(pane);
                            this.rubber_autoscroll_rows(pane, cur, cols, rows);
                        }
                    }
                    cx.notify();
                });
            });

            // ── Global up ────────────────────────────────────────────────────
            let vu = v.clone();
            window.on_mouse_event::<MouseUpEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vu.update(cx, |this, cx| {
                    // End column resize.
                    if this.col_resize.take().is_some() {
                        cx.notify();
                        return;
                    }

                    if let Some(r) = this.rubber.take() {
                        if !r.started && !r.additive {
                            // Plain click on empty space → clear selection.
                            // If pos hit an item rect, that item's on_click already handled it.
                            let hit = this.row_rects.get(&r.pane).is_some_and(|rects| {
                                rects.iter().any(|(_, b)| {
                                    pos.x >= b.left()
                                        && pos.x <= b.right()
                                        && pos.y >= b.top()
                                        && pos.y <= b.bottom()
                                })
                            });
                            if !hit {
                                if let Some(ts) = this.state.tab_states.get_mut(&r.pane) {
                                    ts.selection.clear();
                                }
                            }
                        }
                        cx.notify();
                    }
                });
            });

            // Keep repainting while rubber is active (continuous autoscroll at edges).
            if has_rubber {
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
