//! Extensions manager window + the prompt modal for parameterized commands.

use crate::app::ext::ExtAction;
use crate::input::action::Action;
use crate::state::app_state::AppState;

fn ext(a: ExtAction) -> Action {
    Action::Ext(a)
}

/// The "Extensions" manager: list installed, enable/disable, install, uninstall.
pub fn show_manager(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    if !state.ext.show_manager {
        return;
    }
    struct Row {
        id: String,
        name: String,
        version: String,
        description: String,
        kind: String,
        enabled: bool,
        builtin: bool,
    }
    let rows: Vec<Row> = state
        .ext
        .registry
        .extensions
        .iter()
        .map(|e| Row {
            id: e.manifest.id.clone(),
            name: e.manifest.name.clone(),
            version: e.manifest.version.clone(),
            description: e.manifest.description.clone(),
            kind: format!("{:?}", e.manifest.kind).to_lowercase(),
            enabled: e.enabled,
            builtin: e.builtin,
        })
        .collect();

    let mut open = true;
    egui::Window::new("Extensions")
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{}  Install…", egui_phosphor::regular::PLUS))
                    .on_hover_text("Install from a .zip bundle or a folder")
                    .clicked()
                {
                    actions.push(ext(ExtAction::Install));
                }
                if ui.button("Open folder").clicked() {
                    let _ = open::that_detached(crate::ext::extensions_dir());
                }
            });
            if let Some(fb) = &state.ext.feedback {
                ui.colored_label(egui::Color32::from_rgb(120, 180, 120), fb);
            }
            if let Some(err) = &state.ext.last_error {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
            }
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for row in rows {
                    ui.horizontal(|ui| {
                        let mut enabled = row.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            actions.push(ext(ExtAction::SetEnabled {
                                id: row.id.clone(),
                                enabled,
                            }));
                        }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&row.name);
                                ui.label(
                                    egui::RichText::new(format!("v{} · {}", row.version, row.kind))
                                        .small()
                                        .weak(),
                                );
                                if row.builtin {
                                    ui.label(egui::RichText::new("builtin").small().weak());
                                }
                            });
                            if !row.description.is_empty() {
                                ui.label(egui::RichText::new(&row.description).small());
                            }
                        });
                        if !row.builtin {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(egui_phosphor::regular::TRASH)
                                        .on_hover_text("Uninstall")
                                        .clicked()
                                    {
                                        actions.push(ext(ExtAction::Uninstall(row.id.clone())));
                                    }
                                },
                            );
                        }
                    });
                    ui.separator();
                }
            });
        });
    if !open {
        state.ext.show_manager = false;
        state.ext.feedback = None;
        state.ext.last_error = None;
    }
}

/// Modal collecting prompt values for a command, shown before it runs.
pub fn show_prompt(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    let Some(pending) = state.ext.pending.as_mut() else {
        return;
    };
    let title = pending.cmd.command.title.clone();
    let count = pending.paths.len();
    let mut open = true;
    let mut submitted = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("{count} item(s) selected"))
                    .small()
                    .weak(),
            );
            egui::Grid::new("ext_prompt_grid").num_columns(2).show(ui, |ui| {
                for (_, label, value) in pending.answers.iter_mut() {
                    ui.label(label.as_str());
                    ui.text_edit_singleline(value);
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    submitted = true;
                }
                if ui.button("Cancel").clicked() {
                    actions.push(ext(ExtAction::CancelPending));
                }
            });
        });
    if submitted {
        actions.push(ext(ExtAction::ConfirmPending));
    } else if !open {
        actions.push(ext(ExtAction::CancelPending));
    }
}
