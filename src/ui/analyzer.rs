use std::collections::HashSet;
use std::time::Duration;

use egui::{Color32, RichText, Stroke, Vec2};

use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::space_lens::{PhysicsNode, SpaceItem};
use crate::util::display::{format_size, FOLDER_ICON_COLOR};
use crate::util::extensions::ExtensionRegistry;

const LIST_MIN_WIDTH: f32 = 250.0;
const LIST_MAX_WIDTH: f32 = 340.0;
const CANVAS_MIN_WIDTH: f32 = 360.0;
const ROW_HEIGHT: f32 = 24.0;
const MAX_BUBBLES: usize = 30;

pub fn show_analyzer_window(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    let open = state.overlay.show_analyzer;
    if state.space_lens.active_path.is_none() && open {
        if let Some(path) = state.active_tab_data().map(|tab| tab.path.clone()) {
            state.space_lens.open_for_path(path);
        }
    }
    state.space_lens.update();

    let mut window_open = open;
    let mut close_after_action = false;
    let response = egui::Window::new("Lupa de Espacio (Space Lens)")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(true)
        .default_size([920.0, 620.0])
        .min_size([LIST_MIN_WIDTH + CANVAS_MIN_WIDTH + 36.0, 460.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            show_header(ui, state);
            ui.separator();

            let total_size = state.space_lens.items.iter().map(|item| item.size).sum();
            let available = ui.available_size();
            let list_width = (available.x * 0.34)
                .clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH)
                .min((available.x - CANVAS_MIN_WIDTH).max(LIST_MIN_WIDTH));

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(list_width, available.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| show_item_list(ui, state, actions, &mut close_after_action),
                );
                ui.separator();
                show_bubble_canvas(ui, state, total_size);
            });
        });

    if close_after_action {
        window_open = false;
    }
    if !window_open {
        state.space_lens.close();
    }
    state.overlay.show_analyzer = window_open;

    if state.space_lens.loading {
        ctx.request_repaint_after(Duration::from_millis(80));
    } else if response.is_some_and(|response| response.response.changed()) {
        ctx.request_repaint();
    }
}

fn show_header(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let path = state
            .space_lens
            .active_path
            .as_deref()
            .map(crate::util::display::display_name)
            .unwrap_or_default();
        ui.heading(path);
        if state.space_lens.loading {
            ui.spinner();
            ui.weak("Analizando...");
        } else {
            ui.weak("Listo");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Actualizar").clicked() {
                if let Some(path) = state.space_lens.active_path.clone() {
                    state.space_lens.open_for_path(path);
                }
            }
            ui.weak(format!(
                "{} entradas | {} escaneados",
                state.space_lens.scanned_entries,
                format_size(state.space_lens.scanned_bytes)
            ));
            if state.space_lens.error_count > 0 {
                ui.colored_label(
                    Color32::from_rgb(230, 170, 70),
                    format!("{} errores", state.space_lens.error_count),
                );
            }
        });
    });
}

fn show_item_list(
    ui: &mut egui::Ui,
    state: &mut AppState,
    actions: &mut Vec<Action>,
    close_after_action: &mut bool,
) {
    let total_size: u64 = state.space_lens.items.iter().map(|item| item.size).sum();
    ui.label(
        RichText::new(format!(
            "{} elementos | {}",
            state.space_lens.items.len(),
            format_size(total_size)
        ))
        .weak(),
    );
    ui.add_space(4.0);

    let footer_height = 38.0;
    let scroll_height = (ui.available_height() - footer_height).max(80.0);
    let mut toggles = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("space_lens_items")
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, state.space_lens.items.len(), |ui, range| {
            for item in &state.space_lens.items[range] {
                let selected = state.space_lens.selected_for_deletion.contains(&item.path);
                let mut checked = selected;
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut checked, "").changed() {
                        toggles.push((item.path.clone(), checked));
                    }
                    let (icon, color) = item_icon(item);
                    ui.colored_label(color, icon);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(RichText::new(format_size(item.size)).small());
                        ui.add(egui::Label::new(truncate_chars(&item.name, 30)).truncate());
                    });
                });
            }
        });
    for (path, selected) in toggles {
        if selected {
            state.space_lens.selected_for_deletion.insert(path);
        } else {
            state.space_lens.selected_for_deletion.remove(&path);
        }
    }

    ui.separator();
    let selected_size = state
        .space_lens
        .items
        .iter()
        .filter(|item| state.space_lens.selected_for_deletion.contains(&item.path))
        .map(|item| item.size)
        .sum();
    ui.horizontal(|ui| {
        ui.weak(RichText::new(format!(
            "{} seleccionados | {}",
            state.space_lens.selected_for_deletion.len(),
            format_size(selected_size)
        )));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = egui::Button::new(RichText::new("Mover a papelera").color(Color32::WHITE))
                .fill(Color32::from_rgb(190, 45, 85));
            if ui
                .add_enabled(!state.space_lens.selected_for_deletion.is_empty(), button)
                .clicked()
            {
                actions.push(Action::Delete {
                    paths: state
                        .space_lens
                        .selected_for_deletion
                        .iter()
                        .cloned()
                        .collect(),
                    to_trash: true,
                });
                *close_after_action = true;
            }
        });
    });
}

fn show_bubble_canvas(ui: &mut egui::Ui, state: &mut AppState, total_size: u64) {
    let canvas_size = ui.available_size().max(egui::vec2(1.0, 1.0));
    let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, Color32::from_rgb(25, 16, 42));

    if state.space_lens.items.is_empty() {
        let label = if state.space_lens.loading {
            "Escaneando el directorio..."
        } else {
            "Directorio vacío o sin permisos."
        };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(15.0),
            ui.visuals().weak_text_color(),
        );
        return;
    }

    let moving = run_bubble_physics(state, rect, total_size);
    let pointer_down = ui.input(|input| input.pointer.primary_down());
    let pointer_pressed = pointer_down && !state.space_lens.bubble_pointer_down;
    state.space_lens.bubble_pointer_down = pointer_down;
    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| rect.contains(*pos));
    let clicked_path = if pointer_pressed {
        pointer.and_then(|pointer| {
            state
                .space_lens
                .physics_nodes
                .iter()
                .filter(|(_, node)| pointer.distance(node.pos) <= node.radius)
                .min_by(|(_, left), (_, right)| left.radius.total_cmp(&right.radius))
                .map(|(path, _)| path.clone())
        })
    } else {
        None
    };

    for (path, node) in &state.space_lens.physics_nodes {
        let Some(item) = state
            .space_lens
            .items
            .iter()
            .find(|item| item.path == *path)
        else {
            continue;
        };
        let selected = state.space_lens.selected_for_deletion.contains(path);
        draw_bubble(&painter, node, item, selected);
    }

    if let Some(path) = clicked_path {
        if !state.space_lens.selected_for_deletion.insert(path.clone()) {
            state.space_lens.selected_for_deletion.remove(&path);
        }
        ui.ctx().request_repaint();
    }
    if moving {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

fn draw_bubble(painter: &egui::Painter, node: &PhysicsNode, item: &SpaceItem, selected: bool) {
    if node.radius <= 4.0 {
        return;
    }
    let fill_alpha = if selected { 0.78 } else { 0.28 };
    let stroke_width = if selected { 3.0 } else { 1.5 };
    let stroke_color = if selected { Color32::WHITE } else { node.color };
    painter.circle_filled(node.pos, node.radius, node.color.gamma_multiply(fill_alpha));
    painter.circle_stroke(
        node.pos,
        node.radius,
        Stroke::new(stroke_width, stroke_color),
    );
    painter.circle_stroke(
        node.pos,
        node.radius + 2.0,
        Stroke::new(1.0, node.color.gamma_multiply(0.35)),
    );

    if node.radius < 26.0 {
        return;
    }
    let (icon, _) = item_icon(item);
    painter.text(
        node.pos - Vec2::new(0.0, 11.0),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        Color32::WHITE,
    );
    painter.text(
        node.pos + Vec2::new(0.0, 8.0),
        egui::Align2::CENTER_CENTER,
        truncate_chars(&item.name, 14),
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
    if node.radius >= 42.0 {
        painter.text(
            node.pos + Vec2::new(0.0, 22.0),
            egui::Align2::CENTER_CENTER,
            format_size(item.size),
            egui::FontId::proportional(10.0),
            Color32::WHITE.gamma_multiply(0.75),
        );
    }
}

fn run_bubble_physics(state: &mut AppState, rect: egui::Rect, total_size: u64) -> bool {
    let center = rect.center();
    let colors = [
        Color32::from_rgb(210, 40, 180),
        Color32::from_rgb(180, 40, 210),
        Color32::from_rgb(100, 70, 255),
        Color32::from_rgb(255, 70, 130),
        Color32::from_rgb(90, 160, 255),
    ];
    let max_area = rect.width() * rect.height() * 0.46;
    let max_radius = (rect.width().min(rect.height()) * 0.34).max(8.0);
    let visible_paths: HashSet<_> = state
        .space_lens
        .items
        .iter()
        .take(MAX_BUBBLES)
        .map(|item| item.path.clone())
        .collect();
    state
        .space_lens
        .physics_nodes
        .retain(|path, _| visible_paths.contains(path));

    for (index, item) in state.space_lens.items.iter().take(MAX_BUBBLES).enumerate() {
        let area_ratio = if total_size > 0 {
            item.size as f64 / total_size as f64
        } else {
            0.0
        };
        let radius = ((max_area as f64 * area_ratio) / std::f64::consts::PI)
            .sqrt()
            .max(7.0) as f32;
        let target_radius = radius.min(max_radius);
        let angle = index as f32 * 1.37;
        let distance = (28.0 + index as f32 * 8.0).min(rect.width().min(rect.height()) * 0.28);
        let position = center + egui::vec2(angle.cos(), angle.sin()) * distance;
        let color = colors[index % colors.len()];

        let node = state
            .space_lens
            .physics_nodes
            .entry(item.path.clone())
            .or_insert(PhysicsNode {
                pos: position,
                vel: Vec2::ZERO,
                radius: 1.0,
                target_radius,
                color,
            });
        node.target_radius = target_radius;
        node.color = color;
    }

    let keys: Vec<_> = state.space_lens.physics_nodes.keys().cloned().collect();
    let mut moving = false;
    for _ in 0..5 {
        for key in &keys {
            let node = state.space_lens.physics_nodes.get_mut(key).unwrap();
            let offset = center - node.pos;
            node.vel += offset * 0.001;
            let radius_delta = node.target_radius - node.radius;
            node.radius += radius_delta * 0.12;
            moving |= radius_delta.abs() > 0.2;
        }

        for left in 0..keys.len() {
            for right in (left + 1)..keys.len() {
                let left_key = &keys[left];
                let right_key = &keys[right];
                let left_pos = state.space_lens.physics_nodes[left_key].pos;
                let right_pos = state.space_lens.physics_nodes[right_key].pos;
                let min_distance = state.space_lens.physics_nodes[left_key].radius
                    + state.space_lens.physics_nodes[right_key].radius
                    + 6.0;
                let offset = left_pos - right_pos;
                let distance = offset.length();
                if distance < min_distance && distance > 0.01 {
                    let force = offset.normalized() * ((min_distance - distance) * 0.08);
                    state
                        .space_lens
                        .physics_nodes
                        .get_mut(left_key)
                        .unwrap()
                        .vel += force;
                    state
                        .space_lens
                        .physics_nodes
                        .get_mut(right_key)
                        .unwrap()
                        .vel -= force;
                }
            }
        }

        for key in &keys {
            let node = state.space_lens.physics_nodes.get_mut(key).unwrap();
            node.pos += node.vel;
            node.vel *= 0.78;
            moving |= node.vel.length_sq() > 0.01;

            let radius = node.radius;
            node.pos.x = node
                .pos
                .x
                .clamp(rect.left() + radius, rect.right() - radius);
            node.pos.y = node
                .pos
                .y
                .clamp(rect.top() + radius, rect.bottom() - radius);
        }
    }
    moving
}

fn item_icon(item: &SpaceItem) -> (&'static str, Color32) {
    if item.is_dir {
        return (ExtensionRegistry::icon(None), FOLDER_ICON_COLOR);
    }
    let extension = item
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_lowercase);
    let extension = extension.as_deref();
    (
        ExtensionRegistry::icon(extension),
        ExtensionRegistry::color(extension),
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_chars;

    #[test]
    fn truncation_preserves_unicode_boundaries() {
        assert_eq!(truncate_chars("Imágenes recientes", 8), "Imágenes...");
        assert_eq!(truncate_chars("Música", 8), "Música");
    }
}
