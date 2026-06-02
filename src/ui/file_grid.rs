use egui_tiles::TileId;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::tab::TabState;
use crate::ui::file_view_common::{is_preview_image, render_grid};
use crate::util::extensions::ExtensionRegistry;

#[allow(clippy::too_many_arguments)]
pub fn show_file_grid(
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
    let cell_size = egui::vec2(thumbnail_size + 12.0, thumbnail_size + 32.0);
    let preview_size = egui::vec2(thumbnail_size, thumbnail_size);
    let spacing = egui::vec2(8.0, 8.0);

    render_grid(
        ui,
        tile_id,
        pane_path,
        tab_state,
        cell_size,
        spacing,
        egui::Layout::top_down(egui::Align::Center),
        dragged_files,
        drag_hover,
        external_drop_target,
        actions,
        |ui, entry, _is_selected| {
            ui.vertical_centered(|ui| {
                show_thumbnail(ui, entry, preview_size, thumbnail_size);
                ui.add(
                    egui::Label::new(&entry.name)
                        .truncate()
                        .sense(egui::Sense::hover()),
                );
            });
        },
    );
}

fn show_thumbnail(
    ui: &mut egui::Ui,
    entry: &FileEntry,
    preview_size: egui::Vec2,
    thumbnail_size: f32,
) {
    const PADDING: f32 = 6.0;

    ui.allocate_ui_with_layout(
        preview_size,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.set_min_size(preview_size);

            let rect = ui.max_rect();
            ui.painter().rect(
                rect,
                4.0,
                ui.visuals().faint_bg_color,
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                egui::StrokeKind::Inside,
            );

            if is_preview_image(entry.extension.as_deref()) {
                let uri = format!("file://{}", entry.path.to_string_lossy());
                let image_size = preview_size - egui::Vec2::splat(PADDING * 2.0);
                ui.add(
                    egui::Image::new(uri)
                        .fit_to_exact_size(image_size)
                        .corner_radius(3),
                );
            } else {
                let icon = ExtensionRegistry::icon(entry.extension.as_deref());
                let color = if entry.kind == FileKind::Dir {
                    crate::util::display::FOLDER_ICON_COLOR
                } else {
                    ExtensionRegistry::color(entry.extension.as_deref())
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(icon)
                            .size((thumbnail_size * 0.32).max(24.0))
                            .color(color),
                    )
                    .selectable(false),
                );
            }
        },
    );
}
