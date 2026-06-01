use std::path::PathBuf;

use crate::input::action::Action;

const DROP_SHELF_VIEWPORT: &str = "drop_shelf";

pub fn show_drop_shelf(
    ctx: &egui::Context,
    open: &mut bool,
    files: &mut Vec<PathBuf>,
    pending_native_drag: &mut Option<Vec<PathBuf>>,
    actions: &mut Vec<Action>,
) {
    if !*open {
        return;
    }

    let builder = egui::ViewportBuilder::default()
        .with_title("Atlas Drop Shelf")
        .with_inner_size([360.0, 320.0])
        .with_min_inner_size([260.0, 180.0])
        .with_always_on_top()
        .with_drag_and_drop(true);

    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of(DROP_SHELF_VIEWPORT),
        builder,
        |ctx, _class| {
            if ctx.input(|input| input.viewport().close_requested()) {
                *open = false;
            }

            let dropped: Vec<PathBuf> = ctx.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.clone())
                    .collect()
            });
            add_unique_paths(files, dropped);

            let hovered_count = ctx.input(|input| input.raw.hovered_files.len());
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Drop Shelf");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!files.is_empty(), egui::Button::new("Clear"))
                            .clicked()
                        {
                            files.clear();
                        }
                    });
                });
                ui.label("Drop files here to keep them handy.");
                ui.separator();

                if hovered_count > 0 {
                    let label = if hovered_count == 1 {
                        "Release to add 1 item".to_string()
                    } else {
                        format!("Release to add {hovered_count} items")
                    };
                    ui.add_sized(
                        [ui.available_width(), 42.0],
                        egui::Label::new(egui::RichText::new(label).strong()),
                    );
                    ui.separator();
                }

                if files.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("The shelf is empty");
                    });
                    return;
                }

                let mut remove_idx = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (idx, path) in files.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy())
                                .unwrap_or_else(|| path.to_string_lossy());
                            let response = ui
                                .add(
                                    egui::Label::new(name.as_ref())
                                        .sense(egui::Sense::click_and_drag()),
                                )
                                .on_hover_text(path.display().to_string());
                            if response.drag_started() {
                                *pending_native_drag = Some(vec![path.clone()]);
                            }
                            if response.double_clicked() {
                                actions.push(Action::Open(vec![path.clone()]));
                            }
                            if ui.small_button("Open").clicked() {
                                actions.push(Action::Open(vec![path.clone()]));
                            }
                            if ui.small_button("Remove").clicked() {
                                remove_idx = Some(idx);
                            }
                        });
                    }
                });
                if let Some(idx) = remove_idx {
                    files.remove(idx);
                }
            });
        },
    );
}

fn add_unique_paths(files: &mut Vec<PathBuf>, dropped: impl IntoIterator<Item = PathBuf>) {
    for path in dropped {
        if !files.contains(&path) {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::add_unique_paths;
    use std::path::PathBuf;

    #[test]
    fn dropped_paths_keep_order_without_duplicates() {
        let mut files = vec![PathBuf::from("/tmp/first")];

        add_unique_paths(
            &mut files,
            [
                PathBuf::from("/tmp/second"),
                PathBuf::from("/tmp/first"),
                PathBuf::from("/tmp/third"),
            ],
        );

        assert_eq!(
            files,
            [
                PathBuf::from("/tmp/first"),
                PathBuf::from("/tmp/second"),
                PathBuf::from("/tmp/third"),
            ]
        );
    }
}
