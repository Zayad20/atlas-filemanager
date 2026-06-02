use crate::input::action::Action;
use crate::state::ops::{PasteConflictState, Resolution};

/// Modal shown when a paste collides with existing names at the destination.
/// Offers a single decision applied to all conflicts: Overwrite / Skip / Rename,
/// or Cancel. Pushes `Action::ResolvePasteConflict`.
pub fn show_paste_conflict(
    ctx: &egui::Context,
    conflict: &PasteConflictState,
    actions: &mut Vec<Action>,
) {
    let verb = if conflict.is_cut { "Move" } else { "Copy" };
    let count = conflict.conflicts.len();

    let mut choice: Option<Option<Resolution>> = None;

    egui::Modal::new(egui::Id::new("paste_conflict_modal")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading(format!("{count} item(s) already exist"));
        ui.add_space(4.0);
        ui.label(format!(
            "The destination already contains item(s) with the same name. How should the {} proceed?",
            verb.to_lowercase()
        ));
        ui.add_space(8.0);

        // List the conflicting names (capped to keep the dialog compact).
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for path in conflict.conflicts.iter().take(50) {
                    ui.label(format!(
                        "• {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                if count > 50 {
                    ui.label(format!("…and {} more", count - 50));
                }
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui
                .button("Rename")
                .on_hover_text("Keep both — paste under a new name like \"file (1)\"")
                .clicked()
            {
                choice = Some(Some(Resolution::Rename));
            }
            if ui
                .button("Overwrite")
                .on_hover_text("Replace the existing item(s)")
                .clicked()
            {
                choice = Some(Some(Resolution::Overwrite));
            }
            if ui
                .button("Skip")
                .on_hover_text("Keep the existing item(s); don't paste the conflicting one(s)")
                .clicked()
            {
                choice = Some(Some(Resolution::Skip));
            }
            ui.add_space(8.0);
            if ui.button("Cancel").clicked() {
                choice = Some(None);
            }
        });
    });

    // Esc cancels.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        choice = Some(None);
    }

    if let Some(resolution) = choice {
        actions.push(Action::ResolvePasteConflict(resolution));
    }
}
