use egui_tiles::TileId;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::FileKind;
use crate::state::tab::TabState;
use crate::ui::file_view_common::render_grid;

/// Compact multi-column list view: small `icon + name` rows that flow row-major and
/// wrap into as many columns as fit the width. Shares all interaction (selection,
/// rubber-band, drag&drop, context menu) with the Icons view via [`render_grid`].
#[allow(clippy::too_many_arguments)]
pub fn show_file_list_compact(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &std::path::Path,
    tab_state: &mut TabState,
    dragged_files: &mut Option<DraggedFiles>,
    drag_hover: &mut DragHoverState,
    external_drop_target: &mut Option<ExternalDropTarget>,
    actions: &mut Vec<Action>,
) {
    let item_size = egui::vec2(220.0, 22.0);
    let spacing = egui::vec2(6.0, 2.0);

    render_grid(
        ui,
        tile_id,
        pane_path,
        tab_state,
        item_size,
        spacing,
        egui::Layout::left_to_right(egui::Align::Center),
        dragged_files,
        drag_hover,
        external_drop_target,
        actions,
        |ui, entry, _is_selected| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let is_dir = entry.kind == FileKind::Dir;
            crate::ui::os_icon::show_file_icon(ui, entry.extension.as_deref(), is_dir, 16.0);
            ui.add(
                egui::Label::new(&entry.name)
                    .truncate()
                    .sense(egui::Sense::hover()),
            );
        },
    );
}
