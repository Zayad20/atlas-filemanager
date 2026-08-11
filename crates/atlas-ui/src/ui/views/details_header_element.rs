//! Single-node Details column header (replaces ~30–40 nested divs per pane).
//!
//! Paints labels / sort glyphs / resize grips; handles sort click, column reorder,
//! and resize press. Live resize completion still goes through `interactions.rs`.

use crate::app::{App, ColResizeState, DetailsColumn};
use crate::input::action::Action;
use crate::ui::icons::AtlasIcon;
use crate::ui::surface::{chrome_font_size_header, paint_theme_icon, shape_text, Slot};
use crate::ui::views::details::{
    DetailsColors, DetailsContentLod, DetailsRowLayout, CELL_PAD, DETAILS_ROW_H, ROW_TEXT_Y_NUDGE,
};
use atlas_model::file_entry::{SortColumn, SortDir};
use atlas_model::pane::TabId;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, DispatchPhase, Element, ElementId, Font,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Refineable, ShapedLine,
    SharedString, Style, StyleRefinement, Styled, TextAlign, WeakEntity, Window,
};
use std::cell::RefCell;
use std::rc::Rc;

const RESIZE_HALF: f32 = 5.0;

#[derive(Clone, Copy)]
enum HeaderHit {
    ResizeName,
    ResizeMeta(DetailsColumn),
    SortName,
    SortMeta(DetailsColumn),
}

struct HeaderPress {
    hit: HeaderHit,
    start_screen_x: f32,
    moved: bool,
}

pub(crate) struct DetailsHeaderElement {
    id: ElementId,
    pane_id: TabId,
    app: WeakEntity<App>,
    layout: DetailsRowLayout,
    colors: DetailsColors,
    font: Font,
    font_size: Pixels,
    sort_col: SortColumn,
    sort_dir: SortDir,
    has_type_filter: bool,
    /// Snapshot for right-click type filter menu (avoid mut borrow in paint).
    type_labels: Vec<String>,
    dragging: Option<DetailsColumn>,
    drag_x: f32,
    content_lod: DetailsContentLod,
    style: StyleRefinement,
}

pub(crate) struct DetailsHeaderPrepaint {
    name_line: Option<ShapedLine>,
    meta_lines: Vec<(DetailsColumn, ShapedLine)>,
    layout: DetailsRowLayout,
    header_h: f32,
    hitbox: Hitbox,
}

impl DetailsHeaderElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: TabId,
        app: WeakEntity<App>,
        layout: DetailsRowLayout,
        colors: DetailsColors,
        font: Font,
        font_size: Pixels,
        sort_col: SortColumn,
        sort_dir: SortDir,
        has_type_filter: bool,
        type_labels: Vec<String>,
        dragging: Option<DetailsColumn>,
        drag_x: f32,
        content_lod: DetailsContentLod,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!(
                "details-header-el-{}",
                pane_id.0
            ))),
            pane_id,
            app,
            layout,
            colors,
            font,
            font_size,
            sort_col,
            sort_dir,
            has_type_filter,
            type_labels,
            dragging,
            drag_x,
            content_lod,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for DetailsHeaderElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for DetailsHeaderElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn shape_label(
    text: &str,
    font: &Font,
    font_size: Pixels,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    shape_text(text, font, font_size, color, window)
}

fn column_label(column: DetailsColumn) -> String {
    match column {
        DetailsColumn::Size => rust_i18n::t!("column_size").into_owned(),
        DetailsColumn::Modified => rust_i18n::t!("column_modified").into_owned(),
        DetailsColumn::Type => rust_i18n::t!("column_type").into_owned(),
    }
}

fn sort_for_column(column: DetailsColumn) -> SortColumn {
    match column {
        DetailsColumn::Size => SortColumn::Size,
        DetailsColumn::Modified => SortColumn::Modified,
        DetailsColumn::Type => SortColumn::Extension,
    }
}

fn hit_test(layout: &DetailsRowLayout, local_x: f32, width: f32) -> Option<HeaderHit> {
    if local_x < 0.0 || local_x > width {
        return None;
    }
    if (local_x - layout.name_right).abs() <= RESIZE_HALF {
        return Some(HeaderHit::ResizeName);
    }
    for col in &layout.columns {
        let edge = col.x + col.w;
        if (local_x - edge).abs() <= RESIZE_HALF {
            return Some(HeaderHit::ResizeMeta(col.column));
        }
    }
    if local_x >= layout.name_x && local_x < layout.name_right {
        return Some(HeaderHit::SortName);
    }
    for col in &layout.columns {
        if local_x >= col.x && local_x < col.x + col.w {
            return Some(HeaderHit::SortMeta(col.column));
        }
    }
    None
}

impl Element for DetailsHeaderElement {
    type RequestLayoutState = Style;
    type PrepaintState = DetailsHeaderPrepaint;

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
        crate::perf::count("details.header.layout");
        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let header_h = DETAILS_ROW_H * zoom_scale;
        let mut style = Style::default();
        style.refine(&self.style);
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Absolute(
            gpui::AbsoluteLength::Pixels(px(header_h)),
        ));
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
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
        let _perf = crate::perf::span("details.header.prepaint");
        crate::perf::count("details.header.prepaint");

        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let header_h = DETAILS_ROW_H * zoom_scale;
        // Header labels use text_xs chrome scale.
        self.font_size = chrome_font_size_header(window);
        let font = &self.font;
        let font_size = self.font_size;
        // Header input is registered on the window from paint; this makes it
        // respect popup/modal occlusion instead of only checking raw bounds.
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let name_active = self.sort_col == SortColumn::Name;
        let name_color = if name_active {
            self.colors.accent
        } else {
            self.colors.muted
        };
        let name_line = self.content_lod.paints_name_header().then(|| {
            shape_label(
                &rust_i18n::t!("column_name"),
                font,
                font_size,
                name_color,
                window,
            )
        });

        let mut meta_lines = Vec::with_capacity(self.layout.columns.len());
        if self.content_lod.paints_metadata() {
            for col in &self.layout.columns {
                let sort = sort_for_column(col.column);
                let active = self.sort_col == sort
                    || (col.column == DetailsColumn::Type && self.has_type_filter);
                let color = if active {
                    self.colors.accent
                } else {
                    self.colors.muted
                };
                let line = shape_label(&column_label(col.column), font, font_size, color, window);
                meta_lines.push((col.column, line));
            }
        }

        DetailsHeaderPrepaint {
            name_line,
            meta_lines,
            layout: self.layout.clone(),
            header_h,
            hitbox,
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
        let _perf = crate::perf::span("details.header.paint");
        crate::perf::count("details.header.paint");

        let pane_id = self.pane_id;
        let app = self.app.clone();
        let sort_col = self.sort_col.clone();
        let sort_dir = self.sort_dir.clone();
        let has_type_filter = self.has_type_filter;
        let type_labels = self.type_labels.clone();
        let dragging = self.dragging;
        let drag_x = self.drag_x;
        let accent = self.colors.accent;
        let header_bg = self.colors.header_bg;
        let border = self.colors.border;
        let layout_snap = prepaint.layout.clone();
        let header_h = prepaint.header_h;
        let press: Rc<RefCell<Option<HeaderPress>>> = Rc::new(RefCell::new(None));

        style.paint(bounds, window, cx, |window, cx| {
            let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
            let cell_pad = CELL_PAD * zoom_scale;
            let text_y = bounds.origin.y + px(ROW_TEXT_Y_NUDGE * zoom_scale);
            let width = f32::from(bounds.size.width);

            window.paint_quad(gpui::fill(bounds, header_bg));
            window.paint_quad(gpui::fill(
                Bounds {
                    origin: point(bounds.origin.x, bounds.origin.y + px(header_h - 1.0)),
                    size: size(bounds.size.width, px(1.0)),
                },
                border.opacity(0.45),
            ));

            let paint_sort_glyph = |window: &mut Window,
                                    cx: &mut GpuiApp,
                                    active: bool,
                                    dir: &SortDir,
                                    glyph_right: f32,
                                    mid_y: f32| {
                if !active {
                    return;
                }
                let icon_sz = 9.0 * zoom_scale;
                let kind = if *dir == SortDir::Asc {
                    AtlasIcon::ArrowUp
                } else {
                    AtlasIcon::ArrowDown
                };
                let local_x = glyph_right - icon_sz - f32::from(bounds.origin.x);
                let local_y = mid_y - icon_sz * 0.5 - f32::from(bounds.origin.y);
                // Tiny sort arrows: Phosphor SVG stays sharper at 9px than Fluent.
                paint_theme_icon(
                    window,
                    cx,
                    bounds.origin,
                    Slot::new(local_x, local_y, icon_sz, icon_sz),
                    kind,
                    false,
                    icon_sz,
                    accent,
                );
            };

            // Name
            {
                let name_x =
                    bounds.origin.x + px(layout_snap.name_x + cell_pad.min(layout_snap.name_col_w));
                let name_clip = Bounds {
                    origin: point(bounds.origin.x + px(layout_snap.name_x), bounds.origin.y),
                    size: size(px(layout_snap.name_col_w.max(0.0)), px(header_h)),
                };
                if self.content_lod.paints_name_header()
                    && layout_snap.name_col_w > 15.0 * zoom_scale
                {
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: name_clip.intersect(&bounds),
                        }),
                        |window| {
                            if let Some(line) = &prepaint.name_line {
                                let _ = line.paint(
                                    point(name_x, text_y),
                                    px(header_h),
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        },
                    );
                }
                if self.content_lod.paints_name_header() {
                    paint_sort_glyph(
                        window,
                        cx,
                        sort_col == SortColumn::Name,
                        &sort_dir,
                        f32::from(bounds.origin.x) + layout_snap.name_right - 4.0 * zoom_scale,
                        f32::from(bounds.origin.y) + header_h * 0.5,
                    );
                }
            }

            for col in &layout_snap.columns {
                if dragging == Some(col.column) {
                    continue;
                }
                let sort = sort_for_column(col.column);
                let active = sort_col == sort;
                let Some((_, line)) = prepaint.meta_lines.iter().find(|(c, _)| *c == col.column)
                else {
                    continue;
                };
                let col_x = bounds.origin.x + px(col.x);
                let text_x = col_x + px(cell_pad.min(col.w));
                let clip = Bounds {
                    origin: point(col_x, bounds.origin.y),
                    size: size(px(col.w.max(0.0)), px(header_h)),
                };
                if col.w > 15.0 * zoom_scale {
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: clip.intersect(&bounds),
                        }),
                        |window| {
                            let _ = line.paint(
                                point(text_x, text_y),
                                px(header_h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        },
                    );
                }
                if self.content_lod.paints_metadata()
                    && col.column == DetailsColumn::Type
                    && has_type_filter
                {
                    let dot = 5.0 * zoom_scale;
                    window.paint_quad(gpui::fill(
                        Bounds {
                            origin: point(
                                col_x + px(col.w - 14.0 * zoom_scale),
                                bounds.origin.y + px((header_h - dot) * 0.5),
                            ),
                            size: size(px(dot), px(dot)),
                        },
                        accent,
                    ));
                }
                if self.content_lod.paints_metadata() {
                    paint_sort_glyph(
                        window,
                        cx,
                        active,
                        &sort_dir,
                        f32::from(col_x) + col.w - 4.0 * zoom_scale,
                        f32::from(bounds.origin.y) + header_h * 0.5,
                    );
                }
            }

            let paint_grip = |window: &mut Window, edge_x: f32| {
                window.paint_quad(gpui::fill(
                    Bounds {
                        origin: point(
                            bounds.origin.x + px(edge_x),
                            bounds.origin.y + px(4.0 * zoom_scale),
                        ),
                        size: size(px(1.0), px((header_h - 8.0 * zoom_scale).max(1.0))),
                    },
                    border.opacity(0.55),
                ));
            };
            paint_grip(window, layout_snap.name_right);
            for col in &layout_snap.columns {
                paint_grip(window, col.x + col.w);
            }

            if let Some(col) =
                dragging.and_then(|d| layout_snap.columns.iter().find(|c| c.column == d).copied())
            {
                let w = col.w;
                let left = (drag_x - w * 0.5).max(0.0);
                let ghost = Bounds {
                    origin: point(bounds.origin.x + px(left), bounds.origin.y),
                    size: size(px(w), px(header_h)),
                };
                window.paint_quad(gpui::fill(ghost, header_bg));
                window.paint_quad(gpui::outline(ghost, accent, Default::default()));
                if let Some((_, line)) = prepaint.meta_lines.iter().find(|(c, _)| *c == col.column)
                {
                    let _ = line.paint(
                        point(ghost.origin.x + px(cell_pad), text_y),
                        px(header_h),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }

            let Some(app_entity) = app.upgrade() else {
                return;
            };

            // Left down: resize or begin click/drag
            let press_dn = press.clone();
            let layout_dn = layout_snap.clone();
            let down_view = app_entity.clone();
            let down_hitbox = prepaint.hitbox.clone();
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !down_hitbox.is_hovered(window) {
                    return;
                }
                let pos = ev.position;
                if !bounds.contains(&pos) {
                    return;
                }
                let local_x = f32::from(pos.x - bounds.origin.x);

                if ev.button == MouseButton::Right {
                    if matches!(
                        hit_test(&layout_dn, local_x, width),
                        Some(HeaderHit::SortMeta(DetailsColumn::Type))
                    ) {
                        // Opened in a separate handler that has `window`.
                    }
                    cx.stop_propagation();
                    return;
                }
                if ev.button != MouseButton::Left {
                    return;
                }
                let Some(hit) = hit_test(&layout_dn, local_x, width) else {
                    cx.stop_propagation();
                    return;
                };
                match hit {
                    HeaderHit::ResizeName => {
                        let start_width = layout_dn.name_right;
                        down_view.update(cx, move |this, _| {
                            this.col_resize = Some(ColResizeState {
                                col: "name",
                                pane: pane_id,
                                start_x: f32::from(pos.x),
                                start_width,
                            });
                        });
                    }
                    HeaderHit::ResizeMeta(column) => {
                        let start_width = layout_dn
                            .columns
                            .iter()
                            .find(|col| col.column == column)
                            .map(|col| col.w)
                            .unwrap_or(0.0);
                        down_view.update(cx, move |this, _| {
                            this.col_resize = Some(ColResizeState {
                                col: column.key(),
                                pane: pane_id,
                                start_x: f32::from(pos.x),
                                start_width,
                            });
                        });
                    }
                    HeaderHit::SortName | HeaderHit::SortMeta(_) => {
                        *press_dn.borrow_mut() = Some(HeaderPress {
                            hit,
                            start_screen_x: f32::from(pos.x),
                            moved: false,
                        });
                    }
                }
                cx.stop_propagation();
            });

            // Right-click Type filter (needs Window)
            let layout_rt = layout_snap.clone();
            let right_view = app_entity.clone();
            let right_hitbox = prepaint.hitbox.clone();
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Right {
                    return;
                }
                if !right_hitbox.is_hovered(window) {
                    return;
                }
                let pos = ev.position;
                if !bounds.contains(&pos) {
                    return;
                }
                let local_x = f32::from(pos.x - bounds.origin.x);
                if !matches!(
                    hit_test(&layout_rt, local_x, width),
                    Some(HeaderHit::SortMeta(DetailsColumn::Type))
                ) {
                    return;
                }
                let labels = type_labels.clone();
                right_view.update(cx, |this, cx| {
                    this.open_type_filter_menu(pane_id, pos, labels, window, cx);
                });
                cx.stop_propagation();
            });

            // Move: live column reorder
            let press_mv = press.clone();
            let move_view = app_entity.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let mut press = press_mv.borrow_mut();
                if let Some(p) = press.as_mut() {
                    let dx = (f32::from(ev.position.x) - p.start_screen_x).abs();
                    if dx > 4.0 {
                        p.moved = true;
                        if let HeaderHit::SortMeta(column) = p.hit {
                            let local_x = f32::from(ev.position.x - bounds.origin.x);
                            let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
                            move_view.update(cx, |this, cx| {
                                this.reorder_details_live(pane_id, column, local_x, zoom);
                                this.notify_pane(pane_id, cx);
                            });
                        }
                    }
                } else if let Some(column) = move_view.read(cx).col_dragging {
                    let local_x = f32::from(ev.position.x - bounds.origin.x);
                    let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
                    move_view.update(cx, |this, cx| {
                        this.reorder_details_live(pane_id, column, local_x, zoom);
                        this.notify_pane(pane_id, cx);
                    });
                }
            });

            // Up: sort click or finish reorder
            let press_up = press;
            let up_view = app_entity;
            let sort_col_up = sort_col.clone();
            let sort_dir_up = sort_dir.clone();
            window.on_mouse_event::<MouseUpEvent>(move |ev, phase, _window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                    return;
                }
                let Some(p) = press_up.borrow_mut().take() else {
                    return;
                };
                if p.moved {
                    up_view.update(cx, |this, cx| {
                        if this.col_dragging.is_some() {
                            this.finish_details_reorder();
                            this.notify_all_panes(cx);
                        }
                    });
                    return;
                }
                let (col, next_dir) = match p.hit {
                    HeaderHit::SortName => {
                        let next = if sort_col_up == SortColumn::Name && sort_dir_up == SortDir::Asc
                        {
                            SortDir::Desc
                        } else {
                            SortDir::Asc
                        };
                        (SortColumn::Name, next)
                    }
                    HeaderHit::SortMeta(column) => {
                        let sort = sort_for_column(column);
                        let next = if sort_col_up == sort && sort_dir_up == SortDir::Asc {
                            SortDir::Desc
                        } else {
                            SortDir::Asc
                        };
                        (sort, next)
                    }
                    _ => return,
                };
                up_view.update(cx, |this, cx| {
                    this.process_actions(vec![Action::SetSort { col, dir: next_dir }]);
                    cx.notify();
                });
            });
        });
    }
}
