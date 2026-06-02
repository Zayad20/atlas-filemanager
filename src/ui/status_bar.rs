use crate::state::app_state::AppState;
use crate::util::display::format_size;

pub fn show_status_bar(ctx: &egui::Context, state: &AppState) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(22.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(ts) = state.active_tab_state() {
                    let n_total = ts.entries.len();
                    let n_sel = ts.selection.selected.len();
                    if n_sel > 0 {
                        let total_size: u64 = ts
                            .selection.selected
                            .iter()
                            .filter_map(|&i| ts.entries.get(i).and_then(|e| e.size))
                            .sum();
                        ui.label(format!(
                            "{} of {} selected — {}",
                            n_sel,
                            n_total,
                            format_size(total_size)
                        ));
                    } else {
                        ui.label(format!("{n_total} items"));
                    }
                }

                // Progress for active ops
                for op in state.ops.active_ops.values() {
                    let pct = if op.total > 0 {
                        op.done as f32 / op.total as f32
                    } else {
                        crate::ui::anim::animate_pulse(ui, 1.4)
                    };
                    ui.add(egui::ProgressBar::new(pct).desired_width(120.0));
                    if op.total > 0 {
                        ui.label(
                            op.current_file
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        );
                    }
                }
            });
        });
}
