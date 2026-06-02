use egui::{Id, Rect, Ui, UiBuilder, Vec2};

pub fn cubic_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn cubic_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0_f32).powi(3) / 2.0
    }
}

pub fn sine_in_out(t: f32) -> f32 {
    -(std::f32::consts::PI * t).cos() * 0.5 + 0.5
}

/// Animate a boolean with easing. Returns value in [0.0, 1.0].
pub fn animate_bool_eased(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    value: bool,
    duration: f32,
    easing: fn(f32) -> f32,
) -> f32 {
    ctx.animate_bool_with_time_and_easing(Id::new(id), value, duration, easing)
}

/// Vertical collapse/expand with animated height.
pub fn collapse_vertical(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    visible: bool,
    content: impl FnOnce(&mut Ui),
) {
    let id = Id::new(id);
    let t = animate_bool_eased(
        ui.ctx(),
        id.with("t"),
        visible,
        ui.style().animation_time,
        cubic_in_out,
    );

    let last_h: f32 = ui.ctx().memory_mut(|m| *m.data.get_temp_mut_or(id, 0.0));
    let current_h = last_h * t;

    let available = ui.available_size();
    let origin = ui.next_widget_position();

    let clip_rect = ui.clip_rect().intersect(Rect::from_min_size(
        origin,
        Vec2::new(available.x, current_h),
    ));

    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(origin, available))
            .layout(*ui.layout()),
    );
    child.set_clip_rect(clip_rect);
    content(&mut child);

    let size = child.min_size().min(available);
    ui.ctx().memory_mut(|m| m.data.insert_temp(id, size.y));

    ui.allocate_rect(
        Rect::from_min_size(origin, Vec2::new(size.x, current_h)),
        egui::Sense::hover(),
    );
}

/// Smooth [0→1→0] pulse — useful for indeterminate progress indicators.
pub fn animate_pulse(ui: &mut Ui, period_secs: f32) -> f32 {
    // Indeterminate progress only needs ~30 fps; an unconditional `request_repaint`
    // would drive the whole app at full frame rate for the duration of every op.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
    let t = ui.input(|i| i.time as f32);
    let phase = (t % period_secs) / period_secs;
    let rt = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    sine_in_out(rt)
}
