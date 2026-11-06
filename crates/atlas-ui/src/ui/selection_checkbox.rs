//! Shared selected-item checkbox for file views.
//!
//! Grid uses the element form as an overlay; canvas-backed views use `paint` so
//! they retain their virtualized paint path. Both forms share the same palette.

use gpui::prelude::FluentBuilder;
use gpui::ColorExt as _;
use gpui::{
    div, point, px, size, AnimationExt, Bounds, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Styled, Window,
};
use gpui_component::ActiveTheme;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

pub const ANIM_SECS: f32 = 0.18;
/// Checkbox size (logical px) in row views; it replaces the row's file icon.
pub const ROW_SIZE: f32 = 15.0;
pub const SPRING: crate::ui::motion::Spring = crate::ui::motion::Spring {
    omega: 34.0,
    zeta: 0.56,
};

/// Visible checkbox transitions, keyed by file identity instead of recycled indices.
/// Ordinary repaints and hover never restart the selection pop.
pub(crate) struct SelectionFeedback<K> {
    visible: HashMap<K, Option<Instant>>,
}

impl<K: Eq + Hash + Clone> Default for SelectionFeedback<K> {
    fn default() -> Self {
        Self {
            visible: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone> SelectionFeedback<K> {
    pub(crate) fn sample(&mut self, items: &[(K, bool)], now: Instant) -> Vec<Option<Instant>> {
        let mut next = HashMap::with_capacity(items.len());
        let timestamps = items
            .iter()
            .map(|(key, shown)| {
                let at = shown.then(|| self.visible.get(key).copied().flatten().unwrap_or(now));
                next.insert(key.clone(), at);
                at
            })
            .collect();
        self.visible = next;
        timestamps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn checkbox_feedback_follows_identity_without_restarting_on_repaint_or_sort() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let mut feedback = SelectionFeedback::default();
        assert_eq!(
            feedback.sample(&[("a", false), ("b", false)], now),
            [None, None]
        );
        assert_eq!(
            feedback.sample(&[("a", true), ("b", true)], later),
            [Some(later), Some(later)]
        );
        assert_eq!(
            feedback.sample(&[("b", true), ("a", true)], later + Duration::from_secs(1)),
            [Some(later), Some(later)]
        );
        feedback.sample(&[("a", false), ("b", true)], later);
        assert_eq!(
            feedback.sample(&[("a", true), ("b", true)], now),
            [Some(now), Some(later)]
        );
    }

    #[test]
    fn checkbox_feedback_prunes_files_that_leave_the_viewport() {
        let now = Instant::now();
        let mut feedback = SelectionFeedback::default();
        feedback.sample(&[("a", true), ("b", true)], now);
        feedback.sample(&[("b", true)], now);
        assert_eq!(feedback.visible.len(), 1);
        assert!(!feedback.visible.contains_key("a"));
        feedback.sample(&[], now);
        assert!(feedback.visible.is_empty());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckState {
    Empty,
    Mixed,
    Checked,
}

/// DG control form, including the indeterminate state used by table headers.
pub(crate) fn paint_control(
    window: &mut Window,
    cx: &mut gpui::App,
    center: gpui::Point<gpui::Pixels>,
    size_px: f32,
    state: CheckState,
    disabled: bool,
) {
    let theme = cx.theme();
    let filled = state != CheckState::Empty;
    let border = if filled {
        theme.accent
    } else {
        theme
            .muted_foreground
            .opacity(if disabled { 0.3 } else { 0.65 })
    };
    let mark = theme.accent_foreground;
    let fill = if filled {
        theme.accent
    } else {
        gpui::transparent_black()
    };
    let bounds = Bounds {
        origin: point(center.x - px(size_px * 0.5), center.y - px(size_px * 0.5)),
        size: size(px(size_px), px(size_px)),
    };
    window.paint_quad(gpui::quad(
        bounds,
        gpui::Corners::all(px(size_px * 0.30)),
        fill,
        gpui::Edges::all(px(size_px * (1.5 / 14.0))),
        border,
        Default::default(),
    ));
    if state == CheckState::Mixed {
        window.paint_quad(gpui::fill(
            Bounds {
                origin: point(center.x - px(size_px * 0.24), center.y - px(0.75)),
                size: size(px(size_px * 0.48), px(1.5)),
            },
            mark,
        ));
    } else if state == CheckState::Checked {
        let mark_size = size_px * 0.8;
        let _ = window.paint_svg(
            Bounds {
                origin: point(
                    center.x - px(mark_size * 0.5),
                    center.y - px(mark_size * 0.5),
                ),
                size: size(px(mark_size), px(mark_size)),
            },
            "icons/check.svg".into(),
            None,
            Default::default(),
            mark,
            cx,
        );
    }
}

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
    let elapsed = selected_at.map(|at| at.elapsed().as_secs_f32());
    let active = !cx.reduce_motion() && elapsed.is_some_and(|t| t < SPRING.settle_secs(0.02));
    let (progress, scale) = if active {
        let elapsed = elapsed.unwrap();
        (
            (elapsed / ANIM_SECS).clamp(0.0, 1.0),
            SPRING.value(elapsed).max(0.0),
        )
    } else {
        (1.0, 1.0)
    };
    if active {
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
    mark.alpha *= progress;
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
                    .map(|this| {
                        if cx.reduce_motion() {
                            this.into_any_element()
                        } else {
                            this.with_spring(
                                SharedString::from("selection-checkbox-pop"),
                                gpui::SpringAnimation::new(SPRING.config())
                                    .to(1.0_f32)
                                    .from(0.0),
                                move |this, scale| {
                                    this.opacity(scale.clamp(0.0, 1.0))
                                        .size(px(size_px * scale.max(0.0)))
                                },
                            )
                            .into_any_element()
                        }
                    }),
            )
    }
}
