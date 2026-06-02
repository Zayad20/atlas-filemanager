use crate::state::drag::{DragHoverState, DragHoverTarget};

/// Paint an immediate "you can drop here" highlight (used for file→tag drops, which
/// assign on release rather than spring-loading).
pub fn paint_drop_highlight(response: &egui::Response) {
    response.ctx.debug_painter().rect_stroke(
        response.rect.expand(2.0),
        4.0,
        egui::Stroke::new(2.0, response.ctx.style().visuals.selection.stroke.color),
        egui::StrokeKind::Inside,
    );
}

/// Paint the "spring-loaded navigation" confirmation blink on a drag target.
pub fn paint_drag_hover_confirmation(
    response: &egui::Response,
    drag_hover: &DragHoverState,
    target: &DragHoverTarget,
) {
    let now = response.ctx.input(|input| input.time);
    if drag_hover.confirmation_visible(target, now) {
        response.ctx.debug_painter().rect_stroke(
            response.rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, response.ctx.style().visuals.selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}
