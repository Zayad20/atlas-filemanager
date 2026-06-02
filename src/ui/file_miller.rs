use egui_tiles::TileId;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::FileKind;
use crate::state::tab::{LoadState, TabState};
use crate::ui::file_view_common::{finish_drop, handle_cell_drop_targets};
use crate::util::extensions::ExtensionRegistry;

const COLUMN_WIDTH: f32 = 240.0;

/// Miller (cascading) columns view. Each column lists one directory; selecting a
/// folder opens its contents in the next column. A selected file drives the shared
/// inspector preview. Selection here is single-item per column (standard Finder
/// behaviour), so it does not use the rubber-band machinery.
#[allow(clippy::too_many_arguments)]
pub fn show_file_miller(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    tab_state: &mut TabState,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
) {
    // Miller does its own hit-testing, so the global pointer handler must ignore
    // this tab.
    tab_state.file_list_hit_regions = None;

    // Lazily (re)initialise the first column from the current pane path.
    let needs_init = tab_state
        .miller
        .columns
        .first()
        .map(|c| c.path.as_path())
        != Some(pane_path);
    if needs_init {
        actions.push(Action::MillerInit {
            tile: tile_id,
            path: pane_path.to_path_buf(),
        });
        ui.label("Loading…");
        return;
    }

    let focused = tab_state.miller.focused;
    let want_scroll = tab_state.selection.scroll_to_selected;
    let mut drop_dest = None;
    let mut scrolled = false;
    let full_height = ui.available_height();

    let list_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        list_rect,
        ui.id().with(("file_miller_background", tile_id)),
        egui::Sense::click(),
    );

    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let column_count = tab_state.miller.columns.len();
                for c in 0..column_count {
                    ui.allocate_ui(egui::vec2(COLUMN_WIDTH, full_height), |ui| {
                        ui.set_min_height(full_height);
                        render_column(
                            ui,
                            tile_id,
                            c,
                            focused,
                            want_scroll,
                            tab_state,
                            dragged_files,
                            drag_hover,
                            external_drop_target,
                            actions,
                            &mut drop_dest,
                            &mut scrolled,
                        );
                    });
                    ui.separator();
                }
            });
        });

    if scrolled {
        tab_state.selection.scroll_to_selected = false;
    }

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
}

#[allow(clippy::too_many_arguments)]
fn render_column(
    ui: &mut egui::Ui,
    tile_id: TileId,
    column: usize,
    focused: usize,
    want_scroll: bool,
    tab_state: &TabState,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
    drop_dest: &mut Option<std::path::PathBuf>,
    scrolled: &mut bool,
) {
    let Some(col) = tab_state.miller.columns.get(column) else {
        return;
    };
    if col.load_state == LoadState::Loading && col.entries.is_empty() {
        ui.label("Loading…");
        return;
    }
    if col.entries.is_empty() {
        ui.weak("Empty");
        return;
    }
    let selected_idx = col.selected;
    egui::ScrollArea::vertical()
        .id_salt(("miller_col", tile_id, column))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let row_size = egui::vec2(ui.available_width(), 20.0);
            for (idx, entry) in col.entries.iter().enumerate() {
                let is_selected = selected_idx == Some(idx);
                let icon = ExtensionRegistry::icon(entry.extension.as_deref());
                let color = if entry.kind == FileKind::Dir {
                    crate::util::display::FOLDER_ICON_COLOR
                } else {
                    ExtensionRegistry::color(entry.extension.as_deref())
                };
                let is_dir = entry.kind == FileKind::Dir;
                let inner = ui.allocate_ui_with_layout(
                    row_size,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if is_selected {
                            ui.painter().rect_filled(
                                ui.max_rect(),
                                3.0,
                                ui.visuals().selection.bg_fill,
                            );
                        }
                        ui.add_space(4.0);
                        ui.colored_label(color, icon);
                        ui.add(egui::Label::new(&entry.name).truncate().selectable(false));
                        if is_dir {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add_space(4.0);
                                    ui.weak("›");
                                },
                            );
                        }
                    },
                );
                let resp = ui.interact(
                    inner.response.rect,
                    ui.id().with(("miller_row", tile_id, column, idx)),
                    egui::Sense::click_and_drag(),
                );

                if want_scroll && column == focused && is_selected {
                    resp.scroll_to_me(Some(egui::Align::Center));
                    *scrolled = true;
                }
                if resp.clicked() {
                    actions.push(Action::MillerSelect {
                        tile: tile_id,
                        column,
                        idx,
                    });
                }
                if resp.double_clicked() {
                    actions.push(Action::MillerActivate {
                        tile: tile_id,
                        column,
                        idx,
                    });
                }
                if resp.drag_started() {
                    *dragged_files = Some(DraggedFiles {
                        source_tab: tile_id,
                        paths: vec![entry.path.clone()],
                    });
                }
                handle_cell_drop_targets(
                    &resp,
                    tile_id,
                    entry,
                    dragged_files,
                    drag_hover,
                    external_drop_target,
                    drop_dest,
                );
                resp.context_menu(|ui| {
                    crate::ui::context_menu::show_context_menu(
                        ui,
                        tile_id,
                        col.path.as_path(),
                        std::slice::from_ref(&entry.path),
                        actions,
                    );
                });
            }
        });
}
