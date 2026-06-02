use crate::input::action::Action;

use crate::state::batch_rename::{BatchRenameState, RenameMode};

pub fn show_batch_rename(
    ctx: &egui::Context,
    state: &mut BatchRenameState,
    open: &mut bool,
    actions: &mut Vec<Action>,
) {
    let modal = egui::Modal::new(egui::Id::new("batch_rename")).show(ctx, |ui| {
        ui.heading("Batch Rename");
        ui.add_space(8.0);

        // Mode selector
        egui::ComboBox::from_label("Mode")
            .selected_text(state.mode.to_string())
            .show_ui(ui, |ui| {
                for mode in [
                    RenameMode::Find,
                    RenameMode::Prefix,
                    RenameMode::Suffix,
                    RenameMode::Number,
                ] {
                    let label = mode.to_string();
                    if ui.selectable_label(state.mode == mode, label).clicked() {
                        state.mode = mode;
                        state.update_previews();
                    }
                }
            });

        // Pattern inputs
        ui.horizontal(|ui| match state.mode {
            RenameMode::Find => {
                ui.label("Find:");
                if ui.text_edit_singleline(&mut state.pattern).changed() {
                    state.update_previews();
                }
                ui.label("Replace:");
                if ui.text_edit_singleline(&mut state.replacement).changed() {
                    state.update_previews();
                }
            }
            RenameMode::Prefix => {
                ui.label("Prefix:");
                if ui.text_edit_singleline(&mut state.pattern).changed() {
                    state.update_previews();
                }
            }
            RenameMode::Suffix => {
                ui.label("Suffix (before ext):");
                if ui.text_edit_singleline(&mut state.pattern).changed() {
                    state.update_previews();
                }
            }
            RenameMode::Number => {
                ui.label("Separator:");
                if ui.text_edit_singleline(&mut state.replacement).changed() {
                    state.update_previews();
                }
            }
        });

        ui.add_space(8.0);
        ui.label("Preview:");

        // Preview table
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                egui::Grid::new("rename_preview")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (path, new_name) in state.files.iter().zip(state.previews.iter()) {
                            let old_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            ui.label(&old_name);
                            let color = if old_name == *new_name {
                                egui::Color32::GRAY
                            } else {
                                egui::Color32::GREEN
                            };
                            ui.colored_label(color, new_name);
                            ui.end_row();
                        }
                    });
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Rename").clicked() {
                for (path, new_name) in state.files.iter().zip(state.previews.iter()) {
                    if let Some(parent) = path.parent() {
                        let new_path = parent.join(new_name);
                        if new_path != *path {
                            actions.push(Action::Rename {
                                from: path.clone(),
                                to: new_path,
                            });
                        }
                    }
                }
                *open = false;
            }
            if ui.button("Cancel").clicked() {
                *open = false;
            }
        });
    });

    if modal.should_close() {
        *open = false;
    }
}
