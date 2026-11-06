//! Recycle Bin state is queried off-thread; the icon acknowledges real changes.
use crate::{
    app::App,
    ui::{
        dg::motion::ActionPulse,
        icons::AtlasIcon,
        surface::{paint_theme_icon, Slot},
    },
};
use gpui::{
    canvas, prelude::*, px, Corners, Entity, Hsla, IntoElement, RenderImage, RenderOnce, Window,
};
use std::{sync::Arc, time::Instant};

#[derive(IntoElement)]
pub(crate) struct RecycleIcon {
    pub app: Entity<App>,
    pub texture: Option<Arc<RenderImage>>,
    pub color: Hsla,
    pub windows: bool,
    pub size: f32,
}

impl RenderOnce for RecycleIcon {
    fn render(self, _window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let full = self.app.read(cx).recycle_bin_full;
        let color = self.color;
        let native = self.texture;
        let windows = self.windows;
        canvas(
            move |_, window, cx| {
                let now = Instant::now();
                let state = window.use_keyed_state("recycle-bin-state-feedback", cx, |_, _| {
                    (full, ActionPulse::<bool>::default())
                });
                let reduced = cx.reduce_motion();
                state.update(cx, |state, _| {
                    if state.0 != full {
                        if let (Some(_), Some(full)) = (state.0, full) {
                            state.1.trigger(full, now);
                        }
                        state.0 = full;
                    }
                    state.1.sample(&full.unwrap_or(false), now, reduced)
                })
            },
            move |bounds, (pulse, active), window, cx| {
                let width = f32::from(bounds.size.width);
                if let Some(texture) = &native {
                    let bounds = crate::ui::thumbs::contain_bounds(texture, bounds);
                    let _ = window.paint_image(
                        bounds,
                        bounds,
                        Corners::all(px(0.0)),
                        texture.clone(),
                        0,
                        false,
                    );
                } else {
                    paint_theme_icon(
                        window,
                        cx,
                        bounds.origin,
                        Slot::new(0.0, 0.0, width, f32::from(bounds.size.height)),
                        AtlasIcon::Trash,
                        windows,
                        width,
                        color,
                    );
                    if full == Some(true) {
                        let dot = gpui::Bounds {
                            origin: gpui::point(
                                bounds.right() - px(4.0),
                                bounds.bottom() - px(4.0),
                            ),
                            size: gpui::size(px(4.0), px(4.0)),
                        };
                        crate::ui::surface::paint_rounded(window, dot, 2.0, color);
                    }
                }
                if pulse > 0.0 {
                    let mut tint = color;
                    tint.alpha *= pulse * 0.65;
                    window.paint_quad(
                        gpui::outline(bounds, tint, Default::default())
                            .corner_radii(Corners::all(px(4.0))),
                    );
                }
                if active {
                    window.request_animation_frame();
                }
            },
        )
        .size(crate::ui::rem_px(self.size))
        .flex_none()
    }
}
