//! Measured, clipped disclosure. Child geometry remains natural during resize.
use crate::ui::dg::motion::RetainedSpring;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Hsla, IntoElement, RenderOnce, SharedString,
    Window,
};
use gpui_component::ElementExt;
use std::time::Instant;

struct RevealState {
    natural: Option<f32>,
    height: RetainedSpring,
}

#[derive(IntoElement)]
pub(crate) struct DisclosureBody {
    id: SharedString,
    open: bool,
    content: AnyElement,
    animate_entry: bool,
}

impl DisclosureBody {
    pub fn new(id: impl Into<SharedString>, open: bool, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            open,
            content: content.into_any_element(),
            animate_entry: false,
        }
    }

    pub fn animate_entry(mut self) -> Self {
        self.animate_entry = true;
        self
    }
}

impl RenderOnce for DisclosureBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| RevealState {
            natural: None,
            height: RetainedSpring::new(0.0, Instant::now()),
        });
        let instant = cx.reduce_motion();
        let (height, active) = state.update(cx, |s, _| {
            s.height.sample(
                if self.open {
                    s.natural.unwrap_or(0.0)
                } else {
                    0.0
                },
                instant,
                Instant::now(),
            )
        });
        let open = self.open;
        let animate_entry = self.animate_entry && !instant;
        div()
            .id(self.id)
            .w_full()
            .flex()
            .flex_col()
            .flex_none()
            .h(px(height.max(0.0)))
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .child(self.content)
                    .on_prepaint_child(move |bounds, window, cx| {
                        let natural = f32::from(bounds.size.height);
                        let changed = state.update(cx, |s, _| {
                            if s.natural.is_some_and(|h| (h - natural).abs() < 0.25) {
                                return false;
                            }
                            if s.natural.is_none() {
                                s.height = RetainedSpring::new(
                                    if open && !animate_entry { natural } else { 0.0 },
                                    Instant::now(),
                                );
                            }
                            s.natural = Some(natural);
                            true
                        });
                        if changed {
                            window.refresh();
                        }
                    }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        if active {
                            window.request_animation_frame();
                        }
                    },
                )
                .size(px(0.0)),
            )
    }
}

/// Explicit-height surfaces keep their own hit bounds while changing extent.
#[derive(IntoElement)]
pub(crate) struct AnimatedHeight {
    pub id: SharedString,
    pub height: f32,
    pub instant: bool,
    pub content: gpui::Stateful<gpui::Div>,
}

impl RenderOnce for AnimatedHeight {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = Instant::now();
        let target = self.height * f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let state = window.use_keyed_state(self.id, cx, |_, _| RetainedSpring::new(target, now));
        let instant = self.instant || cx.reduce_motion();
        let (height, active) = state.update(cx, |spring, _| spring.sample(target, instant, now));
        self.content.h(px(height.max(0.0))).child(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    if active {
                        window.request_animation_frame();
                    }
                },
            )
            .size(px(0.0)),
        )
    }
}

pub(crate) fn chevron(id: SharedString, open: bool, color: Hsla) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let now = Instant::now();
            let state = window.use_keyed_state(id.clone(), cx, |_, _| {
                RetainedSpring::new(if open { 1.0 } else { 0.0 }, now)
            });
            let instant = cx.reduce_motion();
            let (value, active) = state.update(cx, |s, _| {
                s.sample_unit(if open { 1.0 } else { 0.0 }, instant, now)
            });
            (bounds, value, active)
        },
        move |_, (bounds, value, active), window, cx| {
            let center = bounds.center();
            let scale = window.scale_factor();
            let transform = gpui::TransformationMatrix::unit()
                .translate(center.scale(scale))
                .rotate(gpui::radians(value * std::f32::consts::FRAC_PI_2))
                .translate(center.scale(-scale));
            let _ = window.paint_svg(
                bounds,
                crate::ui::icons::AtlasIcon::CaretRight.path().into(),
                None,
                transform,
                color,
                cx,
            );
            if active {
                window.request_animation_frame();
            }
        },
    )
    .size(crate::ui::rem_px(16.0))
    .flex_none()
}
