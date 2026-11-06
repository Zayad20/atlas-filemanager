//! Direct sidebar ink. Native wrappers own gestures, focus, tooltips and menus;
//! one clipped leaf shapes and paints each row instead of an icon/text subtree.
use crate::ui::{
    icons::AtlasIcon,
    surface::{paint_rounded, paint_theme_icon, shape_text_truncated, Slot},
};
use gpui::{canvas, point, prelude::*, px, Corners, Hsla, IntoElement, RenderImage, TextAlign};
use std::sync::Arc;

pub(crate) enum RowIcon {
    Theme(AtlasIcon, Hsla),
    Texture(Arc<RenderImage>),
    Swatch(Hsla),
    None,
}

pub(crate) struct RowInk {
    label: String,
    leading: RowIcon,
    color: Hsla,
    windows: bool,
    detail: Option<(String, Hsla)>,
    subtitle: bool,
    progress: Option<(f32, Hsla)>,
    centered: bool,
}

impl RowInk {
    pub fn new(label: impl Into<String>, leading: RowIcon, color: Hsla, windows: bool) -> Self {
        Self {
            label: label.into(),
            leading,
            color,
            windows,
            detail: None,
            subtitle: false,
            progress: None,
            centered: false,
        }
    }
    pub fn detail(mut self, label: String, color: Hsla, subtitle: bool) -> Self {
        self.detail = (!label.is_empty()).then_some((label, color));
        self.subtitle = subtitle;
        self
    }
    pub fn progress(mut self, fraction: f32, color: Hsla) -> Self {
        self.progress = Some((fraction.clamp(0.0, 1.0), color));
        self
    }
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
    pub fn into_element(self) -> impl IntoElement {
        let windows = self.windows;
        let color = self.color;
        let leading = self.leading;
        let has_leading = !matches!(leading, RowIcon::None);
        let centered = self.centered;
        let subtitle = self.subtitle;
        let progress = self.progress;
        canvas(
            move |bounds, window, _cx| {
                crate::perf::count("sidebar_ink.prepaint");
                let _span = crate::perf::span("sidebar_ink.prepaint");
                let visible = bounds.intersect(&window.content_mask().bounds);
                if visible.size.width <= px(0.0) || visible.size.height <= px(0.0) {
                    return None;
                }
                let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
                let font = window.text_style().font();
                let font_size = window.text_style().font_size.to_pixels(window.rem_size());
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let icon_size = (if windows { 18.0 } else { 16.0 }) * zoom;
                let prefix = if !has_leading {
                    0.0
                } else {
                    icon_size + (if windows { 10.0 } else { 8.0 }) * zoom
                };
                let detail = self.detail.map(|(label, color)| {
                    shape_text_truncated(
                        &label,
                        &font,
                        px(12.0 * zoom),
                        color,
                        if subtitle {
                            (width - prefix).max(0.0)
                        } else {
                            (width * 0.48).max(0.0)
                        },
                        window,
                    )
                });
                let detail_width = detail.as_ref().filter(|_| !subtitle).map_or(0.0, |line| {
                    crate::ui::surface::line_width(line) + 8.0 * zoom
                });
                let label = shape_text_truncated(
                    &self.label,
                    &font,
                    font_size,
                    color,
                    (width - prefix - detail_width).max(0.0),
                    window,
                );
                Some((
                    label, detail, width, height, icon_size, prefix, zoom, font_size,
                ))
            },
            move |bounds, ink, window, cx| {
                let Some((label, detail, width, height, icon_size, prefix, zoom, font_size)) = ink
                else {
                    return;
                };
                crate::perf::count("sidebar_ink.paint");
                let _span = crate::perf::span("sidebar_ink.paint");
                if width <= 0.0 || height <= 0.0 {
                    return;
                }
                window.paint_layer(bounds, |window| {
                    let content_h =
                        (height - if progress.is_some() { 7.0 * zoom } else { 0.0 }).max(0.0);
                    let icon_x = if centered {
                        (width - icon_size) * 0.5
                    } else {
                        0.0
                    };
                    let icon_y = (content_h - icon_size) * 0.5;
                    let icon_slot = Slot::new(icon_x, icon_y, icon_size, icon_size);
                    match &leading {
                        RowIcon::Theme(icon, color) => paint_theme_icon(
                            window,
                            cx,
                            bounds.origin,
                            icon_slot,
                            *icon,
                            windows,
                            icon_size,
                            *color,
                        ),
                        RowIcon::Texture(texture) => {
                            let icon_bounds = crate::ui::thumbs::contain_bounds(
                                texture,
                                icon_slot.to_bounds(bounds.origin),
                            );
                            let _ = window.paint_image(
                                icon_bounds,
                                icon_bounds,
                                Corners::all(px(0.0)),
                                texture.clone(),
                                0,
                                false,
                            );
                        }
                        RowIcon::Swatch(color) => {
                            let diameter = 11.0 * zoom;
                            paint_rounded(
                                window,
                                Slot::new(
                                    icon_x + (icon_size - diameter) * 0.5,
                                    icon_y + (icon_size - diameter) * 0.5,
                                    diameter,
                                    diameter,
                                )
                                .to_bounds(bounds.origin),
                                diameter * 0.5,
                                *color,
                            );
                        }
                        RowIcon::None => {}
                    }
                    if !centered {
                        let text_h = f32::from(font_size) * 1.25;
                        let y = if subtitle && detail.is_some() {
                            (content_h - text_h - 15.0 * zoom) * 0.5
                        } else {
                            (content_h - text_h) * 0.5
                        };
                        let _ = label.paint(
                            point(bounds.left() + px(prefix), bounds.top() + px(y)),
                            px(text_h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        if let Some(detail) = detail {
                            let detail_w = crate::ui::surface::line_width(&detail);
                            let x = if subtitle {
                                prefix
                            } else {
                                (width - detail_w).max(prefix)
                            };
                            let y = if subtitle {
                                y + text_h
                            } else {
                                (content_h - 15.0 * zoom) * 0.5
                            };
                            let _ = detail.paint(
                                point(bounds.left() + px(x), bounds.top() + px(y)),
                                px(15.0 * zoom),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    if let Some((fraction, color)) = progress {
                        let bar = Slot::new(0.0, (height - 4.0 * zoom).max(0.0), width, 4.0 * zoom);
                        paint_rounded(
                            window,
                            bar.to_bounds(bounds.origin),
                            2.0 * zoom,
                            crate::ui::glass::track_fill(),
                        );
                        paint_rounded(
                            window,
                            Slot::new(bar.x, bar.y, width * fraction, bar.h)
                                .to_bounds(bounds.origin),
                            2.0 * zoom,
                            color,
                        );
                    }
                });
            },
        )
        .flex_1()
        .min_w(px(0.0))
        .h_full()
    }
}

pub(crate) fn icon_or_texture(
    icon: AtlasIcon,
    texture: Option<Arc<RenderImage>>,
    color: Hsla,
) -> RowIcon {
    texture.map_or(RowIcon::Theme(icon, color), RowIcon::Texture)
}
