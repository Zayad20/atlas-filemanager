//! Shared selected-item checkbox for file views.
//!
//! Grid uses the element form as an overlay; canvas-backed views use `paint` so
//! they retain their virtualized paint path. Both forms share the same palette.

use gpui::{
    div, point, px, size, AnimationExt, Bounds, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Styled, Window,
};
use gpui_component::ActiveTheme;

pub const ANIM_SECS: f32 = 0.18;
pub const SPRING: crate::ui::motion::Spring = crate::ui::motion::Spring {
    omega: 34.0,
    zeta: 0.56,
};

#[derive(Clone, Copy)]
pub(crate) struct SelectionCheckboxColors {
    pub surface: Hsla,
    pub mark: Hsla,
    pub border: Hsla,
    pub shadow: Hsla,
}

impl SelectionCheckboxColors {
    fn from_theme(cx: &gpui::App) -> Self {
        let theme = cx.theme();
        Self {
            // The check floats over selected artwork rather than cutting a solid
            // white hole through it. These values remain legible on dark, light,
            // glass, and high-saturation accent themes.
            surface: theme.background.opacity(0.56),
            mark: theme.accent_foreground,
            border: theme.accent_foreground.opacity(0.72),
            shadow: theme.background,
        }
    }
}

/// Paint form for canvas/custom Elements. `progress` is 0..1 and `scale` may
/// overshoot slightly for the selection pop.
pub(crate) fn paint_animated(
    window: &mut Window,
    cx: &mut gpui::App,
    center: gpui::Point<gpui::Pixels>,
    size_px: f32,
    selected_at: Option<std::time::Instant>,
) {
    let elapsed = selected_at
        .map(|at| at.elapsed().as_secs_f32())
        .unwrap_or(ANIM_SECS);
    let progress = (elapsed / ANIM_SECS).clamp(0.0, 1.0);
    let scale = SPRING.value(elapsed).max(0.0);
    if progress < 1.0 {
        window.request_animation_frame();
    }
    let colors = SelectionCheckboxColors::from_theme(cx);
    let box_size = size_px * scale;
    let radius = px((box_size * 0.26).max(3.0));
    let bounds = Bounds {
        origin: point(center.x - px(box_size * 0.5), center.y - px(box_size * 0.5)),
        size: size(px(box_size), px(box_size)),
    };
    // The small shadow separates the control from both bright thumbnails and
    // saturated selection fills. The border gets its color from the active theme.
    window.paint_quad(
        gpui::fill(
            Bounds {
                origin: point(bounds.origin.x + px(1.0), bounds.origin.y + px(1.0)),
                size: bounds.size,
            },
            colors.shadow.opacity(0.28 * progress),
        )
        .corner_radii(radius),
    );
    window.paint_quad(gpui::fill(bounds, colors.surface).corner_radii(radius));
    window
        .paint_quad(gpui::outline(bounds, colors.border, Default::default()).corner_radii(radius));

    let check_size = size_px * (0.64 + 0.28 * crate::ui::motion::ease_out_quart(progress));
    let mut mark = colors.mark;
    mark.a *= progress;
    let _ = window.paint_svg(
        Bounds {
            origin: point(
                center.x - px(check_size * 0.5),
                center.y - px(check_size * 0.5),
            ),
            size: size(px(check_size), px(check_size)),
        },
        SharedString::from("icons/check.svg"),
        None,
        Default::default(),
        mark,
        cx,
    );
}

/// Element form for layout-based views such as Grid. It is positioned by the
/// caller and animates only when the selected overlay is mounted.
#[derive(IntoElement)]
pub(crate) struct SelectionCheckbox {
    id: SharedString,
    size_px: f32,
}

impl SelectionCheckbox {
    pub(crate) fn new(id: impl Into<SharedString>, size_px: f32) -> Self {
        Self {
            id: id.into(),
            size_px,
        }
    }
}

impl RenderOnce for SelectionCheckbox {
    fn render(self, _window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let colors = SelectionCheckboxColors::from_theme(cx);
        let visual_id = SharedString::from(format!("{}-visual", self.id));
        let size_px = self.size_px;
        div()
            .id(self.id)
            .size(px(size_px))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id(visual_id)
                    .size(px(size_px))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px((size_px * 0.26).max(3.0)))
                    .bg(colors.surface)
                    .border_1()
                    .border_color(colors.border)
                    .shadow_sm()
                    .text_color(colors.mark)
                    .child(
                        gpui::svg()
                            .path("icons/check.svg")
                            .size(px(size_px * 0.62))
                            .text_color(colors.mark),
                    )
                    .with_animation(
                        SharedString::from("selection-checkbox-pop"),
                        crate::ui::motion::linear_anim(std::time::Duration::from_secs_f32(
                            ANIM_SECS,
                        )),
                        move |this, t| {
                            let scale = SPRING.value(t * ANIM_SECS).max(0.0);
                            this.opacity(t).size(px(size_px * scale))
                        },
                    ),
            )
    }
}
