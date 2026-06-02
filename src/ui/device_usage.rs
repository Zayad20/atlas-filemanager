use egui::{Color32, RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints};

use crate::state::app_state::AppState;

pub fn show_device_usage(ui: &mut Ui, state: &mut AppState) {
    state.device_usage.update();

    let n = state.device_usage.cpu_history.len();
    let cpu_points: PlotPoints = state
        .device_usage
        .cpu_history
        .iter()
        .enumerate()
        .map(|(i, &v)| [i as f64, v as f64])
        .collect();

    let ram_points: PlotPoints = state
        .device_usage
        .ram_history
        .iter()
        .enumerate()
        .map(|(i, &v)| [i as f64, v as f64])
        .collect();

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Activity").strong().size(13.0).color(ui.visuals().text_color().gamma_multiply(0.7)));
        });
        ui.add_space(4.0);

        let is_dark = ui.visuals().dark_mode;
        
        // Use vibrant colors that look good in both light and dark mode
        let cpu_color = if is_dark { Color32::from_rgb(180, 255, 180) } else { Color32::from_rgb(40, 160, 40) };
        let ram_color = if is_dark { Color32::from_rgb(180, 200, 255) } else { Color32::from_rgb(40, 100, 200) };

        let cpu_line = Line::new("CPU", cpu_points)
            .color(cpu_color)
            .width(2.0)
            .fill(0.15); // Area chart effect

        let ram_line = Line::new("RAM", ram_points)
            .color(ram_color)
            .width(2.0)
            .fill(0.15); // Area chart effect

        let height = 80.0;
        
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(0, 4))
            .show(ui, |ui| {
                Plot::new("device_usage_plot")
                    .height(height)
                    .show_x(false)
                    .show_y(false)
                    .show_axes([false, false]) // completely hide axes for a modern look
                    .show_grid([false, false]) // hide grid 
                    .show_background(false) // transparent background
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .include_y(0.0)
                    .include_y(100.0)
                    .include_x(0.0)
                    .include_x(f64::max(0.0, (n as f64) - 1.0))
                    .show(ui, |plot_ui| {
                        plot_ui.line(ram_line);
                        plot_ui.line(cpu_line);
                    });
            });
            
        // Show current values briefly
        if let (Some(cpu), Some(ram)) = (state.device_usage.cpu_history.back(), state.device_usage.ram_history.back()) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("CPU: {cpu:.1}%")).size(10.0).color(cpu_color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("RAM: {ram:.1}%")).size(10.0).color(ram_color));
                });
            });
        }
        
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    });
}
