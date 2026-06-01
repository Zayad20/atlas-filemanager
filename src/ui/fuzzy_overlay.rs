use crate::input::action::Action;

#[derive(Default)]
pub struct FuzzyOverlayState {
    pub query: String,
    pub selected_idx: usize,
    pub scroll_to_selected: bool,
    /// Incremented each time the overlay opens so the fade-in restarts.
    pub generation: u64,
}

impl FuzzyOverlayState {
    pub fn reset(&mut self) {
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_to_selected = false;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Generic fuzzy overlay panel: shows a search box + scrollable result list.
/// `on_select` converts the selected string item to an Action.
/// `on_close` is the Action pushed when Escape/click-outside closes it.
pub fn show_fuzzy_overlay<T: AsRef<str>>(
    ctx: &egui::Context,
    overlay_id: egui::Id,
    state: &mut FuzzyOverlayState,
    items: &[T],
    close_action: Action,
    on_select: impl Fn(&T) -> Action,
    actions: &mut Vec<Action>,
) {
    let area = egui::Area::new(overlay_id)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 48.0))
        .order(egui::Order::Foreground);

    let fade = crate::ui::anim::animate_bool_eased(
        ctx,
        overlay_id.with(("fade", state.generation)),
        true,
        0.15,
        crate::ui::anim::cubic_out,
    );

    let resp = area.show(ctx, |ui| {
        ui.set_opacity(fade);
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(500.0);
            ui.set_max_width(620.0);

            // Search input — request focus every frame so it stays active
            let edit = egui::TextEdit::singleline(&mut state.query)
                .hint_text("Type to search…")
                .desired_width(f32::INFINITY);
            let text_resp = ui.add(edit);
            text_resp.request_focus();

            // Keyboard nav
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    state.selected_idx =
                        (state.selected_idx + 1).min(items.len().saturating_sub(1));
                    state.scroll_to_selected = true;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    state.selected_idx = state.selected_idx.saturating_sub(1);
                    state.scroll_to_selected = true;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    if let Some(item) = items.get(state.selected_idx) {
                        actions.push(on_select(item));
                        actions.push(close_action.clone());
                    }
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    actions.push(close_action.clone());
                }
            });

            ui.separator();

            // Results list
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, item) in items.iter().enumerate() {
                        let resp = ui.selectable_label(i == state.selected_idx, item.as_ref());
                        if state.scroll_to_selected && i == state.selected_idx {
                            resp.scroll_to_me(None);
                            state.scroll_to_selected = false;
                        }
                        if resp.clicked() {
                            actions.push(on_select(item));
                            actions.push(close_action.clone());
                        }
                    }
                });

            if items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("No results").weak());
                });
            }
        });
    });

    // Close on click outside the overlay
    if resp.response.clicked_elsewhere() {
        actions.push(close_action);
    }
}
