use super::ColorPickerState;
use gpui::{
    App, Bounds, ColorExt, DispatchPhase, Entity, Hsla, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Styled, Window, canvas, point, px,
};
use std::time::Instant;

#[path = "angular_math.rs"]
mod math;
use math::{Hsv, HueTransition};

#[derive(Clone, Copy)]
enum Part {
    Hue,
    SaturationValue,
}

pub(super) struct AngularState {
    pub(super) animate: bool,
    hsv: Hsv,
    hue: HueTransition,
    last_color: Option<Hsla>,
    bounds: Bounds<Pixels>,
    drag: Option<Part>,
}

impl Default for AngularState {
    fn default() -> Self {
        Self {
            animate: true,
            hsv: Hsv::from_hsl([0.0, 0.0, 0.0, 1.0]),
            hue: HueTransition::default(),
            last_color: None,
            bounds: Bounds::default(),
            drag: None,
        }
    }
}

impl AngularState {
    fn sync(&mut self, color: Hsla) {
        if self.last_color == Some(color) {
            return;
        }
        let mut hsv = Hsv::from_hsl([
            color.hue.into_positive_degrees() / 360.0,
            color.saturation,
            color.lightness,
            color.alpha,
        ]);
        if hsv.saturation < 0.0001 || hsv.value < 0.0001 {
            hsv.hue = self.hsv.hue;
        }
        self.hue.retarget(hsv.hue, Instant::now());
        self.hsv = hsv;
        self.last_color = Some(color);
    }

    fn apply(&mut self, part: Part, position: Point<Pixels>) -> Hsla {
        let geometry = Geometry::new(self.bounds);
        match part {
            Part::Hue => {
                self.hsv.hue = (f32::from(position.y - geometry.center.y))
                    .atan2(f32::from(position.x - geometry.center.x))
                    .rem_euclid(std::f32::consts::TAU)
                    / std::f32::consts::TAU;
                self.hue.retarget(self.hsv.hue, Instant::now());
            }
            Part::SaturationValue => {
                self.hsv.saturation = (f32::from(position.x - geometry.square.left())
                    / f32::from(geometry.square.size.width))
                .clamp(0.0, 1.0);
                self.hsv.value = (1.0
                    - f32::from(position.y - geometry.square.top())
                        / f32::from(geometry.square.size.height))
                .clamp(0.0, 1.0);
            }
        }
        let [h, s, l, a] = self.hsv.to_hsl();
        let color = gpui::hsla(h, s, l, a);
        self.last_color = Some(color);
        color
    }
}

struct Geometry {
    center: Point<Pixels>,
    outer: f32,
    inner: f32,
    square: Bounds<Pixels>,
}

impl Geometry {
    fn new(bounds: Bounds<Pixels>) -> Self {
        let center = bounds.center();
        let outer = f32::from(bounds.size.width.min(bounds.size.height)) * 0.48;
        let inner = outer * 0.78;
        let side = inner * std::f32::consts::SQRT_2 * 0.88;
        let square = Bounds::new(
            point(center.x - px(side * 0.5), center.y - px(side * 0.5)),
            gpui::size(px(side), px(side)),
        );
        Self {
            center,
            outer,
            inner,
            square,
        }
    }
    fn polar(&self, angle: f32, radius: f32) -> Point<Pixels> {
        point(
            self.center.x + px(angle.cos() * radius),
            self.center.y + px(angle.sin() * radius),
        )
    }
    fn part(&self, position: Point<Pixels>) -> Option<Part> {
        if self.square.contains(&position) {
            return Some(Part::SaturationValue);
        }
        let x = f32::from(position.x - self.center.x);
        let y = f32::from(position.y - self.center.y);
        let radius = x.hypot(y);
        (radius >= self.inner && radius <= self.outer).then_some(Part::Hue)
    }
}

fn marker(window: &mut Window, position: Point<Pixels>, radius: f32) {
    for (r, color) in [
        (radius + 1.0, gpui::black()),
        (radius, gpui::white()),
        (radius - 2.0, gpui::black()),
    ] {
        let bounds = Bounds::new(
            point(position.x - px(r), position.y - px(r)),
            gpui::size(px(r * 2.0), px(r * 2.0)),
        );
        window.paint_quad(gpui::quad(
            bounds,
            gpui::Corners::all(px(r)),
            gpui::transparent_black(),
            gpui::Edges::all(px(1.0)),
            color,
            Default::default(),
        ));
    }
}

pub(super) fn picker(state: Entity<ColorPickerState>) -> impl IntoElement {
    let layout_state = state.clone();
    canvas(
        move |bounds, window, cx| {
            layout_state.update(cx, |state, _| {
                state.angular.bounds = bounds;
                state.angular.sync(
                    state
                        .hovered_color
                        .or(state.value)
                        .unwrap_or_else(gpui::black),
                );
            });
            window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
        },
        move |bounds, hitbox, window, cx| {
            let geometry = Geometry::new(bounds);
            let (hsv, (hue, animating)) = {
                let state = state.read(cx);
                (state.angular.hsv, if state.angular.animate { state.angular.hue.sample(Instant::now()) } else { (state.angular.hsv.hue, false) })
            };
            for segment in 0..180 {
                let a = segment as f32 / 180.0 * std::f32::consts::TAU;
                let b = (segment + 1) as f32 / 180.0 * std::f32::consts::TAU + 0.001;
                let mut path = gpui::PathBuilder::fill();
                path.move_to(geometry.polar(a, geometry.inner));
                path.line_to(geometry.polar(a, geometry.outer));
                path.line_to(geometry.polar(b, geometry.outer));
                path.line_to(geometry.polar(b, geometry.inner));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, gpui::hsla(segment as f32 / 180.0, 1.0, 0.5, 1.0));
                }
            }
            // Opaque strips keep Value in sRGB rather than darkening an sRGB
            // gradient by alpha blending in linear light. Each physical row is
            // sampled from the same HSV math used by pointer selection.
            let rows = (f32::from(geometry.square.size.height) * window.scale_factor()).ceil().max(2.0) as usize;
            let row_h = geometry.square.size.height / rows as f32;
            for row in 0..rows {
                let value = 1.0 - (row as f32 + 0.5) / rows as f32;
                let color = |saturation| {
                    let [h, s, l, a] = Hsv { hue: hsv.hue, saturation, value, alpha: 1.0 }.to_hsl();
                    gpui::hsla(h, s, l, a)
                };
                let strip = Bounds::new(point(geometry.square.left(), geometry.square.top() + row_h * row as f32),
                    gpui::size(geometry.square.size.width, row_h + px(0.1 / window.scale_factor())));
                window.paint_quad(gpui::fill(strip, gpui::linear_gradient(90.0,
                    gpui::linear_color_stop(color(0.0), 0.0), gpui::linear_color_stop(color(1.0), 1.0))
                    .color_space(gpui::ColorSpace::Srgb)));
            }
            let sv_marker = point(
                geometry.square.left() + geometry.square.size.width * hsv.saturation,
                geometry.square.top() + geometry.square.size.height * (1.0 - hsv.value),
            );
            marker(
                window,
                geometry.polar(
                    hue * std::f32::consts::TAU,
                    (geometry.inner + geometry.outer) * 0.5,
                ),
                5.0,
            );
            marker(window, sv_marker, 5.0);
            if animating {
                window.request_animation_frame();
            }

            let down_state = state.downgrade();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let Some(part) = geometry.part(event.position) else {
                    return;
                };
                _ = down_state.update(cx, |state, cx| {
                    state.angular.drag = Some(part);
                    let color = state.angular.apply(part, event.position);
                    state.update_value(Some(color), true, window, cx);
                    cx.stop_propagation();
                });
            });
            let move_state = state.downgrade();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                _ = move_state.update(cx, |state, cx| {
                    let Some(part) = state.angular.drag else {
                        return;
                    };
                    if event.pressed_button != Some(MouseButton::Left) {
                        state.angular.drag = None;
                        return;
                    }
                    let color = state.angular.apply(part, event.position);
                    state.update_value(Some(color), true, window, cx);
                    cx.stop_propagation();
                });
            });
            let up_state = state.downgrade();
            window.on_mouse_event(move |event: &MouseUpEvent, _, _, cx| {
                if event.button == MouseButton::Left {
                    _ = up_state.update(cx, |state, _| {
                        state.angular.drag = None;
                    });
                }
            });
        },
    )
    .w_full()
    .h(gpui::rems(14.0))
}
