use egui_tiles::TileId;

use crate::input::action::Action;
use crate::state::drag::{DragHoverState, DraggedFiles, ExternalDropTarget};
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::tab::TabState;
use crate::ui::file_view_common::{is_preview_image, render_grid};

const THUMBNAIL_MAX_BYTES: u64 = 40 * 1024 * 1024;

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

    // Skip thumbnail prefetch while the pane is being actively resized —
    // last_pane_width is updated by render_grid at the end of each frame,
    // so it still holds the previous frame's width here.
    let is_resizing = (ui.available_width() - tab_state.last_pane_width).abs() > 0.5;
    if !is_resizing {
        prefetch_adjacent_thumbnails(ui.ctx(), tab_state, thumbnail_size);
    }

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

fn prefetch_adjacent_thumbnails(
    ctx: &egui::Context,
    tab_state: &mut TabState,
    thumbnail_size: f32,
) {
    const PREFETCH_ROWS: usize = 2;

    let Some(regions) = tab_state.file_list_hit_regions.as_ref() else {
        return;
    };
    let Some(first_idx) = regions.item_rects.first().map(|(idx, _)| *idx) else {
        return;
    };
    let Some(last_idx) = regions.item_rects.last().map(|(idx, _)| *idx) else {
        return;
    };
    let columns = tab_state.grid_columns.max(1);
    let padding = columns * PREFETCH_ROWS;
    let visible_indices = tab_state.visible_indices_shared();
    let (first, last) = if tab_state.filter.is_empty() {
        (first_idx, last_idx)
    } else {
        let Some(first) = visible_indices.iter().position(|idx| *idx == first_idx) else {
            return;
        };
        let Some(last) = visible_indices.iter().position(|idx| *idx == last_idx) else {
            return;
        };
        (first, last)
    };

    let before = first.saturating_sub(padding)..first;
    let after = (last + 1)..(last + 1 + padding).min(visible_indices.len());
    // Queue far cells first. The loader is latest-first, so the cells directly
    // adjacent to the viewport run before more speculative work.
    for pos in after.rev().chain(before) {
        let entry = &tab_state.entries[visible_indices[pos]];
        if is_thumbnail_previewable(entry) {
            crate::ui::thumbnail_loader::prefetch(
                ctx,
                &entry.path,
                thumbnail_size,
                entry.modified,
                entry.size,
            );
        }
    }
}

fn is_thumbnail_previewable(entry: &FileEntry) -> bool {
    is_preview_image(entry.extension.as_deref())
        && entry.path.is_absolute()
        && entry.size.is_none_or(|size| size <= THUMBNAIL_MAX_BYTES)
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

            // Only decode real, reasonably-sized files. Access-mode remote entries
            // carry relative paths (no real file behind them) so a thumbnail load
            // would fail/spin forever — show the icon instead. The size cap stops
            // huge images from being decoded as thumbnails (and throttles slow
            // FUSE-mounted cloud dirs full of large photos).
            if is_thumbnail_previewable(entry) {
                let uri = crate::ui::thumbnail_loader::uri(
                    &entry.path,
                    thumbnail_size,
                    entry.modified,
                    entry.size,
                );
                let image_size = preview_size - egui::Vec2::splat(PADDING * 2.0);
                ui.add(
                    egui::Image::new(uri)
                        .fit_to_exact_size(image_size)
                        .corner_radius(3),
                );
            } else {
                let is_dir = entry.kind == FileKind::Dir;
                let icon_display = preview_size - egui::Vec2::splat(PADDING * 2.0);
                crate::ui::os_icon::show_file_icon_large(
                    ui,
                    entry.extension.as_deref(),
                    is_dir,
                    thumbnail_size,
                    icon_display,
                );
            }
        },
    );
}
