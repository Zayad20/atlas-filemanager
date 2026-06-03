use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DragHoverTarget, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::{FileEntry, FileKind, SortDir};
use crate::state::tab::{FileListHitRegions, TabState};
use crate::ui::drag_feedback::paint_drag_hover_confirmation;
use crate::ui::view_layout::GridLayout;

pub fn is_preview_image(extension: Option<&str>) -> bool {
    matches!(
        extension.map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif")
    )
}

pub fn drag_navigation_active(ctx: &egui::Context, dragged_files: &Option<DraggedFiles>) -> bool {
    dragged_files.is_some() || ctx.input(|input| !input.raw.hovered_files.is_empty())
}

/// Auto-scrolls the list when the pointer nears the top/bottom edge while `active`
/// (used both for file drag-and-drop and for rubber-band selection).
pub fn auto_scroll_during_drag(ui: &egui::Ui, active: bool) {
    const EDGE_SIZE: f32 = 52.0;
    const MAX_SPEED: f32 = 760.0;

    if !active {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_hover_pos() else {
        return;
    };
    let viewport = ui.clip_rect();
    if pointer.x < viewport.left() || pointer.x > viewport.right() {
        return;
    }
    let proximity = if pointer.y < viewport.top() + EDGE_SIZE {
        ((viewport.top() + EDGE_SIZE - pointer.y) / EDGE_SIZE).clamp(0.0, 1.0)
    } else if pointer.y > viewport.bottom() - EDGE_SIZE {
        -((pointer.y - (viewport.bottom() - EDGE_SIZE)) / EDGE_SIZE).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if proximity == 0.0 {
        return;
    }
    let dt = ui.input(|input| input.stable_dt.min(0.05));
    ui.scroll_with_delta(egui::vec2(0.0, proximity * MAX_SPEED * dt));
    ui.ctx().request_repaint();
}

pub fn finish_drop(
    ui: &egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    list_rect: egui::Rect,
    mut drop_dest: Option<PathBuf>,
    dragged_files: &mut Option<DraggedFiles>,
    actions: &mut Vec<Action>,
) {
    if dragged_files.as_ref().is_some_and(|drag| {
        drag.source_tab != tile_id
            || drag
                .paths
                .iter()
                .any(|path| path.parent() != Some(pane_path))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pointer| list_rect.contains(pointer))
        && ui.input(|input| input.pointer.any_released())
    {
        drop_dest.get_or_insert_with(|| pane_path.to_path_buf());
    }
    if let Some(dest) = drop_dest {
        if let Some(drag) = dragged_files.take() {
            actions.push(Action::Move {
                paths: drag.paths,
                dest,
            });
        }
    }
}

pub fn handle_pointer_events(
    ctx: &egui::Context,
    tab_states: &mut HashMap<TileId, TabState>,
    actions: &mut Vec<Action>,
) {
    if egui::Popup::is_any_open(ctx) {
        clear_click_tracking(tab_states);
        return;
    }

    const MAX_CLICK_DIST: f32 = 6.0;
    const MAX_DOUBLE_CLICK_DELAY: f64 = 0.3;

    let (time, events) = ctx.input(|input| (input.time, input.events.clone()));

    for event in events {
        let egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } = event
        else {
            continue;
        };

        match (button, pressed) {
            (egui::PointerButton::Primary, true) => {
                for tab_state in tab_states.values_mut() {
                    tab_state.selection.primary_press = None;
                }
                if let Some((tile, Some(idx))) = hit_list(ctx, tab_states, pos) {
                    if let Some(tab_state) = tab_states.get_mut(&tile) {
                        tab_state.selection.primary_press = Some((idx, pos));
                    }
                    actions.push(Action::Select {
                        tile,
                        idx,
                        shift: modifiers.shift,
                        ctrl: modifiers.ctrl,
                    });
                } else if let Some((tile, None)) = hit_list(ctx, tab_states, pos) {
                    actions.push(Action::ClearSelection(tile));
                }
            }
            (egui::PointerButton::Primary, false) => {
                let pressed_item = tab_states.iter_mut().find_map(|(&tile, tab_state)| {
                    tab_state
                        .selection
                        .primary_press
                        .take()
                        .map(|(idx, origin)| (tile, idx, origin))
                });
                let completed_click = pressed_item.and_then(|(tile, idx, origin)| {
                    (hit_list(ctx, tab_states, pos) == Some((tile, Some(idx)))
                        && origin.distance(pos) <= MAX_CLICK_DIST)
                        .then_some((tile, idx))
                });
                if let Some((tile, idx)) = completed_click {
                    let is_double_click = tab_states.get(&tile).is_some_and(|tab_state| {
                        tab_state.selection.last_primary_click.is_some_and(
                            |(previous_idx, previous_time)| {
                                previous_idx == idx
                                    && time - previous_time <= MAX_DOUBLE_CLICK_DELAY
                            },
                        )
                    });
                    for tab_state in tab_states.values_mut() {
                        tab_state.selection.last_primary_click = None;
                    }
                    if is_double_click {
                        actions.push(Action::Activate { tile, idx });
                    } else if let Some(tab_state) = tab_states.get_mut(&tile) {
                        tab_state.selection.last_primary_click = Some((idx, time));
                    }
                } else {
                    for tab_state in tab_states.values_mut() {
                        tab_state.selection.last_primary_click = None;
                    }
                }
            }
            (egui::PointerButton::Secondary, true) => {
                if let Some((tile, Some(idx))) = hit_list(ctx, tab_states, pos) {
                    let should_select = tab_states
                        .get(&tile)
                        .is_some_and(|tab_state| !tab_state.selection.selected.contains(&idx));
                    if should_select {
                        actions.push(Action::Select {
                            tile,
                            idx,
                            shift: modifiers.shift,
                            ctrl: modifiers.ctrl,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn hit_list(
    ctx: &egui::Context,
    tab_states: &HashMap<TileId, TabState>,
    pos: egui::Pos2,
) -> Option<(TileId, Option<usize>)> {
    tab_states.iter().find_map(|(&tile, tab_state)| {
        let regions = tab_state.file_list_hit_regions.as_ref()?;
        (regions.list_rect.contains(pos) && ctx.layer_id_at(pos) == Some(regions.layer_id)).then(
            || {
                let idx = regions
                    .item_rects
                    .iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(idx, _)| *idx);
                (tile, idx)
            },
        )
    })
}

pub fn clear_click_tracking(tab_states: &mut HashMap<TileId, TabState>) {
    for tab_state in tab_states.values_mut() {
        tab_state.selection.primary_press = None;
        tab_state.selection.last_primary_click = None;
    }
}

pub fn record_hit_regions(
    ui: &egui::Ui,
    tab_state: &mut TabState,
    list_rect: egui::Rect,
    item_rects: Vec<(usize, egui::Rect)>,
) {
    tab_state.file_list_hit_regions = Some(FileListHitRegions {
        list_rect,
        layer_id: ui.layer_id(),
        item_rects,
    });
}

/// Rubber-band selection driven by the analytical [`GridLayout`].
///
/// The anchor and the band are tracked in content coordinates (independent of
/// scroll), and membership is computed from `layout` rather than from the rects of
/// the currently-rendered rows. That keeps the selection correct while the list
/// auto-scrolls and items leave the viewport — the long-standing select-box bug.
///
/// `content_origin` is the screen position of content-local `(0, 0)` for this frame
/// (see [`GridLayout::content_origin`]); `None` when nothing was rendered.
pub fn update_drag_selection(
    ui: &egui::Ui,
    tab_state: &mut TabState,
    background: &egui::Response,
    layout: &GridLayout,
    content_origin: Option<egui::Pos2>,
    visible_indices: &[usize],
) {
    if background.drag_started() {
        if let (Some(origin), Some(co)) = (
            ui.input(|input| input.pointer.press_origin()),
            content_origin,
        ) {
            let local = (origin - co).to_pos2();
            // Don't start a rubber-band when the press lands on an item (that's a
            // click / drag-move, not a marquee).
            if layout.pos_at(local).is_none() {
                tab_state.selection.drag_select_anchor = Some(local.to_vec2());
                tab_state.selection.drag_select_base = if ui.input(|input| input.modifiers.ctrl) {
                    tab_state.selection.selected.clone()
                } else {
                    Default::default()
                };
            }
        }
    }
    if let (Some(anchor), Some(co), Some(pointer)) = (
        tab_state.selection.drag_select_anchor,
        content_origin,
        ui.input(|input| input.pointer.interact_pos()),
    ) {
        let cur = (pointer - co).to_pos2();
        let band = egui::Rect::from_two_pos(anchor.to_pos2(), cur);
        let mut selected = tab_state.selection.drag_select_base.clone();
        for pos in layout.indices_in_local_rect(band) {
            if let Some(&idx) = visible_indices.get(pos) {
                selected.insert(idx);
            }
        }
        tab_state.selection.selected = selected;

        let screen = band.translate(co.to_vec2());
        ui.painter().rect_stroke(
            screen,
            0.0,
            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(
            screen,
            0.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.25),
        );
    }
    if background.drag_stopped() {
        tab_state.selection.drag_select_anchor = None;
        tab_state.selection.drag_select_base.clear();
    }
}

/// Whether a rubber-band marquee is currently in progress for this tab.
pub fn rubber_band_active(tab_state: &TabState) -> bool {
    tab_state.selection.drag_select_anchor.is_some()
}

/// Folder drop / drag-hover / external-drop bookkeeping for a single cell. Shared by
/// every grid-style view so the drag behaviour is identical across them.
pub(crate) fn handle_cell_drop_targets(
    resp: &egui::Response,
    tile_id: TileId,
    entry: &FileEntry,
    dragged_files: &Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    drop_dest: &mut Option<PathBuf>,
) {
    if entry.kind != FileKind::Dir {
        return;
    }
    let target = DragHoverTarget::Folder {
        tab: tile_id,
        path: entry.path.clone(),
    };
    if resp.contains_pointer() && resp.ctx.input(|input| input.pointer.any_released()) {
        *drop_dest = Some(entry.path.clone());
    }
    if resp.contains_pointer() && drag_navigation_active(&resp.ctx, dragged_files) {
        drag_hover.observed_target = Some(target.clone());
    }
    paint_drag_hover_confirmation(resp, drag_hover, &target);
    if resp.contains_pointer() && resp.ctx.input(|input| !input.raw.hovered_files.is_empty()) {
        *external_drop_target = Some(ExternalDropTarget {
            tab: tile_id,
            dest: entry.path.clone(),
        });
    }
}

/// Generic row-major grid renderer shared by the Icons (grid) and List views.
///
/// `draw_cell` paints the per-item content into a cell sized `item_size` laid out
/// with `cell_layout`; everything else — virtualization, selection feedback,
/// click/drag sensing, context menus, folder drop targets, rubber-band and hit
/// regions — is handled here so the two views stay DRY and behave identically.
#[allow(clippy::too_many_arguments)]
pub fn render_grid<F>(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    tab_state: &mut TabState,
    item_size: egui::Vec2,
    spacing: egui::Vec2,
    cell_layout: egui::Layout,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
    mut draw_cell: F,
) where
    F: FnMut(&mut egui::Ui, &FileEntry, bool),
{
    let visible_indices = tab_state.visible_indices_shared();
    let selected_snapshot = tab_state.selection.selected.clone();
    let list_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        list_rect,
        ui.id().with(("file_grid_background", tile_id)),
        egui::Sense::click_and_drag(),
    );

    let avail_w = ui.available_width();
    let target_cols = GridLayout::columns_for_width(avail_w, item_size.x, spacing.x);
    let is_resizing = (avail_w - tab_state.last_pane_width).abs() > 0.5;
    tab_state.last_pane_width = avail_w;

    // Freeze column count during active resize to avoid scroll-height jumps
    // when crossing column boundaries. Fall back to target if previous count
    // no longer fits (pane shrank enough to cause overflow).
    let columns = if is_resizing && tab_state.grid_columns > 0 {
        let prev = tab_state.grid_columns;
        let min_w = (prev as f32) * (item_size.x + spacing.x) - spacing.x;
        if avail_w >= min_w { prev } else { target_cols }
    } else {
        target_cols
    };
    tab_state.grid_columns = columns;

    let (column_spacing, side_margin) = GridLayout::horizontal_distribution(
        avail_w,
        item_size.x,
        columns,
        spacing.x,
        24.0,
    );
    let layout = GridLayout::new(
        item_size,
        egui::vec2(column_spacing, spacing.y),
        columns,
        visible_indices.len(),
    );
    let scroll_active =
        drag_navigation_active(ui.ctx(), dragged_files) || rubber_band_active(tab_state);

    let mut cell_rects: Vec<(usize, egui::Rect)> = Vec::new();
    let mut origin_sample: Option<(usize, egui::Rect)> = None;
    let mut drop_dest = None;

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = spacing.y;
        egui::ScrollArea::vertical()
            .id_salt(("grid_scroll", tile_id))
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            .show_rows(ui, item_size.y, layout.rows(), |ui, rows| {
                auto_scroll_during_drag(ui, scroll_active);
                for row in rows {
                    ui.horizontal(|ui| {
                        ui.add_space(side_margin);
                        ui.spacing_mut().item_spacing.x = column_spacing;
                        let start = row * columns;
                        let end = (start + columns).min(visible_indices.len());
                        for (offset, &idx) in visible_indices[start..end].iter().enumerate() {
                            let pos = start + offset;
                            let entry = &tab_state.entries[idx];
                            let is_selected = selected_snapshot.contains(&idx);
                            let inner = ui.allocate_ui_with_layout(item_size, cell_layout, |ui| {
                                if is_selected {
                                    ui.painter().rect_filled(
                                        ui.max_rect(),
                                        4.0,
                                        ui.visuals().selection.bg_fill,
                                    );
                                }
                                draw_cell(ui, entry, is_selected);
                            });
                            let resp = ui.interact(
                                inner.response.rect,
                                ui.id().with(("file_grid_cell", tile_id, idx)),
                                egui::Sense::click_and_drag(),
                            );
                            cell_rects.push((idx, resp.rect));
                            origin_sample.get_or_insert((pos, resp.rect));
                            if tab_state.selection.scroll_to_selected && is_selected {
                                resp.scroll_to_me(Some(egui::Align::Center));
                                tab_state.selection.scroll_to_selected = false;
                            }
                            if resp.drag_started() {
                                let paths = if is_selected {
                                    selected_paths(tab_state)
                                } else {
                                    vec![entry.path.clone()]
                                };
                                *dragged_files = Some(DraggedFiles {
                                    source_tab: tile_id,
                                    paths,
                                });
                            }
                            handle_cell_drop_targets(
                                &resp,
                                tile_id,
                                entry,
                                dragged_files,
                                drag_hover,
                                external_drop_target,
                                &mut drop_dest,
                            );
                            resp.context_menu(|ui| {
                                let paths = if is_selected {
                                    selected_paths(tab_state)
                                } else {
                                    vec![entry.path.clone()]
                                };
                                crate::ui::context_menu::show_context_menu(
                                    ui, tile_id, pane_path, &paths, actions,
                                );
                            });
                        }
                    });
                }
            });
    });

    let content_origin = origin_sample.map(|(pos, rect)| layout.content_origin(pos, rect));
    update_drag_selection(
        ui,
        tab_state,
        &background,
        &layout,
        content_origin,
        &visible_indices,
    );
    record_hit_regions(ui, tab_state, list_rect, cell_rects);
    background.context_menu(|ui| {
        crate::ui::context_menu::show_context_menu(ui, tile_id, pane_path, &[], actions);
    });
    finish_drop(
        ui,
        tile_id,
        pane_path,
        list_rect,
        drop_dest,
        dragged_files,
        actions,
    );
    show_filter_bar(ui, tab_state, visible_indices.len());
}

pub fn selected_paths(tab_state: &TabState) -> Vec<PathBuf> {
    tab_state
        .selection
        .selected
        .iter()
        .filter_map(|idx| tab_state.entries.get(*idx))
        .map(|entry| entry.path.clone())
        .collect()
}

pub fn show_filter_bar(ui: &mut egui::Ui, tab_state: &mut TabState, visible_count: usize) {
    if tab_state.filter.is_empty() {
        return;
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Filter:").weak());
        ui.label(egui::RichText::new(&tab_state.filter).monospace());
        ui.label(egui::RichText::new(format!("({visible_count})")).weak());
        if ui.small_button("x").clicked() {
            tab_state.filter.clear();
        }
    });
}

pub fn sort_arrow(label: &str, active: bool, dir: &SortDir) -> String {
    if active {
        let arrow = if *dir == SortDir::Asc { " ↑" } else { " ↓" };
        format!("{label}{arrow}")
    } else {
        label.to_string()
    }
}

pub fn toggle_dir(currently_active: bool, current: &SortDir) -> SortDir {
    if currently_active {
        if *current == SortDir::Asc {
            SortDir::Desc
        } else {
            SortDir::Asc
        }
    } else {
        SortDir::Asc
    }
}

#[cfg(test)]
mod tests {
    use super::handle_pointer_events;
    use crate::input::action::Action;
    use crate::state::tab::{FileListHitRegions, TabState};
    use egui::{pos2, Event, PointerButton};
    use egui_tiles::TileId;
    use std::collections::HashMap;

    fn pointer_button(x: f32, pressed: bool) -> Event {
        Event::PointerButton {
            pos: pos2(x, 15.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn setup() -> (egui::Context, HashMap<TileId, TabState>) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |_| {});
        });

        let state = TabState {
            file_list_hit_regions: Some(FileListHitRegions {
                list_rect: egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0)),
                layer_id: ctx.layer_id_at(pos2(15.0, 15.0)).expect("background layer"),
                item_rects: vec![
                    (
                        0,
                        egui::Rect::from_min_max(pos2(10.0, 10.0), pos2(50.0, 30.0)),
                    ),
                    (
                        1,
                        egui::Rect::from_min_max(pos2(60.0, 10.0), pos2(100.0, 30.0)),
                    ),
                ],
            }),
            ..Default::default()
        };
        (ctx, HashMap::from([(TileId(1), state)]))
    }

    fn actions_for(
        ctx: &egui::Context,
        tab_states: &mut HashMap<TileId, TabState>,
        time: f64,
        events: Vec<Event>,
    ) -> Vec<Action> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let _ = ctx.run(raw, |ctx| {
            handle_pointer_events(ctx, tab_states, &mut actions);
        });
        actions
    }

    #[test]
    fn preserves_order_for_multiple_clicks_in_one_frame() {
        let (ctx, mut tab_states) = setup();
        let actions = actions_for(
            &ctx,
            &mut tab_states,
            1.0,
            vec![
                pointer_button(15.0, true),
                pointer_button(15.0, false),
                pointer_button(65.0, true),
                pointer_button(65.0, false),
            ],
        );

        assert!(matches!(
            actions.as_slice(),
            [Action::Select { idx: 0, .. }, Action::Select { idx: 1, .. }]
        ));
    }

    #[test]
    fn activates_only_when_both_clicks_target_the_same_item() {
        let (ctx, mut tab_states) = setup();
        let actions = actions_for(
            &ctx,
            &mut tab_states,
            1.0,
            vec![
                pointer_button(15.0, true),
                pointer_button(15.0, false),
                pointer_button(15.0, true),
                pointer_button(15.0, false),
            ],
        );

        assert!(matches!(
            actions.as_slice(),
            [
                Action::Select { idx: 0, .. },
                Action::Select { idx: 0, .. },
                Action::Activate { idx: 0, .. }
            ]
        ));
    }
}
