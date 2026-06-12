use crate::input::action::Action;
use crate::fs::preview::{PreviewResult, DIR_PREVIEW_LIMIT};
use crate::state::app_state::AppState;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::preview_cache::MultiPreviewState;
use crate::state::settings::ViewMode;
use crate::state::tab::TabState;
use crate::state::tag_cache::{TagCache, TagId};
use crate::util::display::{format_date, format_size};
use crate::util::extensions::ExtensionRegistry;
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::SystemTime;

thread_local! {
    static MARKDOWN_CACHE: std::cell::RefCell<egui_commonmark::CommonMarkCache> =
        std::cell::RefCell::new(egui_commonmark::CommonMarkCache::default());
}

#[derive(Default)]
pub struct SelectionSummary {
    pub count: usize,
    pub file_count: usize,
    pub dir_count: usize,
    pub image_count: usize,
    pub total_size: u64,
    pub known_size_count: usize,
    pub single: Option<SelectedEntry>,
    /// Paths of every selected entry (for the tag section).
    pub paths: Vec<PathBuf>,
}

pub struct SelectedEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub extension: Option<String>,
}

impl SelectionSummary {
    pub fn for_tab(tab_state: &TabState, view_mode: &ViewMode) -> Self {
        if view_mode == &ViewMode::Miller {
            return tab_state
                .miller
                .columns
                .get(tab_state.miller.focused)
                .and_then(|column| column.entries.get(column.selected?))
                .map(Self::for_entry)
                .unwrap_or_default();
        }

        let mut selected: Vec<usize> = tab_state.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        let entries: Vec<&FileEntry> = selected
            .into_iter()
            .filter_map(|idx| tab_state.entries.get(idx))
            .collect();
        Self::for_entries(&entries)
    }

    fn for_entry(entry: &FileEntry) -> Self {
        Self::for_entries(&[entry])
    }

    fn for_entries(entries: &[&FileEntry]) -> Self {
        let mut summary = Self {
            paths: entries.iter().map(|e| e.path.clone()).collect(),
            ..Self::default()
        };
        for entry in entries {
            summary.count += 1;
            match entry.kind {
                FileKind::Dir => summary.dir_count += 1,
                FileKind::File | FileKind::Symlink => summary.file_count += 1,
            }
            if let Some(size) = entry.size {
                summary.total_size += size;
                summary.known_size_count += 1;
            }
            if entry
                .extension
                .as_deref()
                .is_some_and(crate::fs::preview::is_image_ext)
            {
                summary.image_count += 1;
            }
        }
        if let [entry] = entries {
            summary.single = Some(SelectedEntry {
                name: entry.name.clone(),
                path: entry.path.clone(),
                kind: entry.kind.clone(),
                size: entry.size,
                modified: entry.modified,
                extension: entry.extension.clone(),
            });
        }
        summary
    }
}

pub fn show_inspector(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
    egui::SidePanel::right("inspector")
        .default_width(state.settings.inspector_width)
        .resizable(true)
        .show(ctx, |ui| {
            let summary = state
                .active_tab_state()
                .zip(state.active_tab_data())
                .map(|(tab, data)| SelectionSummary::for_tab(tab, &data.view_mode))
                .unwrap_or_default();
            show_preview(
                ui,
                &summary,
                state.preview.results.get(&state.focused_tile),
                state.preview.textures.get(&state.focused_tile),
                state.preview.multi.get(&state.focused_tile),
                &state.tag_cache,
                actions,
            );
        });
}

#[allow(clippy::too_many_arguments)]
pub fn show_preview(
    ui: &mut egui::Ui,
    summary: &SelectionSummary,
    preview_result: Option<&PreviewResult>,
    preview_texture: Option<&egui::TextureHandle>,
    multi_preview: Option<&MultiPreviewState>,
    tag_cache: &TagCache,
    actions: &mut Vec<Action>,
) {
    if summary.count == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new("No selection").weak());
        });
        return;
    }
    if summary.count > 1 {
        show_multi_selection(ui, summary, multi_preview);
        show_tag_section(ui, tag_cache, &summary.paths, actions);
        return;
    }

    if let Some(entry) = &summary.single {
        show_single_metadata(ui, entry);
        show_tag_section(ui, tag_cache, &summary.paths, actions);
        ui.separator();
    }

    match preview_result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Loading preview...").weak());
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
            let source = promote_nested_fenced_code_blocks(source);
            let viewport_width = ui.available_width();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(viewport_width);
                    ui.shrink_clip_rect(ui.max_rect());
                    MARKDOWN_CACHE.with(|cache| {
                        egui_commonmark::CommonMarkViewer::new()
                            .max_image_width(Some(viewport_width as usize))
                            .default_width(Some(viewport_width as usize))
                            .default_implicit_uri_scheme(base_uri)
                            .show(ui, &mut cache.borrow_mut(), &source);
                    });
                });
        }
        Some(PreviewResult::Image {
            width,
            height,
            original_width,
            original_height,
            exif,
            ..
        }) => {
            ui.weak(format!("{original_width} x {original_height} px"));
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
                                ("📁", crate::util::display::FOLDER_ICON_COLOR)
                            } else {
                                let ext = entry.extension.as_deref();
                                (ExtensionRegistry::icon(ext), ExtensionRegistry::color(ext))
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

fn show_single_metadata(ui: &mut egui::Ui, entry: &SelectedEntry) {
    ui.horizontal(|ui| {
        let (icon, color) = if entry.kind == FileKind::Dir {
            ("📁", crate::util::display::FOLDER_ICON_COLOR)
        } else {
            let ext = entry.extension.as_deref();
            (ExtensionRegistry::icon(ext), ExtensionRegistry::color(ext))
        };
        ui.colored_label(color, egui::RichText::new(icon).size(28.0));
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(&entry.name).strong());
            ui.weak(egui::RichText::new(entry.path.display().to_string()).size(11.0));
        });
    });
    egui::Grid::new("single_preview_metadata")
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            if let Some(size) = entry.size {
                ui.weak("Size");
                ui.label(format_size(size));
                ui.end_row();
            }
            if let Some(modified) = entry.modified {
                ui.weak("Modified");
                ui.label(format_date(modified));
                ui.end_row();
            }
            if let Some(extension) = &entry.extension {
                ui.weak("Type");
                ui.label(extension);
                ui.end_row();
            }
        });
}

/// Tags present on *every* selected path (the ones safe to show as "the selection's tags").
fn common_tags(cache: &TagCache, paths: &[PathBuf]) -> Vec<TagId> {
    let mut iter = paths.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let mut common: Vec<TagId> = cache.tags_of(first).to_vec();
    for p in iter {
        let tags = cache.tags_of(p);
        common.retain(|t| tags.contains(t));
        if common.is_empty() {
            break;
        }
    }
    common
}

/// A colored, optionally-removable tag chip. Returns the click response.
fn tag_chip(ui: &mut egui::Ui, name: &str, color: [u8; 3], removable: bool) -> egui::Response {
    let fill = egui::Color32::from_rgb(color[0], color[1], color[2]);
    // Pick a readable text color from the chip's luminance.
    let lum = 0.299 * color[0] as f32 + 0.587 * color[1] as f32 + 0.114 * color[2] as f32;
    let text_color = if lum > 140.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    let label = if removable {
        format!("{name}  ✕")
    } else {
        name.to_string()
    };
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label, font, text_color);
    let padding = egui::vec2(8.0, 3.0);
    let (rect, resp) =
        ui.allocate_exact_size(galley.size() + padding * 2.0, egui::Sense::click());
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley(rect.min + padding, galley, text_color);
    if removable {
        resp.on_hover_text("Remove tag")
    } else {
        resp
    }
}

/// Tag chips for the current selection + an "add tag" menu. Writes assign/unassign
/// actions; the optimistic cache update makes chips appear/disappear the same frame.
fn show_tag_section(
    ui: &mut egui::Ui,
    cache: &TagCache,
    paths: &[PathBuf],
    actions: &mut Vec<Action>,
) {
    if paths.is_empty() {
        return;
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.weak("Tags");
        let common = common_tags(cache, paths);
        for tag in &common {
            if let Some(def) = cache.def(*tag) {
                if tag_chip(ui, &def.name, def.color, true).clicked() {
                    actions.push(Action::UnassignTagFromFiles {
                        tag: *tag,
                        paths: paths.to_vec(),
                    });
                }
            }
        }
        ui.menu_button("＋", |ui| {
            let mut any = false;
            for def in &cache.defs {
                if common.contains(&def.id) {
                    continue;
                }
                any = true;
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        5.0,
                        egui::Color32::from_rgb(def.color[0], def.color[1], def.color[2]),
                    );
                    if ui.button(&def.name).clicked() {
                        actions.push(Action::AssignTagToFiles {
                            tag: def.id,
                            paths: paths.to_vec(),
                        });
                        ui.close();
                    }
                });
            }
            if !any {
                ui.weak("No more tags");
            }
        })
        .response
        .on_hover_text("Add tag");
    });
}

fn show_multi_selection(
    ui: &mut egui::Ui,
    summary: &SelectionSummary,
    multi_preview: Option<&MultiPreviewState>,
) {
    ui.heading(format!("{} items selected", summary.count));
    ui.horizontal_wrapped(|ui| {
        if summary.file_count > 0 {
            ui.label(format!("{} files", summary.file_count));
        }
        if summary.dir_count > 0 {
            ui.label(format!("{} folders", summary.dir_count));
        }
        if summary.known_size_count > 0 {
            ui.label(format_size(summary.total_size));
        }
    });
    ui.separator();

    if summary.image_count > 0 {
        if let Some(multi) = multi_preview {
            show_thumbnail_stack(ui, summary.count, multi);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Loading thumbnails...").weak());
            });
        }
    }

    if summary.image_count != summary.count {
        ui.vertical_centered(|ui| {
            if summary.image_count == 0 {
                ui.add_space(24.0);
                ui.label(egui::RichText::new("▦").size(48.0).weak());
            }
            ui.weak("Mixed selection");
            if summary.image_count > 0 {
                ui.weak(format!("{} images", summary.image_count));
            }
        });
    }
}

fn show_thumbnail_stack(ui: &mut egui::Ui, selected_count: usize, multi: &MultiPreviewState) {
    if multi.paths.is_empty() {
        return;
    }

    let available_width = ui.available_width();
    let card_width = (available_width * 0.72).clamp(112.0, 260.0);
    let card_height = (card_width * 0.72).clamp(88.0, 190.0);
    let stack_height = card_height + 54.0;
    let (stack_rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, stack_height),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(stack_rect);
    let center = stack_rect.center() + egui::vec2(0.0, 4.0);
    let angles = [-0.025, -0.10, 0.085, -0.145];
    let offsets = [
        egui::vec2(0.0, 8.0),
        egui::vec2(-13.0, 7.0),
        egui::vec2(14.0, 4.0),
        egui::vec2(-5.0, -1.0),
    ];

    // Draw the first selected image last so it remains the clear front card.
    for idx in (0..multi.paths.len()).rev() {
        let path = &multi.paths[idx];
        let card = egui::Rect::from_center_size(
            center + offsets[idx],
            egui::vec2(card_width, card_height),
        );
        let angle = angles[idx];
        paint_rotated_card(
            &painter,
            card,
            angle,
            multi
                .textures
                .iter()
                .find(|(loaded, _)| loaded == path)
                .map(|(_, texture)| texture),
            ui.visuals(),
        );
    }

    if selected_count > multi.paths.len() {
        let extra = selected_count - multi.paths.len();
        let text = format!("+{extra}");
        let galley =
            painter.layout_no_wrap(text, egui::FontId::proportional(12.0), egui::Color32::WHITE);
        let badge_size = galley.size() + egui::vec2(12.0, 8.0);
        let badge = egui::Rect::from_center_size(
            center + egui::vec2(card_width * 0.42, card_height * 0.38),
            badge_size,
        );
        painter.rect_filled(badge, 5.0, egui::Color32::from_black_alpha(205));
        painter.galley(
            badge.min + egui::vec2(6.0, 4.0),
            galley,
            egui::Color32::WHITE,
        );
    }
}

fn paint_rotated_card(
    painter: &egui::Painter,
    card: egui::Rect,
    angle: f32,
    texture: Option<&egui::TextureHandle>,
    visuals: &egui::Visuals,
) {
    let rotation = egui::emath::Rot2::from_angle(angle);
    let corners = rotated_rect_corners(card, rotation);
    let shadow = corners
        .iter()
        .map(|point| *point + egui::vec2(2.0, 3.0))
        .collect::<Vec<_>>();
    painter.add(egui::Shape::convex_polygon(
        shadow,
        egui::Color32::from_black_alpha(70),
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        corners.to_vec(),
        visuals.extreme_bg_color,
        visuals.widgets.noninteractive.bg_stroke,
    ));

    let Some(texture) = texture else {
        painter.text(
            card.center(),
            egui::Align2::CENTER_CENTER,
            "...",
            egui::FontId::proportional(18.0),
            visuals.weak_text_color(),
        );
        return;
    };
    let content = card.shrink(5.0);
    let image_size = texture.size_vec2();
    let scale = (content.width() / image_size.x)
        .min(content.height() / image_size.y)
        .min(1.0);
    let image_rect = egui::Rect::from_center_size(content.center(), image_size * scale);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(
        image_rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    mesh.rotate(rotation, card.center());
    painter.add(mesh);
}

fn rotated_rect_corners(rect: egui::Rect, rotation: egui::emath::Rot2) -> [egui::Pos2; 4] {
    [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
    .map(|point| rect.center() + rotation * (point - rect.center()))
}

/// `egui_commonmark` renders a fenced code block with the full viewer width even
/// when the block is nested inside a list item. Promote only those nested fences
/// so their selectable `TextEdit` cannot overflow from an already-indented x.
fn promote_nested_fenced_code_blocks(source: &str) -> Cow<'_, str> {
    let mut output = String::with_capacity(source.len());
    let mut active: Option<(String, &'static str)> = None;
    let mut changed = false;

    for line in source.split_inclusive('\n') {
        if let Some((indent, marker)) = active.as_ref() {
            let line = line.strip_prefix(indent).unwrap_or(line);
            let closes_fence = line.trim_start().starts_with(marker);
            output.push_str(line);
            changed = true;
            if closes_fence {
                active = None;
            }
            continue;
        }

        let trimmed = line.trim_start_matches([' ', '\t']);
        let indent_len = line.len() - trimmed.len();
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if indent_len > 0 {
            if let Some(marker) = marker {
                output.push_str(trimmed);
                active = Some((line[..indent_len].to_owned(), marker));
                changed = true;
                continue;
            }
        }
        output.push_str(line);
    }

    if changed {
        Cow::Owned(output)
    } else {
        Cow::Borrowed(source)
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

    // Keep room for metadata below the image instead of claiming the entire inspector.
    let avail = ui.available_size();
    let preview_height = avail.y.min((avail.x * 0.9).clamp(160.0, 520.0));
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(avail.x, preview_height),
        egui::Sense::click_and_drag(),
    );

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

#[cfg(test)]
mod tests {
    use super::promote_nested_fenced_code_blocks;

    #[test]
    fn promotes_only_nested_fenced_code_blocks() {
        let source = "- outline\n\n  ```rust\n  fn main() {}\n  ```\n\n```\ntop_level\n```\n";
        let normalized = promote_nested_fenced_code_blocks(source);

        assert_eq!(
            normalized,
            "- outline\n\n```rust\nfn main() {}\n```\n\n```\ntop_level\n```\n"
        );
    }
}
