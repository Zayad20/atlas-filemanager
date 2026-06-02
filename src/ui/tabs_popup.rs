use crate::input::action::Action;
use crate::state::app_state::AppState;

#[derive(Default)]
pub struct TabsPopupState {
    pub query: String,
    pub open: bool,
}

pub fn show_tabs_popup(
    ctx: &egui::Context,
    state: &AppState,
    popup: &mut TabsPopupState,
    actions: &mut Vec<Action>,
) {
    if !popup.open {
        return;
    }

    egui::Area::new(egui::Id::new("tabs_popup"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 48.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(500.0);
                ui.heading("Open Tabs");
                ui.separator();

                let edit = egui::TextEdit::singleline(&mut popup.query)
                    .hint_text("Filter tabs…")
                    .desired_width(f32::INFINITY);
                let resp = ui.add(edit);
                resp.request_focus();

                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    popup.open = false;
                }

                let query_lower = popup.query.to_lowercase();

                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        // List all open pane tiles
                        for (id, tile) in state.pane_tree.tiles.iter() {
                            if let egui_tiles::Tile::Pane(pane_data) = tile {
                                let path_str = pane_data.path.to_string_lossy();
                                if !query_lower.is_empty()
                                    && !path_str.to_lowercase().contains(&query_lower)
                                {
                                    continue;
                                }
                                let label = crate::util::display::display_name(&pane_data.path);
                                let is_active = *id == state.focused_tile;
                                let resp = ui
                                    .selectable_label(is_active, &label)
                                    .on_hover_text(path_str.as_ref());
                                if resp.clicked() {
                                    actions.push(Action::FocusTab(*id));
                                    popup.open = false;
                                }
                            }
                        }
                    });
            });
        });
}
