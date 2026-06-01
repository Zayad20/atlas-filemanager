use crate::state::settings::{InspectorPlacement, Settings, ViewMode};

pub fn show_settings(ctx: &egui::Context, settings: &mut Settings, open: &mut bool) {
    egui::Window::new("Settings")
        .open(open)
        .resizable(true)
        .default_width(480.0)
        .collapsible(false)
        .show(ctx, |ui| {
            egui::CollapsingHeader::new("General")
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut settings.show_hidden, "Show hidden files (Ctrl+.)");
                    ui.checkbox(&mut settings.show_sidebar, "Show sidebar (Ctrl+B)");
                    ui.checkbox(&mut settings.show_inspector, "Enable preview for new tabs");
                    ui.horizontal(|ui| {
                        ui.label("Inspector placement:");
                        egui::ComboBox::from_id_salt("inspector_placement")
                            .selected_text(match settings.inspector_placement {
                                InspectorPlacement::Global => "Global right panel",
                                InspectorPlacement::ActiveTab => "Inside active tab",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut settings.inspector_placement,
                                    InspectorPlacement::Global,
                                    "Global right panel",
                                );
                                ui.selectable_value(
                                    &mut settings.inspector_placement,
                                    InspectorPlacement::ActiveTab,
                                    "Inside active tab",
                                );
                            });
                    });
                });

            egui::CollapsingHeader::new("Appearance")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Theme is set per-profile in the sidebar.")
                            .weak()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Default view for new tabs:");
                        egui::ComboBox::from_id_salt("view_mode")
                            .selected_text(match settings.view_mode {
                                ViewMode::Details => "Details",
                                ViewMode::Grid => "Grid",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut settings.view_mode,
                                    ViewMode::Details,
                                    "Details",
                                );
                                ui.selectable_value(
                                    &mut settings.view_mode,
                                    ViewMode::Grid,
                                    "Grid",
                                );
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sidebar width:");
                        ui.add(egui::Slider::new(
                            &mut settings.sidebar_width,
                            160.0..=400.0,
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Inspector width:");
                        ui.add(egui::Slider::new(
                            &mut settings.inspector_width,
                            200.0..=500.0,
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Font size:");
                        ui.add(egui::Slider::new(&mut settings.font_size, 10.0..=24.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Default thumbnail size:");
                        ui.add(egui::Slider::new(
                            &mut settings.thumbnail_size,
                            80.0..=256.0,
                        ));
                    });
                });

            egui::CollapsingHeader::new("Keyboard Shortcuts")
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new("shortcuts")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            let shortcuts = [
                                ("New Tab", "Ctrl+T"),
                                ("Close Tab", "Ctrl+W"),
                                ("Goto Bar", "Ctrl+P"),
                                ("Edit Location", "Ctrl+L"),
                                ("Command Palette", "Ctrl+Shift+P"),
                                ("Toggle Drop Shelf", "Ctrl+Shift+D"),
                                ("Toggle Inspector", "Ctrl+I"),
                                ("Toggle Inspector", "Space"),
                                ("Toggle Sidebar", "Ctrl+B"),
                                ("Toggle Hidden", "Ctrl+."),
                                ("Select All", "Ctrl+A"),
                                ("Copy", "Ctrl+C"),
                                ("Cut", "Ctrl+X"),
                                ("Paste", "Ctrl+V"),
                                ("Settings", "Ctrl+,"),
                                ("Navigate Back", "Alt+Left"),
                                ("Navigate Forward", "Alt+Right"),
                                ("Navigate Up", "Alt+Up"),
                                ("Undo", "Ctrl+Z"),
                                ("Redo", "Ctrl+Shift+Z"),
                                ("Close Overlays", "Escape"),
                            ];
                            for (action, key) in shortcuts {
                                ui.label(action);
                                ui.label(egui::RichText::new(key).monospace());
                                ui.end_row();
                            }
                        });
                });

            if !settings.recent_folders.is_empty() {
                egui::CollapsingHeader::new("Recent Folders")
                    .default_open(false)
                    .show(ui, |ui| {
                        if ui.button("Clear Recent").clicked() {
                            settings.recent_folders.clear();
                        }
                        for p in settings.recent_folders.iter().rev().take(20) {
                            ui.label(p.to_string_lossy().as_ref());
                        }
                    });
            }
        });
}
