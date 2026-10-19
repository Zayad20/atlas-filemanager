//! The same polar geometry owns paint and hit testing; no invisible box targets.
use super::super::converter::{Phase, Session};
use crate::app::App;
use gpui::prelude::*;
use gpui::{
    canvas, point, px, Bounds, Context, DispatchPhase, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, PathBuilder, Pixels, Point, Window,
};
use std::f32::consts::TAU;

pub(super) const PAGE_SIZE: usize = 4;

pub(super) use super::geometry::angle;
use super::geometry::hit;

fn polar(center: Point<Pixels>, radius: f32, angle: f32) -> Point<Pixels> {
    point(
        center.x + px(radius * angle.cos()),
        center.y + px(radius * angle.sin()),
    )
}

fn sector(
    center: Point<Pixels>,
    radius: f32,
    index: usize,
    count: usize,
) -> Option<gpui::Path<Pixels>> {
    let step = TAU / count.max(1) as f32;
    let start = angle(index, count) - step * 0.5 + 0.035;
    let end = start + step - 0.07;
    let outer = radius - 6.0;
    let inner = radius * 0.40;
    let corner = ((outer - inner) * 0.20).min(inner * step * 0.15);
    let oa = start + corner / outer;
    let ob = end - corner / outer;
    let ia = start + corner / inner;
    let ib = end - corner / inner;
    // Exact circular arcs and four rounded radial corners: a constant-size
    // command buffer, with no per-frame polygon allocations.
    let mut path = PathBuilder::fill();
    let curve = |path: &mut PathBuilder, a: Point<Pixels>, p: Point<Pixels>, b: Point<Pixels>| {
        let c1 = point(
            a.x + (p.x - a.x) * (2.0 / 3.0),
            a.y + (p.y - a.y) * (2.0 / 3.0),
        );
        let c2 = point(
            b.x + (p.x - b.x) * (2.0 / 3.0),
            b.y + (p.y - b.y) * (2.0 / 3.0),
        );
        path.cubic_bezier_to(b, c1, c2);
    };
    path.move_to(polar(center, outer, oa));
    path.arc_to(
        point(px(outer), px(outer)),
        px(0.),
        ob - oa > std::f32::consts::PI,
        true,
        polar(center, outer, ob),
    );
    curve(
        &mut path,
        polar(center, outer, ob),
        polar(center, outer, end),
        polar(center, outer - corner, end),
    );
    path.line_to(polar(center, inner + corner, end));
    curve(
        &mut path,
        polar(center, inner + corner, end),
        polar(center, inner, end),
        polar(center, inner, ib),
    );
    path.arc_to(
        point(px(inner), px(inner)),
        px(0.),
        ib - ia > std::f32::consts::PI,
        false,
        polar(center, inner, ia),
    );
    curve(
        &mut path,
        polar(center, inner, ia),
        polar(center, inner, start),
        polar(center, inner + corner, start),
    );
    path.line_to(polar(center, outer - corner, start));
    curve(
        &mut path,
        polar(center, outer - corner, start),
        polar(center, outer, start),
        polar(center, outer, oa),
    );
    path.close();
    path.build().ok()
}

pub(super) fn paint_wheel(
    diameter: f32,
    selected: usize,
    available: Vec<bool>,
    colors: (Hsla, Hsla, Hsla),
    cx: &mut Context<App>,
) -> gpui::AnyElement {
    let count = available.len();
    let view = cx.entity();
    let page = selected / PAGE_SIZE;
    #[cfg(feature = "conversion-lab")]
    let geometry_view = view.clone();
    canvas(
        move |bounds, _, cx| {
            #[cfg(feature = "conversion-lab")]
            geometry_view.update(cx, |app, _| {
                if let Some(session) = app.converter.as_mut() {
                    session.wheel_bounds = Some(bounds);
                }
            });
            let _ = (bounds, cx);
        },
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _cx| {
            let radius = f32::from(bounds.size.width) * 0.5;
            let center = bounds.center();
            for (index, enabled) in available.iter().enumerate() {
                if let Some(path) = sector(center, radius, index, count) {
                    let color = if index + page * PAGE_SIZE == selected {
                        colors.1
                    } else if *enabled {
                        colors.0
                    } else {
                        colors.2
                    };
                    window.paint_path(path, color);
                }
            }
            let hover_view = view.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, _window, cx| {
                if phase != DispatchPhase::Bubble || !bounds.contains(&ev.position) {
                    return;
                }
                let Some(index) = hit(
                    f32::from(ev.position.x - center.x),
                    f32::from(ev.position.y - center.y),
                    radius,
                    count,
                ) else {
                    return;
                };
                hover_view.update(cx, |app, cx| {
                    if let Some(session) = app
                        .converter
                        .as_mut()
                        .filter(|s| matches!(s.phase, Phase::Choosing))
                    {
                        let selected = page * PAGE_SIZE + index;
                        if session.selected != selected {
                            session.selected = selected;
                            cx.notify();
                        }
                    }
                });
            });
            let click_view = view.clone();
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, _window, cx| {
                if phase != DispatchPhase::Bubble
                    || ev.button != MouseButton::Left
                    || !bounds.contains(&ev.position)
                {
                    return;
                }
                let Some(index) = hit(
                    f32::from(ev.position.x - center.x),
                    f32::from(ev.position.y - center.y),
                    radius,
                    count,
                ) else {
                    return;
                };
                click_view.update(cx, |app, cx| {
                    if let Some(Session {
                        phase: Phase::Choosing,
                        selected,
                        ..
                    }) = app.converter.as_mut()
                    {
                        *selected = page * PAGE_SIZE + index;
                        app.start_conversion();
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            });
        },
    )
    .size(px(diameter))
    .into_any_element()
}
