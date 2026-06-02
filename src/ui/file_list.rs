use egui::Layout;
use egui_extras::{Column, TableBuilder};
use egui_tiles::TileId;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DragHoverTarget, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::{FileKind, SortColumn, SortDir};
use crate::state::settings::ViewMode;
use crate::state::tab::TabState;
use crate::ui::drag_feedback::paint_drag_hover_confirmation;
use crate::util::display::{format_date, format_size};
use crate::util::extensions::ExtensionRegistry;

use crate::ui::file_grid::show_file_grid;
use crate::ui::file_list_compact::show_file_list_compact;
use crate::ui::file_miller::show_file_miller;
use crate::ui::file_view_common::{
    auto_scroll_during_drag, drag_navigation_active, finish_drop, record_hit_regions,
    rubber_band_active, selected_paths, show_filter_bar, sort_arrow, toggle_dir,
    update_drag_selection,
};
use crate::ui::view_layout::GridLayout;

const DETAILS_SELECTION_GUTTER_WIDTH: f32 = 24.0;

#[allow(clippy::too_many_arguments)]
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
    match view_mode {
        ViewMode::Grid => {
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
        ViewMode::List => {
            show_file_list_compact(
                ui,
                tile_id,
                pane_path,
                tab_state,
                dragged_files,
                drag_hover,
                external_drop_target,
                actions,
            );
            return;
        }
        ViewMode::Miller => {
            show_file_miller(
                ui,
                tile_id,
                pane_path,
                tab_state,
                dragged_files,
                drag_hover,
                external_drop_target,
                actions,
            );
            return;
        }
        ViewMode::Details => {}
    }

    let row_height = 20.0;
    let visible_indices: Vec<usize> = tab_state.visible_indices().to_vec();
    tab_state.grid_columns = 1;
    let scroll_active =
        drag_navigation_active(ui.ctx(), dragged_files) || rubber_band_active(tab_state);

    // Snapshot selected and entries for use in closures
    let selected_snapshot = tab_state.selection.selected.clone();
    let list_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        list_rect,
        ui.id().with(("file_list_background", tile_id)),
        egui::Sense::click_and_drag(),
    );
    let mut row_rects = Vec::new();
    let mut origin_sample: Option<(usize, egui::Rect)> = None;
    let mut drop_dest = None;
    let max_scroll_height = (ui.available_height() - 28.0).max(80.0);
    let table_row_spacing = ui.spacing().item_spacing.y;

    TableBuilder::new(ui)
        .id_salt(("details_table", tile_id))
        .striped(true)
        .resizable(true)
        .drag_to_scroll(false)
        .cell_layout(Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(DETAILS_SELECTION_GUTTER_WIDTH).resizable(false)) // marquee gutter
        .column(Column::exact(24.0)) // icon
        .column(Column::remainder().clip(true)) // name
        .column(Column::initial(72.0).range(50.0..=140.0)) // size
        .column(Column::initial(136.0).range(80.0..=200.0)) // modified
        .column(Column::initial(60.0).range(40.0..=120.0)) // type
        .column(Column::exact(DETAILS_SELECTION_GUTTER_WIDTH).resizable(false)) // marquee gutter
        .max_scroll_height(max_scroll_height)
        .header(row_height, |mut header| {
            header.col(|_| {});
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
            header.col(|_| {});
        })
        .body(|mut body| {
            auto_scroll_during_drag(body.ui_mut(), scroll_active);
            body.rows(row_height, visible_indices.len(), |mut row| {
                let row_pos = row.index();
                let idx = visible_indices[row_pos];
                let entry = &tab_state.entries[idx];
                let is_selected = selected_snapshot.contains(&idx);
                row.set_selected(is_selected);

                let cell_sense = egui::Sense::click_and_drag();
                let mut cell_responses = Vec::with_capacity(5);

                // Leave this cell without an interaction so a drag can start a
                // marquee even when every visible row fills the viewport.
                row.col(|_| {});

                // Icon column
                row.col(|ui| {
                    let icon = ExtensionRegistry::icon(entry.extension.as_deref());
                    let color = if entry.kind == FileKind::Dir {
                        crate::util::display::FOLDER_ICON_COLOR
                    } else {
                        ExtensionRegistry::color(entry.extension.as_deref())
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

                // Same neutral marquee gutter on the right edge.
                row.col(|_| {});

                let mut responses = cell_responses.into_iter();
                let mut resp = responses.next().expect("details rows have cells");
                for cell_resp in responses {
                    resp = resp.union(cell_resp);
                }
                row_rects.push((idx, resp.rect));
                origin_sample.get_or_insert((row_pos, resp.rect));
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

    let layout = GridLayout::new(
        egui::vec2(
            origin_sample
                .map(|(_, rect)| rect.width())
                .unwrap_or_default(),
            row_height,
        ),
        egui::vec2(0.0, table_row_spacing),
        1,
        visible_indices.len(),
    );
    let content_origin = origin_sample.map(|(pos, rect)| layout.content_origin(pos, rect));
    update_drag_selection(
        ui,
        tab_state,
        &background,
        &layout,
        content_origin,
        &visible_indices,
    );
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
