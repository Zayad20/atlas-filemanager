//! Flat TabsBar surface: math layout + measured labels + hit slots (1 layout node).

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::AtlasIcon;
use crate::ui::surface::{
    chrome_font_size_tabs, hit_first, line_width, paint_fill, paint_rounded, paint_rounded_corners,
    paint_theme_icon, shape_text, text_origin_y, Slot,
};
use atlas_model::pane::TabId;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, Corners, DispatchPhase, Element,
    ElementId, Font, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, Pixels, Refineable, ShapedLine, SharedString, Style, StyleRefinement, Styled,
    TextAlign, WeakEntity, Window,
};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct TabBarItem {
    pub pane_id: TabId,
    pub label: String,
    pub is_active: bool,
    pub tag_rgb: Option<[u8; 3]>,
}

#[derive(Clone, Copy, Debug)]
enum TabHit {
    Activate(usize),
    Close(usize),
    NewTab,
}

pub(crate) struct TabsBarElement {
    id: ElementId,
    app: WeakEntity<App>,
    tabs: Arc<Vec<TabBarItem>>,
    active: Option<TabId>,
    font: Font,
    font_size: Pixels,
    windows_theme: bool,
    bar_bg: Hsla,
    active_bg: Hsla,
    hover_bg: Hsla,
    active_fg: Hsla,
    tab_fg: Hsla,
    muted: Hsla,
    border: Hsla,
    accent: Hsla,
    style: StyleRefinement,
}

struct TabGeom {
    tab: Slot,
    label: Slot,
    close: Slot,
    tag: Option<Slot>,
}

pub(crate) struct TabsBarPrepaint {
    labels: Vec<ShapedLine>,
    geoms: Vec<TabGeom>,
    new_btn: Slot,
    strip_h: f32,
    radius: f32,
}

impl TabsBarElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app: WeakEntity<App>,
        tabs: Vec<TabBarItem>,
        active: Option<TabId>,
        font: Font,
        font_size: Pixels,
        windows_theme: bool,
        bar_bg: Hsla,
        active_bg: Hsla,
        hover_bg: Hsla,
        active_fg: Hsla,
        tab_fg: Hsla,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from("tabs-bar-surface")),
            app,
            tabs: Arc::new(tabs),
            active,
            font,
            font_size,
            windows_theme,
            bar_bg,
            active_bg,
            hover_bg,
            active_fg,
            tab_fg,
            muted,
            border,
            accent,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for TabsBarElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for TabsBarElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for TabsBarElement {
    type RequestLayoutState = Style;
    type PrepaintState = TabsBarPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> (LayoutId, Self::RequestLayoutState) {
        crate::perf::count("tabs.bar.layout");
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let strip_h = if self.windows_theme { 46.0 } else { 34.0 } * zoom;
        let mut style = Style::default();
        style.refine(&self.style);
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Absolute(
            gpui::AbsoluteLength::Pixels(px(strip_h)),
        ));
        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        _cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        let _perf = crate::perf::span("tabs.bar.prepaint");
        crate::perf::count("tabs.bar.prepaint");

        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        // Force chrome type scale (ignore body font_size from caller).
        self.font_size = chrome_font_size_tabs(window, self.windows_theme);
        let strip_h = if self.windows_theme { 46.0 } else { 34.0 } * zoom;
        let tab_h = if self.windows_theme { 38.0 } else { 26.0 } * zoom;
        let tab_min = if self.windows_theme { 118.0 } else { 56.0 } * zoom;
        let tab_max = if self.windows_theme { 210.0 } else { 220.0 } * zoom;
        let gap = if self.windows_theme { 8.0 } else { 6.0 } * zoom;
        let close_sz = if self.windows_theme { 22.0 } else { 18.0 } * zoom;
        let pad_x = 8.0 * zoom;
        let pad_in = 10.0 * zoom;
        let radius = if self.windows_theme { 8.0 } else { 6.0 } * zoom;
        let new_sz = if self.windows_theme { 32.0 } else { 24.0 } * zoom;
        let tag_dot = 9.0 * zoom;

        // Shape labels with final colors (active vs idle).
        let mut labels = Vec::with_capacity(self.tabs.len());
        let mut natural_ws = Vec::with_capacity(self.tabs.len());
        for tab in self.tabs.iter() {
            let color = if tab.is_active {
                self.active_fg
            } else {
                self.tab_fg
            };
            let line = shape_text(&tab.label, &self.font, self.font_size, color, window);
            let text_w = line_width(&line);
            let tag_w = if tab.tag_rgb.is_some() {
                tag_dot + 6.0 * zoom
            } else {
                0.0
            };
            // pad + tag + text + gap + close + pad
            let natural =
                (pad_in + tag_w + text_w + 6.0 * zoom + close_sz + pad_in).clamp(tab_min, tab_max);
            natural_ws.push(natural);
            labels.push(line);
        }

        let avail = (f32::from(bounds.size.width) - pad_x * 2.0 - new_sz - gap).max(40.0);
        let sum: f32 = natural_ws.iter().sum();
        let scale = if sum > avail && sum > 0.0 {
            avail / sum
        } else {
            1.0
        };

        let y0 = (strip_h - tab_h) * 0.5;
        let mut x = pad_x;
        let mut geoms = Vec::with_capacity(self.tabs.len());
        for (i, tab) in self.tabs.iter().enumerate() {
            let tw = (natural_ws[i] * scale).max(tab_min * 0.55);
            let tab_slot = Slot::new(x, y0, tw, tab_h);
            let mut inner_x = x + pad_in * scale.max(0.5);
            let tag = if tab.tag_rgb.is_some() {
                let s = Slot::new(inner_x, y0 + (tab_h - tag_dot) * 0.5, tag_dot, tag_dot);
                inner_x += tag_dot + 6.0 * zoom;
                Some(s)
            } else {
                None
            };
            let close = Slot::new(
                x + tw - pad_in * scale.max(0.5) - close_sz,
                y0 + (tab_h - close_sz) * 0.5,
                close_sz,
                close_sz,
            );
            let label_w = (close.x - 4.0 * zoom - inner_x).max(0.0);
            let label = Slot::new(inner_x, y0, label_w, tab_h);
            geoms.push(TabGeom {
                tab: tab_slot,
                label,
                close,
                tag,
            });
            x += tw + gap;
            let _ = i;
        }

        let new_btn = Slot::new(x, y0 + (tab_h - new_sz) * 0.5, new_sz, new_sz);

        TabsBarPrepaint {
            labels,
            geoms,
            new_btn,
            strip_h,
            radius,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        let _perf = crate::perf::span("tabs.bar.paint");
        crate::perf::count("tabs.bar.paint");

        let app = self.app.clone();
        let tabs = self.tabs.clone();
        let active_bg = self.active_bg;
        let hover_bg = self.hover_bg;
        let muted = self.muted;
        let bar_bg = self.bar_bg;
        let border = self.border;
        let accent = self.accent;
        let radius = prepaint.radius;
        let strip_h = prepaint.strip_h;
        let origin = bounds.origin;

        // Hover from live mouse (no extra state).
        let mouse = window.mouse_position();
        let mlx = f32::from(mouse.x - origin.x);
        let mly = f32::from(mouse.y - origin.y);
        let hover_i = prepaint.geoms.iter().position(|g| g.tab.contains(mlx, mly));

        let windows_theme = self.windows_theme;
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        // Match old theme_icon sizes (px @ 16rem, then zoom).
        let close_icon_px = if windows_theme { 11.0 } else { 10.0 } * zoom;
        let plus_icon_px = if windows_theme { 15.0 } else { 13.0 } * zoom;

        style.paint(bounds, window, cx, |window, cx| {
            // Tab bar: rounded top corners like old strip (12px).
            let bar_r = 12.0 * zoom;
            paint_rounded_corners(
                window,
                bounds,
                Corners {
                    top_left: px(bar_r),
                    top_right: px(bar_r),
                    bottom_left: px(0.),
                    bottom_right: px(0.),
                },
                bar_bg,
            );
            paint_fill(
                window,
                Bounds {
                    origin: point(origin.x, origin.y + px(strip_h - 1.0)),
                    size: size(bounds.size.width, px(1.0)),
                },
                border.opacity(0.42),
            );

            for (i, tab) in tabs.iter().enumerate() {
                let g = &prepaint.geoms[i];
                let tab_b = g.tab.to_bounds(origin);
                if tab.is_active {
                    paint_rounded(window, tab_b, radius, active_bg);
                } else if hover_i == Some(i) {
                    paint_rounded(window, tab_b, radius, hover_bg);
                }

                if let (Some(tag_slot), Some(rgb)) = (g.tag, tab.tag_rgb) {
                    let c = gpui::rgb(
                        (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]),
                    );
                    paint_rounded(
                        window,
                        tag_slot.to_bounds(origin),
                        tag_slot.h * 0.5,
                        c.into(),
                    );
                }

                if let Some(line) = prepaint.labels.get(i) {
                    let lb = g.label.to_bounds(origin);
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: lb.intersect(&bounds),
                        }),
                        |window| {
                            // origin = top of label slot; line_height = slot h → GPUI centers glyphs.
                            let ty = origin.y + px(text_origin_y(g.label.y, zoom));
                            let _ = line.paint(
                                point(lb.origin.x, ty),
                                px(g.label.h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        },
                    );
                }

                if g.close.contains(mlx, mly) {
                    paint_rounded(
                        window,
                        g.close.to_bounds(origin),
                        radius * 0.6,
                        border.opacity(0.45),
                    );
                }
                paint_theme_icon(
                    window,
                    cx,
                    origin,
                    g.close,
                    AtlasIcon::X,
                    windows_theme,
                    close_icon_px,
                    muted,
                );

                // Active underline: inside the tab pill, bottom edge, not the strip border.
                if tab.is_active {
                    let u = Slot::new(
                        g.tab.x + radius.max(6.0),
                        g.tab.y + g.tab.h - 2.5 * zoom,
                        (g.tab.w - 2.0 * radius.max(6.0)).max(8.0),
                        2.0 * zoom,
                    );
                    paint_fill(window, u.to_bounds(origin), accent);
                }
            }

            if prepaint.new_btn.contains(mlx, mly) {
                paint_rounded(window, prepaint.new_btn.to_bounds(origin), radius, hover_bg);
            }
            paint_theme_icon(
                window,
                cx,
                origin,
                prepaint.new_btn,
                AtlasIcon::Plus,
                windows_theme,
                plus_icon_px,
                muted,
            );

            // Hit table for input
            let mut hits: Vec<(Slot, TabHit)> = Vec::with_capacity(tabs.len() * 2 + 1);
            for (i, g) in prepaint.geoms.iter().enumerate() {
                hits.push((g.close, TabHit::Close(i)));
                hits.push((g.tab, TabHit::Activate(i)));
            }
            hits.push((prepaint.new_btn, TabHit::NewTab));

            let Some(app_entity) = app.upgrade() else {
                return;
            };
            let tabs_hit = tabs.clone();
            let active_for_new = self.active;
            let hits = Arc::new(hits);
            let hit_view = app_entity;
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, _window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                    return;
                }
                if !bounds.contains(&ev.position) {
                    return;
                }
                let lx = f32::from(ev.position.x - bounds.origin.x);
                let ly = f32::from(ev.position.y - bounds.origin.y);
                let Some(hit) = hit_first(&hits, lx, ly) else {
                    return;
                };
                match hit {
                    TabHit::Close(i) => {
                        if let Some(tab) = tabs_hit.get(i) {
                            hit_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseTab(tab.pane_id)]);
                                cx.notify();
                            });
                        }
                    }
                    TabHit::Activate(i) => {
                        if let Some(tab) = tabs_hit.get(i) {
                            hit_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::FocusTab(tab.pane_id)]);
                                cx.notify();
                            });
                        }
                    }
                    TabHit::NewTab => {
                        if let Some(anchor) = active_for_new {
                            hit_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::NewTab(anchor)]);
                                cx.notify();
                            });
                        }
                    }
                }
                cx.stop_propagation();
            });
        });
    }
}
