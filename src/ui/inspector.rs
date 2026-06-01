use crate::fs::preview::{PreviewResult, DIR_PREVIEW_LIMIT};
use crate::state::app_state::AppState;
use crate::util::display::{extension_color, extension_icon, format_size};

thread_local! {
    static MARKDOWN_CACHE: std::cell::RefCell<egui_commonmark::CommonMarkCache> =
        std::cell::RefCell::new(egui_commonmark::CommonMarkCache::default());
}

pub fn show_inspector(ctx: &egui::Context, state: &AppState) {
    egui::SidePanel::right("inspector")
        .default_width(state.settings.inspector_width)
        .resizable(true)
        .show(ctx, |ui| {
            show_preview(
                ui,
                state.preview_results.get(&state.focused_tile),
                state.preview_textures.get(&state.focused_tile),
            );
        });
}

pub fn show_preview(
    ui: &mut egui::Ui,
    preview_result: Option<&PreviewResult>,
    preview_texture: Option<&egui::TextureHandle>,
) {
    match preview_result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("No selection").weak());
            });
        }
        Some(PreviewResult::Text { lines }) => {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                    let mut current_line = Vec::new();
                    for (color, token) in lines {
                        if token == "\n" {
                            ui.horizontal(|ui| {
                                for (c, t) in &current_line {
                                    ui.colored_label(*c, *t);
                                }
                            });
                            current_line.clear();
                        } else if *color == egui::Color32::TRANSPARENT {
                            // skip transparent tokens
                        } else {
                            current_line.push((*color, token.as_str()));
                        }
                    }
                    if !current_line.is_empty() {
                        ui.horizontal(|ui| {
                            for (c, t) in &current_line {
                                ui.colored_label(*c, *t);
                            }
                        });
                    }
                });
        }
        Some(PreviewResult::Markdown { source, base_uri }) => {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    MARKDOWN_CACHE.with(|cache| {
                        egui_commonmark::CommonMarkViewer::new()
                            .max_image_width(Some(ui.available_width() as usize))
                            .default_implicit_uri_scheme(base_uri)
                            .show(ui, &mut cache.borrow_mut(), source);
                    });
                });
        }
        Some(PreviewResult::Image {
            width,
            height,
            exif,
            ..
        }) => {
            show_image(ui, preview_texture, *width, *height);
            if !exif.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("exif_table")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            for (key, val) in exif {
                                ui.label(key);
                                ui.label(val);
                                ui.end_row();
                            }
                        });
                });
            }
        }
        Some(PreviewResult::Pdf { width, height, .. }) => {
            ui.label(egui::RichText::new("PDF preview - first page").weak());
            ui.separator();
            show_image(ui, preview_texture, *width, *height);
        }
        Some(PreviewResult::Dir {
            entries,
            total_count,
            total_size,
        }) => {
            ui.horizontal(|ui| {
                ui.label(format!("{total_count} items"));
                ui.weak("·");
                ui.weak(format_size(*total_size));
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in entries {
                        ui.horizontal(|ui| {
                            let (icon, color) = if entry.is_dir {
                                ("📁", egui::Color32::from_rgb(200, 180, 80))
                            } else {
                                let ext = entry.extension.as_deref();
                                (extension_icon(ext), extension_color(ext))
                            };
                            ui.colored_label(color, icon);
                            ui.label(egui::RichText::new(&entry.name).size(12.0));
                            if let Some(size) = entry.size {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.weak(egui::RichText::new(format_size(size)).size(11.0));
                                    },
                                );
                            }
                        });
                    }
                    if *total_count > DIR_PREVIEW_LIMIT {
                        ui.weak(format!("… and {} more", total_count - DIR_PREVIEW_LIMIT));
                    }
                });
        }
        Some(PreviewResult::Binary { size }) => {
            ui.centered_and_justified(|ui| {
                ui.label(format!("Binary file ({})", format_size(*size)));
            });
        }
        Some(PreviewResult::Error(e)) => {
            ui.colored_label(egui::Color32::RED, e);
        }
    }
}

/// Interactive zoom/pan state for the image preview, kept in egui temp memory
/// keyed by the widget `Id`. `zoom == 1.0` means "fit to the available area".
#[derive(Clone, Copy)]
struct ImageView {
    zoom: f32,
    offset: egui::Vec2,
    tex: egui::TextureId,
}

const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 32.0;

fn show_image(
    ui: &mut egui::Ui,
    preview_texture: Option<&egui::TextureHandle>,
    width: usize,
    height: usize,
) {
    let Some(handle) = preview_texture else {
        return;
    };
    let tex_id = handle.id();
    let img_size = egui::vec2((width as f32).max(1.0), (height as f32).max(1.0));

    // Claim all the remaining space and let it act as a zoom/pan surface.
    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());

    // Scale that makes the image fit the area (never upscales past 1:1 at zoom 1).
    let fit_scale = (rect.width() / img_size.x)
        .min(rect.height() / img_size.y)
        .min(1.0);

    // Load (or reset, when a different image is now shown) the view state.
    let id = response.id;
    let mut view = ui
        .data(|d| d.get_temp::<ImageView>(id))
        .filter(|v| v.tex == tex_id)
        .unwrap_or(ImageView {
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
            tex: tex_id,
        });

    // Double-click resets back to the fitted view.
    if response.double_clicked() {
        view.zoom = 1.0;
        view.offset = egui::Vec2::ZERO;
    }

    // Wheel / pinch zoom, anchored on the cursor so it zooms "into" what's there.
    if response.hovered() {
        let cursor = response.hover_pos().unwrap_or_else(|| rect.center());
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        let pinch = ui.input(|i| i.zoom_delta());
        let factor = (scroll * 0.005).exp() * pinch;
        if (factor - 1.0).abs() > f32::EPSILON {
            let new_zoom = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            let k = new_zoom / view.zoom;
            // Keep the pixel under the cursor anchored: offset' = d(1-k) + offset*k.
            let d = cursor - rect.center();
            view.offset = d * (1.0 - k) + view.offset * k;
            view.zoom = new_zoom;
        }
    }

    // Drag to pan (only meaningful when zoomed in).
    if response.dragged() {
        view.offset += response.drag_delta();
    }

    // Displayed image rectangle, clamped so its edges never slide inside the area.
    let disp = img_size * fit_scale * view.zoom;
    let max_off = ((disp - rect.size()) * 0.5).max(egui::Vec2::ZERO);
    view.offset = egui::vec2(
        view.offset.x.clamp(-max_off.x, max_off.x),
        view.offset.y.clamp(-max_off.y, max_off.y),
    );
    let image_rect = egui::Rect::from_center_size(rect.center() + view.offset, disp);

    // Clip to the reserved area so a zoomed image can't paint over the EXIF table.
    let painter = ui.painter_at(rect);
    painter.image(
        tex_id,
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    // Hint the panning affordance and surface the current zoom level.
    if view.zoom > 1.0 {
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let text = format!("{:.0}%", view.zoom * 100.0);
        let pos = rect.right_top() + egui::vec2(-6.0, 6.0);
        let galley =
            painter.layout_no_wrap(text, egui::FontId::proportional(12.0), egui::Color32::WHITE);
        let bg = egui::Rect::from_min_size(
            pos - egui::vec2(galley.size().x, 0.0) - egui::vec2(4.0, 2.0),
            galley.size() + egui::vec2(8.0, 4.0),
        );
        painter.rect_filled(bg, 3.0, egui::Color32::from_black_alpha(160));
        painter.galley(bg.min + egui::vec2(4.0, 2.0), galley, egui::Color32::WHITE);
    }

    ui.data_mut(|d| d.insert_temp(id, view));
}
