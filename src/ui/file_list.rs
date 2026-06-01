use egui::Layout;
use egui_extras::{Column, TableBuilder};
use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::input::action::Action;
use crate::state::app_state::{DragHoverState, DragHoverTarget, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::{FileKind, SortColumn, SortDir};
use crate::state::settings::ViewMode;
use crate::state::tab::{FileListHitRegions, TabState};
use crate::util::display::{extension_color, extension_icon, format_date, format_size};

pub fn show_file_list(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    tab_state: &mut TabState,
    sort_col: &SortColumn,
    sort_dir: &SortDir,
    view_mode: &ViewMode,
    thumbnail_size: f32,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
) {
    if *view_mode == ViewMode::Grid {
        show_file_grid(
            ui,
            tile_id,
            pane_path,
            tab_state,
            thumbnail_size,
            dragged_files,
            drag_hover,
            external_drop_target,
            actions,
        );
        return;
    }

    let row_height = 20.0;
    let visible_indices: Vec<usize> = tab_state.visible_indices().collect();
    tab_state.grid_columns = 1;

    // Snapshot selected and entries for use in closures
    let selected_snapshot = tab_state.selected.clone();
    let list_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        list_rect,
        ui.id().with(("file_list_background", tile_id)),
        egui::Sense::click_and_drag(),
    );
    let mut row_rects = Vec::new();
    let mut drop_dest = None;
    let max_scroll_height = (ui.available_height() - 28.0).max(80.0);

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(24.0)) // icon
        .column(Column::remainder().clip(true)) // name
        .column(Column::initial(72.0).range(50.0..=140.0)) // size
        .column(Column::initial(136.0).range(80.0..=200.0)) // modified
        .column(Column::initial(60.0).range(40.0..=120.0)) // type
        .max_scroll_height(max_scroll_height)
        .header(row_height, |mut header| {
            header.col(|_| {});
            header.col(|ui| {
                let active = *sort_col == SortColumn::Name;
                let label = sort_arrow("Name", active, sort_dir);
                if ui.selectable_label(active, label).clicked() {
                    actions.push(Action::SetSort {
                        col: SortColumn::Name,
                        dir: toggle_dir(active, sort_dir),
                    });
                }
            });
            header.col(|ui| {
                let active = *sort_col == SortColumn::Size;
                let label = sort_arrow("Size", active, sort_dir);
                if ui.selectable_label(active, label).clicked() {
                    actions.push(Action::SetSort {
                        col: SortColumn::Size,
                        dir: toggle_dir(active, sort_dir),
                    });
                }
            });
            header.col(|ui| {
                let active = *sort_col == SortColumn::Modified;
                let label = sort_arrow("Modified", active, sort_dir);
                if ui.selectable_label(active, label).clicked() {
                    actions.push(Action::SetSort {
                        col: SortColumn::Modified,
                        dir: toggle_dir(active, sort_dir),
                    });
                }
            });
            header.col(|ui| {
                ui.label("Type");
            });
        })
        .body(|mut body| {
            auto_scroll_during_drag(body.ui_mut(), dragged_files);
            body.rows(row_height, visible_indices.len(), |mut row| {
                let idx = visible_indices[row.index()];
                let entry = &tab_state.entries[idx];
                let is_selected = selected_snapshot.contains(&idx);
                row.set_selected(is_selected);

                let cell_sense = egui::Sense::click_and_drag();
                let mut cell_responses = Vec::with_capacity(5);

                // Icon column
                row.col(|ui| {
                    let icon = extension_icon(entry.extension.as_deref());
                    let color = if entry.kind == FileKind::Dir {
                        egui::Color32::from_rgb(200, 180, 80)
                    } else {
                        extension_color(entry.extension.as_deref())
                    };
                    ui.colored_label(color, icon);
                    cell_responses.push(ui.interact(
                        ui.max_rect(),
                        ui.id().with(("details_cell", tile_id, idx, 0)),
                        cell_sense,
                    ));
                });

                // Name column
                row.col(|ui| {
                    ui.label(&entry.name);
                    cell_responses.push(ui.interact(
                        ui.max_rect(),
                        ui.id().with(("details_cell", tile_id, idx, 1)),
                        cell_sense,
                    ));
                });

                // Size column
                row.col(|ui| {
                    let text = match entry.kind {
                        FileKind::Dir => "—".to_string(),
                        _ => entry.size.map(format_size).unwrap_or_default(),
                    };
                    ui.label(text);
                    cell_responses.push(ui.interact(
                        ui.max_rect(),
                        ui.id().with(("details_cell", tile_id, idx, 2)),
                        cell_sense,
                    ));
                });

                // Modified column
                row.col(|ui| {
                    let text = entry.modified.map(format_date).unwrap_or_default();
                    ui.label(text);
                    cell_responses.push(ui.interact(
                        ui.max_rect(),
                        ui.id().with(("details_cell", tile_id, idx, 3)),
                        cell_sense,
                    ));
                });

                // Type column
                row.col(|ui| {
                    let text = match entry.kind {
                        FileKind::Dir => "folder",
                        FileKind::Symlink => "symlink",
                        FileKind::File => entry.extension.as_deref().unwrap_or("file"),
                    };
                    ui.label(text);
                    cell_responses.push(ui.interact(
                        ui.max_rect(),
                        ui.id().with(("details_cell", tile_id, idx, 4)),
                        cell_sense,
                    ));
                });

                let mut responses = cell_responses.into_iter();
                let mut resp = responses.next().expect("details rows have cells");
                for cell_resp in responses {
                    resp = resp.union(cell_resp);
                }
                row_rects.push((idx, resp.rect));
                if tab_state.scroll_to_selected && is_selected {
                    resp.scroll_to_me(Some(egui::Align::Center));
                    tab_state.scroll_to_selected = false;
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
                if entry.kind == FileKind::Dir
                    && resp.contains_pointer()
                    && resp.ctx.input(|input| input.pointer.any_released())
                {
                    drop_dest = Some(entry.path.clone());
                }
                if entry.kind == FileKind::Dir
                    && resp.contains_pointer()
                    && drag_navigation_active(&resp.ctx, dragged_files)
                {
                    drag_hover.observed_target = Some(DragHoverTarget::Folder {
                        tab: tile_id,
                        path: entry.path.clone(),
                    });
                }
                if entry.kind == FileKind::Dir {
                    paint_drag_hover_confirmation(
                        &resp,
                        drag_hover,
                        &DragHoverTarget::Folder {
                            tab: tile_id,
                            path: entry.path.clone(),
                        },
                    );
                }
                if entry.kind == FileKind::Dir
                    && resp.contains_pointer()
                    && resp.ctx.input(|input| !input.raw.hovered_files.is_empty())
                {
                    *external_drop_target = Some(ExternalDropTarget {
                        tab: tile_id,
                        dest: entry.path.clone(),
                    });
                }
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
            });
        });

    update_drag_selection(ui, tab_state, &background, &row_rects);
    record_hit_regions(ui, tab_state, list_rect, row_rects);
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

fn show_file_grid(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    tab_state: &mut TabState,
    thumbnail_size: f32,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
) {
    let visible_indices: Vec<usize> = tab_state.visible_indices().collect();
    let selected_snapshot = tab_state.selected.clone();
    let list_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        list_rect,
        ui.id().with(("file_grid_background", tile_id)),
        egui::Sense::click_and_drag(),
    );
    let mut cell_rects = Vec::new();
    let mut drop_dest = None;
    let cell_size = egui::vec2(thumbnail_size + 24.0, thumbnail_size + 48.0);
    let preview_size = egui::vec2(thumbnail_size, thumbnail_size);

    let spacing = egui::vec2(8.0, 8.0);
    let columns = ((ui.available_width() + spacing.x) / (cell_size.x + spacing.x))
        .floor()
        .max(1.0) as usize;
    tab_state.grid_columns = columns;
    let row_count = visible_indices.len().div_ceil(columns);
    let row_height = cell_size.y + spacing.y;

    egui::ScrollArea::vertical().show_rows(ui, row_height, row_count, |ui, visible_rows| {
        auto_scroll_during_drag(ui, dragged_files);
        for row in visible_rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing.x;
                let start = row * columns;
                let end = (start + columns).min(visible_indices.len());
                for idx in &visible_indices[start..end] {
                    let entry = &tab_state.entries[*idx];
                    let is_selected = selected_snapshot.contains(idx);
                    let inner = ui.allocate_ui_with_layout(
                        cell_size,
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            let rect = ui.max_rect();
                            // Instant selection feedback - no fade, so rapid clicking feels snappy.
                            if is_selected {
                                ui.painter()
                                    .rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                            }
                            ui.vertical_centered(|ui| {
                                if is_preview_image(entry.extension.as_deref()) {
                                    let uri = format!("file://{}", entry.path.to_string_lossy());
                                    ui.add(egui::Image::new(uri).fit_to_exact_size(preview_size));
                                } else {
                                    let icon = if entry.kind == FileKind::Dir {
                                        "Folder"
                                    } else {
                                        extension_icon(entry.extension.as_deref())
                                    };
                                    ui.add_sized(
                                        preview_size,
                                        egui::Label::new(
                                            egui::RichText::new(icon)
                                                .size((thumbnail_size * 0.32).max(24.0))
                                                .color(extension_color(entry.extension.as_deref())),
                                        ),
                                    );
                                }
                                ui.add(
                                    egui::Label::new(&entry.name)
                                        .truncate()
                                        .sense(egui::Sense::hover()),
                                );
                            });
                        },
                    );
                    let resp = ui.interact(
                        inner.response.rect,
                        ui.id().with(("file_grid_cell", tile_id, *idx)),
                        egui::Sense::click_and_drag(),
                    );
                    cell_rects.push((*idx, resp.rect));
                    if tab_state.scroll_to_selected && is_selected {
                        resp.scroll_to_me(Some(egui::Align::Center));
                        tab_state.scroll_to_selected = false;
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
                    if entry.kind == FileKind::Dir
                        && resp.contains_pointer()
                        && ui.input(|input| input.pointer.any_released())
                    {
                        drop_dest = Some(entry.path.clone());
                    }
                    if entry.kind == FileKind::Dir
                        && resp.contains_pointer()
                        && drag_navigation_active(&resp.ctx, dragged_files)
                    {
                        drag_hover.observed_target = Some(DragHoverTarget::Folder {
                            tab: tile_id,
                            path: entry.path.clone(),
                        });
                    }
                    if entry.kind == FileKind::Dir {
                        paint_drag_hover_confirmation(
                            &resp,
                            drag_hover,
                            &DragHoverTarget::Folder {
                                tab: tile_id,
                                path: entry.path.clone(),
                            },
                        );
                    }
                    if entry.kind == FileKind::Dir
                        && resp.contains_pointer()
                        && ui.input(|input| !input.raw.hovered_files.is_empty())
                    {
                        *external_drop_target = Some(ExternalDropTarget {
                            tab: tile_id,
                            dest: entry.path.clone(),
                        });
                    }
                }
            });
        }
    });

    update_drag_selection(ui, tab_state, &background, &cell_rects);
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

fn is_preview_image(extension: Option<&str>) -> bool {
    matches!(
        extension.map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif")
    )
}

fn drag_navigation_active(ctx: &egui::Context, dragged_files: &Option<DraggedFiles>) -> bool {
    dragged_files.is_some() || ctx.input(|input| !input.raw.hovered_files.is_empty())
}

fn auto_scroll_during_drag(ui: &egui::Ui, dragged_files: &Option<DraggedFiles>) {
    const EDGE_SIZE: f32 = 52.0;
    const MAX_SPEED: f32 = 760.0;

    if !drag_navigation_active(ui.ctx(), dragged_files) {
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

fn paint_drag_hover_confirmation(
    response: &egui::Response,
    drag_hover: &DragHoverState,
    target: &DragHoverTarget,
) {
    let now = response.ctx.input(|input| input.time);
    if drag_hover.confirmation_visible(target, now) {
        response.ctx.debug_painter().rect_stroke(
            response.rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, response.ctx.style().visuals.selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}

fn finish_drop(
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
                    tab_state.primary_press = None;
                }
                if let Some((tile, Some(idx))) = hit_list(ctx, tab_states, pos) {
                    if let Some(tab_state) = tab_states.get_mut(&tile) {
                        tab_state.primary_press = Some((idx, pos));
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
                        tab_state
                            .last_primary_click
                            .is_some_and(|(previous_idx, previous_time)| {
                                previous_idx == idx
                                    && time - previous_time <= MAX_DOUBLE_CLICK_DELAY
                            })
                    });
                    for tab_state in tab_states.values_mut() {
                        tab_state.last_primary_click = None;
                    }
                    if is_double_click {
                        actions.push(Action::Activate { tile, idx });
                    } else if let Some(tab_state) = tab_states.get_mut(&tile) {
                        tab_state.last_primary_click = Some((idx, time));
                    }
                } else {
                    for tab_state in tab_states.values_mut() {
                        tab_state.last_primary_click = None;
                    }
                }
            }
            (egui::PointerButton::Secondary, true) => {
                if let Some((tile, Some(idx))) = hit_list(ctx, tab_states, pos) {
                    let should_select = tab_states
                        .get(&tile)
                        .is_some_and(|tab_state| !tab_state.selected.contains(&idx));
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

fn hit_list(
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

fn clear_click_tracking(tab_states: &mut HashMap<TileId, TabState>) {
    for tab_state in tab_states.values_mut() {
        tab_state.primary_press = None;
        tab_state.last_primary_click = None;
    }
}

fn record_hit_regions(
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

fn update_drag_selection(
    ui: &egui::Ui,
    tab_state: &mut TabState,
    background: &egui::Response,
    row_rects: &[(usize, egui::Rect)],
) {
    let primary_drag_active = ui.input(|input| {
        input.pointer.button_down(egui::PointerButton::Primary) && input.pointer.has_pointer()
    });
    if tab_state.drag_select_start.is_some() && (!primary_drag_active || !background.dragged()) {
        clear_drag_selection(tab_state);
    }

    if background.drag_started() {
        let origin = ui.input(|input| input.pointer.press_origin());
        if origin.is_some_and(|origin| row_rects.iter().any(|(_, rect)| rect.contains(origin))) {
            return;
        }
        tab_state.drag_select_start = origin;
        tab_state.drag_select_base = if ui.input(|input| input.modifiers.ctrl) {
            tab_state.selected.clone()
        } else {
            Default::default()
        };
    }
    if let (Some(start), Some(pointer)) = (
        tab_state.drag_select_start,
        ui.input(|input| input.pointer.interact_pos()),
    ) {
        let rect = egui::Rect::from_two_pos(start, pointer);
        let mut selected = tab_state.drag_select_base.clone();
        selected.extend(
            row_rects
                .iter()
                .filter(|(_, row_rect)| rect.intersects(*row_rect))
                .map(|(idx, _)| *idx),
        );
        tab_state.selected = selected;
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(
            rect,
            0.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.25),
        );
    }
    if background.drag_stopped() || !primary_drag_active {
        clear_drag_selection(tab_state);
    }
}

fn clear_drag_selection(tab_state: &mut TabState) {
    tab_state.drag_select_start = None;
    tab_state.drag_select_base.clear();
}

fn selected_paths(tab_state: &TabState) -> Vec<PathBuf> {
    tab_state
        .selected
        .iter()
        .filter_map(|idx| tab_state.entries.get(*idx))
        .map(|entry| entry.path.clone())
        .collect()
}

fn show_filter_bar(ui: &mut egui::Ui, tab_state: &mut TabState, visible_count: usize) {
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

fn sort_arrow(label: &str, active: bool, dir: &SortDir) -> String {
    if active {
        let arrow = if *dir == SortDir::Asc { " ↑" } else { " ↓" };
        format!("{label}{arrow}")
    } else {
        label.to_string()
    }
}

fn toggle_dir(currently_active: bool, current: &SortDir) -> SortDir {
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
