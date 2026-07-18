//! Custom GPUI `Element` for the Details body (rows only).
//!
//! Replaces the dual-canvas + event layer stack with one leaf element that:
//! - syncs `App.details_row_rings` (shape on miss only)
//! - paints clipped rows
//! - registers mouse handlers for hit-test interactions
//!
//! Scroll chrome stays outside (track_scroll + scrollbar) for parity with rubber/ctrl-zoom.

use crate::app::{App, DetailsColumn, DetailsHitModel};
use crate::ui::drag_hover::{
    DragHoverCommand, DragHoverDropCommand, DragHoverId, DragHoverKind, DragHoverTarget,
};
use crate::ui::icons::AtlasIcon;
use crate::ui::views::details::{
    hsla_key_bits, DetailsColors, DetailsRowLayout, CELL_PAD, DETAILS_ROW_H, ROW_ICON_SIZE,
    ROW_PAD_X, ROW_TEXT_Y_NUDGE,
};
use crate::ui::views::details_ring::{self, DetailsRingMeta, DetailsRowSlot};
use atlas_model::file_entry::FileKind;
use atlas_model::pane::TabId;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, Corners, DispatchPhase, Element,
    ElementId, Font, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Refineable, SharedString, Style, StyleRefinement,
    Styled, TextAlign, TextRun, WeakEntity, Window,
};
use std::sync::Arc;

/// Toggle lives in `details::USE_DETAILS_BODY_ELEMENT`.
pub(crate) struct DetailsBodyElement {
    id: ElementId,
    pane_id: TabId,
    app: WeakEntity<App>,
    visible: Arc<Vec<usize>>,
    row_h: f32,
    scroll_y: f32,
    viewport_h: f32,
    start_idx: usize,
    end_idx: usize,
    layout: DetailsRowLayout,
    colors: DetailsColors,
    font: Font,
    font_size: Pixels,
    style: StyleRefinement,
}

/// Data produced in prepaint for paint (ring slots + geometry).
pub(crate) struct DetailsBodyPrepaint {
    rows: Vec<DetailsRowSlot>,
    layout: DetailsRowLayout,
}

impl DetailsBodyElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: TabId,
        app: WeakEntity<App>,
        visible: Arc<Vec<usize>>,
        row_h: f32,
        scroll_y: f32,
        viewport_h: f32,
        start_idx: usize,
        end_idx: usize,
        layout: DetailsRowLayout,
        colors: DetailsColors,
        font: Font,
        font_size: Pixels,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("details-body-{}", pane_id.0))),
            pane_id,
            app,
            visible,
            row_h,
            scroll_y,
            viewport_h,
            start_idx,
            end_idx,
            layout,
            colors,
            font,
            font_size,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for DetailsBodyElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for DetailsBodyElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for DetailsBodyElement {
    type RequestLayoutState = Style;
    type PrepaintState = DetailsBodyPrepaint;

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
        crate::perf::count("details.element.layout");
        let mut style = Style::default();
        style.refine(&self.style);
        // Fill the scroll-content parent (explicit height set by outer div).
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
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
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        let _perf = crate::perf::span("details.element.prepaint");
        crate::perf::count("details.element.prepaint");
        // Per-pane wall time also rolls into the frame sum via Span::drop → time_us.

        let pane_id = self.pane_id;
        let visible = self.visible.clone();
        let start_idx = self.start_idx;
        let end_idx = self.end_idx;
        let row_h = self.row_h;
        let scroll_y = self.scroll_y;
        let viewport_h = self.viewport_h;
        let layout = self.layout.clone();
        let font = self.font.clone();
        let font_size = self.font_size;
        let colors = self.colors;
        let bounds_w = f32::from(bounds.size.width);
        let visible_rows = end_idx.saturating_sub(start_idx);

        let Some(app_entity) = self.app.upgrade() else {
            return DetailsBodyPrepaint {
                rows: Vec::new(),
                layout,
            };
        };

        // Hit model + drag-hover folder targets (same as monolitico canvas).
        let hit_model = DetailsHitModel {
            bounds: gpui::Bounds {
                origin: point(bounds.origin.x, bounds.origin.y + px(scroll_y)),
                size: size(bounds.size.width, px(viewport_h)),
            },
            scroll_y,
            row_h,
            visible: visible.clone(),
            start_idx,
            end_idx,
            total_count: visible.len(),
        };

        let ts = window.text_system();

        // Summed across all panes → details.element.ring_ms in frame_summary.
        let _ring_perf = crate::perf::span("details.element.ring");
        let rows = app_entity.update(cx, |app, _| {
            app.details_hit.insert(pane_id, hit_model.clone());
            if app.drag_hover_maybe_active() {
                let targets = app
                    .state
                    .tab_states
                    .get(&pane_id)
                    .map(|tab_state| {
                        (start_idx..end_idx)
                            .filter_map(|visible_pos| {
                                let entry_idx = *visible.get(visible_pos)?;
                                let entry = tab_state.entries.get(entry_idx)?;
                                matches!(entry.kind, FileKind::Dir).then(|| {
                                    let id = DragHoverId::Folder(pane_id, entry.path.clone());
                                    DragHoverTarget {
                                        id,
                                        bounds: hit_model.row_bounds(visible_pos),
                                        kind: DragHoverKind::Folder,
                                        command: DragHoverCommand::NavigatePathIn(
                                            pane_id,
                                            entry.path.clone(),
                                        ),
                                        drop_command: Some(DragHoverDropCommand::TransferTo(
                                            entry.path.clone(),
                                        )),
                                    }
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for target in targets {
                    app.register_drag_hover_target(target);
                }
            }

            struct RowBuild {
                row: usize,
                entry_idx: usize,
                selected: bool,
                hovered: bool,
                flashing: bool,
                odd: bool,
                dimmed: bool,
                shape_key: u64,
                name: String,
                display_size: String,
                display_date: String,
                display_type: String,
                name_color: Hsla,
                meta_color: Hsla,
                icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
                fallback_icon_path: Option<&'static str>,
            }

            let builds: Vec<RowBuild> = {
                let Some(tab_state) = app.state.tab_states.get(&pane_id) else {
                    return Vec::new();
                };
                let mut builds = Vec::with_capacity(visible_rows);
                for i in start_idx..end_idx {
                    let Some(&entry_idx) = visible.get(i) else {
                        continue;
                    };
                    let Some(entry) = tab_state.entries.get(entry_idx) else {
                        continue;
                    };
                    if crate::ui::views::rename::active_input(app, pane_id, entry_idx).is_some() {
                        continue;
                    }

                    let selected = tab_state.selection.selected.contains(&entry_idx)
                        && !crate::ui::views::rename::suppress_row_chrome(app, pane_id, entry_idx);
                    let dimmed = crate::ui::views::rename::dim_other_entry(app, pane_id, entry_idx);
                    let flashing = matches!(entry.kind, FileKind::Dir)
                        && app.drag_hover.blink_id.as_ref()
                            == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                    let hovered = app.details_hover == Some((pane_id, entry_idx));
                    let odd = i % 2 == 1;

                    let name_color = if selected {
                        colors.accent_fg
                    } else {
                        colors.fg
                    };
                    let name_color = if dimmed {
                        name_color.opacity(0.45)
                    } else {
                        name_color
                    };
                    let meta_color = if selected {
                        colors.accent_fg
                    } else {
                        colors.muted
                    };
                    let meta_color = if dimmed {
                        meta_color.opacity(0.45)
                    } else {
                        meta_color
                    };

                    let kind_disc = match entry.kind {
                        FileKind::Dir => 0u8,
                        _ => 1u8,
                    };
                    let shape_key = details_ring::details_row_shape_key(
                        entry_idx,
                        &entry.name,
                        &entry.display_size,
                        &entry.display_date,
                        &entry.display_type,
                        kind_disc,
                        entry.extension.as_deref(),
                        selected,
                        dimmed,
                        font_size,
                        hsla_key_bits(name_color),
                        hsla_key_bits(meta_color),
                    );

                    let icon_tex = app
                        .file_icon_for_entry(&entry.path, &entry.kind, entry.extension.as_deref())
                        .texture();
                    let fallback_icon_path = if icon_tex.is_none() {
                        Some(match entry.kind {
                            FileKind::Dir => AtlasIcon::Folder.path(),
                            _ => AtlasIcon::File.path(),
                        })
                    } else {
                        None
                    };

                    builds.push(RowBuild {
                        row: i,
                        entry_idx,
                        selected,
                        hovered,
                        flashing,
                        odd,
                        dimmed,
                        shape_key,
                        name: entry.name.to_string(),
                        display_size: entry.display_size.to_string(),
                        display_date: entry.display_date.to_string(),
                        display_type: entry.display_type.to_string(),
                        name_color,
                        meta_color,
                        icon_tex,
                        fallback_icon_path,
                    });
                }
                builds
            };

            let cols = layout.columns.clone();
            let ring = app.details_row_rings.entry(pane_id).or_default();
            ring.ensure_capacity((visible_rows + 8).max(16));

            for b in &builds {
                let name = b.name.clone();
                let display_size = b.display_size.clone();
                let display_date = b.display_date.clone();
                let display_type = b.display_type.clone();
                let font = font.clone();
                let cols = cols.clone();
                let name_color = b.name_color;
                let meta_color = b.meta_color;
                let icon_tex = b.icon_tex.clone();
                let fallback_icon_path = b.fallback_icon_path;
                let shape_key = b.shape_key;
                let entry_idx = b.entry_idx;
                let row_i = b.row;

                ring.sync_row(row_i, shape_key, || {
                    let name_run = TextRun {
                        len: name.len(),
                        font: font.clone(),
                        color: name_color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let name_s = ts.shape_line(name.into(), font_size, &[name_run], None);

                    let mut meta_cols = Vec::new();
                    for col in &cols {
                        if col.x < bounds_w && col.w > 1.0 {
                            let text: SharedString = match col.column {
                                DetailsColumn::Size => display_size.clone().into(),
                                DetailsColumn::Modified => display_date.clone().into(),
                                DetailsColumn::Type => display_type.clone().into(),
                            };
                            let run = TextRun {
                                len: text.len(),
                                font: font.clone(),
                                color: meta_color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            meta_cols.push(DetailsRingMeta {
                                column: col.column,
                                line: ts.shape_line(text, font_size, &[run], None),
                            });
                        }
                    }

                    DetailsRowSlot {
                        row: row_i,
                        entry_idx,
                        shape_key,
                        selected: b.selected,
                        hovered: b.hovered,
                        flashing: b.flashing,
                        odd: b.odd,
                        dimmed: b.dimmed,
                        name_s,
                        meta_cols,
                        icon_tex,
                        fallback_icon_path,
                    }
                });
                ring.touch_flags(
                    row_i, shape_key, b.selected, b.hovered, b.flashing, b.odd, b.dimmed,
                );
            }

            ring.collect_rows(start_idx, end_idx)
        });

        crate::perf::count_by("details.element.rows", rows.len() as u64);

        DetailsBodyPrepaint { rows, layout }
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
        let _perf = crate::perf::span("details.element.paint");
        crate::perf::count("details.element.paint");

        let pane_id = self.pane_id;
        let app = self.app.clone();
        let row_h = self.row_h;
        let accent = self.colors.accent;
        let hover = self.colors.hover;
        let muted = self.colors.muted;
        let stripe_bg = self.colors.stripe_bg;

        style.paint(bounds, window, cx, |window, cx| {
            let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
            let row_icon_size = ROW_ICON_SIZE * zoom_scale;
            let row_pad_x = ROW_PAD_X * zoom_scale;
            let cell_pad = CELL_PAD * zoom_scale;
            let name_col_w = prepaint
                .layout
                .name_col_w
                .min((f32::from(bounds.size.width) - prepaint.layout.name_x).max(0.0));

            for row in &prepaint.rows {
                let y = bounds.origin.y + px(row.row as f32 * row_h);
                let row_bounds = Bounds {
                    origin: point(bounds.origin.x, y),
                    size: size(bounds.size.width, px(row_h)),
                };

                let bg = if row.selected {
                    Some(accent)
                } else if row.flashing {
                    Some(accent.opacity(0.38))
                } else if row.hovered {
                    Some(hover)
                } else if row.odd {
                    Some(stripe_bg)
                } else {
                    None
                };
                if let Some(mut color) = bg {
                    if row.dimmed {
                        color = color.opacity(color.a * 0.45);
                    }
                    window.paint_quad(gpui::fill(row_bounds, color));
                }

                let text_y = y + px(ROW_TEXT_Y_NUDGE * zoom_scale);

                if let Some(tex) = &row.icon_tex {
                    let _ = window.paint_image(
                        Bounds {
                            origin: point(
                                bounds.origin.x + px(row_pad_x),
                                y + px((row_h - row_icon_size) * 0.5),
                            ),
                            size: size(px(row_icon_size), px(row_icon_size)),
                        },
                        Corners::default(),
                        tex.clone(),
                        0,
                        false,
                    );
                } else if let Some(path) = row.fallback_icon_path {
                    let _ = window.paint_svg(
                        Bounds {
                            origin: point(
                                bounds.origin.x + px(row_pad_x),
                                y + px((row_h - row_icon_size) * 0.5),
                            ),
                            size: size(px(row_icon_size), px(row_icon_size)),
                        },
                        SharedString::from(path),
                        None,
                        Default::default(),
                        muted,
                        cx,
                    );
                }

                let name_x = bounds.origin.x + px(prepaint.layout.name_x);
                let name_clip = Bounds {
                    origin: point(name_x, y),
                    size: size(px(name_col_w.max(0.0)), px(row_h)),
                };
                window.with_content_mask(Some(ContentMask { bounds: name_clip }), |window| {
                    let _ = row.name_s.paint(
                        point(name_x, text_y),
                        px(row_h),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                });

                for meta in &row.meta_cols {
                    let Some(layout_col) = prepaint
                        .layout
                        .columns
                        .iter()
                        .find(|col| col.column == meta.column)
                    else {
                        continue;
                    };
                    let col_x = bounds.origin.x + px(layout_col.x);
                    let col_w = layout_col.w;
                    let text_x = col_x + px(cell_pad.min(col_w));
                    let meta_clip = Bounds {
                        origin: point(col_x, y),
                        size: size(px(col_w.max(0.0)), px(row_h)),
                    };
                    window.with_content_mask(Some(ContentMask { bounds: meta_clip }), |window| {
                        let _ = meta.line.paint(
                            point(text_x, text_y),
                            px(row_h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    });
                }
            }

            // Input: same handlers as the old event canvas.
            if let Some(app_entity) = app.upgrade() {
                let move_view = app_entity.clone();
                window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || ev.dragging() {
                        return;
                    }
                    let pos = ev.position;
                    move_view.update(cx, |this, cx| {
                        let hover_entry = this.details_hover_at(pane_id, pos);
                        if this.details_hover != hover_entry {
                            gpui_component::tooltip::Tooltip::dismiss(window, cx);
                            this.details_hover = hover_entry;
                            this.notify_pane(pane_id, cx);
                        }
                    });
                });

                let down_view = app_entity;
                window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let pos = ev.position;
                    if !down_view.read(cx).details_contains_point(pane_id, pos) {
                        return;
                    }
                    down_view.update(cx, |this, cx| {
                        let hit = this.details_entry_at(pane_id, pos);
                        match ev.button {
                            MouseButton::Left => this.details_left_mouse_down(
                                pane_id,
                                hit,
                                pos,
                                ev.modifiers.shift,
                                ev.modifiers.control || ev.modifiers.platform,
                                ev.click_count,
                                cx,
                            ),
                            MouseButton::Right => {
                                this.details_right_mouse_down(pane_id, hit, pos, window, cx)
                            }
                            _ => {}
                        }
                    });
                    if matches!(ev.button, MouseButton::Left | MouseButton::Right) {
                        cx.stop_propagation();
                    }
                });
            }
        });
    }
}

// Silence unused import if DETAILS_ROW_H only used externally later.
const _: f32 = DETAILS_ROW_H;
