//! Details view (table): `render_pane` (dispatch + Details body) + row/header cells.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::{App, ColResizeState, DetailsColumn, RubberTarget};
use crate::input::action::Action;
use crate::ui::drag_hover::DragHoverId;
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_header_fill};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::root::{ctrl_scroll, pane_bounds_recorder, rect_recorder, zone_recorder};
use atlas_model::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{
    div, img, px, AnyElement, Context, Div, Entity, Hsla, MouseButton, Pixels, SharedString,
    Stateful, Window,
};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme, ElementExt};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct DetailsColors {
    pub fg: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub hover: Hsla,
    pub muted: Hsla,
    pub header_bg: Hsla,
    pub stripe_bg: Hsla,
    pub border: Hsla,
}

#[derive(Clone)]
struct DetailsColumnDrag {
    column: DetailsColumn,
}

impl Render for DetailsColumnDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Invisible: the visible floating header box is drawn by `details_header` at a
        // fixed y so it only tracks the cursor horizontally.
        div()
    }
}

impl App {
    /// Render the file view for a pane (Details/Grid/List/Miller). Public to the
    /// crate so `ui::pane_panel::FilePanel` (a Dock `Panel`) can delegate here.
    pub(crate) fn render_pane(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.space_lens.contains_key(&pane_id) {
            return self.render_space_lens(pane_id, window, cx);
        }
        // Keep the last hit-test rects while this pane owns an active rubber drag;
        // cached content may not prepaint immediately after a scoped repaint.
        if !self
            .rubber
            .as_ref()
            .is_some_and(|r| r.target == RubberTarget::Pane(pane_id))
        {
            self.row_rects.borrow_mut().remove(&pane_id);
        }
        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        self.scroll_handle_for(pane_id);
        let should_scroll_selected = self
            .state
            .tab_states
            .get(&pane_id)
            .is_some_and(|ts| ts.selection.scroll_to_selected);
        if should_scroll_selected
            && self.scroll_selected_into_view(pane_id, gpui::ScrollStrategy::Center)
        {
            if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
                ts.selection.scroll_to_selected = false;
            }
        }
        let mode_name = match &view_mode {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
            ViewMode::Miller => "miller",
            ViewMode::Details => "details",
        };
        match &view_mode {
            ViewMode::Grid => crate::perf::count("render.view.grid_dispatch"),
            ViewMode::List => crate::perf::count("render.view.list_dispatch"),
            ViewMode::Miller => crate::perf::count("render.view.miller_dispatch"),
            ViewMode::Details => crate::perf::count("render.view.details"),
        }
        let (entries, visible) = self
            .state
            .tab_states
            .get(&pane_id)
            .map_or((0, 0), |ts| (ts.entries.len(), ts.visible.len()));
        let _perf = crate::perf::span_detail("pane.render", || {
            format!(
                "pane={} mode={mode_name} entries={entries} visible={visible}",
                pane_id.0
            )
        });
        match view_mode {
            ViewMode::Grid => return self.render_grid(pane_id, window, cx),
            ViewMode::List => return self.render_list_compact(pane_id, window, cx),
            ViewMode::Miller => return self.render_miller(pane_id, window, cx),
            ViewMode::Details => {}
        }
        let view = cx.entity();
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let (fg, accent, accent_fg, hover, muted, header_bg, stripe_bg, border) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                if figma_glass {
                    glass_header_fill()
                } else {
                    backdrop_tint(t.secondary, 0.18)
                },
                if figma_glass {
                    t.list_even.opacity(0.12)
                } else {
                    backdrop_tint(t.list_even, 0.08)
                },
                t.border,
            )
        };
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let row_font = window.text_style().font();

        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, border);
        let (sort_col, sort_dir) = self
            .tab_data_for(pane_id)
            .map(|t| (t.sort_col.clone(), t.sort_dir.clone()))
            .unwrap_or_default();

        // Zoom-scaled once here; threaded as already-real-px through the row/header
        // geometry below so every consumption site can stay a plain `px(...)`.
        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let col_widths = self.details_widths_for_pane(pane_id, zoom_scale);
        let column_order = self.details_column_order;
        // Single source of geometry for header AND rows → pixel-perfect alignment.
        // Left-anchored widths (Explorer-style); they stay fixed when there is room
        // and shrink proportionally only until readability minimums in narrow splits.
        // While a reorder slide is running, blend from the pre-swap order so columns
        // glide; request_animation_frame here re-renders only THIS pane (scoped).
        let to_layout = details_row_layout(col_widths, &column_order, zoom_scale);
        let layout = Arc::new(match self.details_reorder_anim() {
            Some(anim) => {
                crate::perf::event(|| format!("details.reorder_anim pane={}", pane_id.0));
                window.request_animation_frame();
                blend_layouts(
                    &details_row_layout(col_widths, &anim.from_order, zoom_scale),
                    &to_layout,
                    anim.progress(),
                )
            }
            None => to_layout,
        });
        let row_h = DETAILS_ROW_H * zoom_scale;
        let dragging = self.col_dragging;
        let drag_x = self.col_drag_x;
        let type_filter_set = self.type_filters.get(&pane_id).cloned().unwrap_or_default();
        let has_type_filter = !type_filter_set.is_empty();

        // type_labels: rebuilt only when entries change (entries_version gate in TabState).
        // filtered_visible: Arc::clone in the common no-filter case (O(1), no alloc).
        let (type_labels, filtered_visible) =
            if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
                let labels = ts.type_labels_cached(entry_type_label).to_vec();
                ts.visible_indices(); // ensure visible cache is fresh before we borrow .visible
                let fv = if type_filter_set.is_empty() {
                    Arc::clone(&ts.visible)
                } else {
                    let vis = Arc::clone(&ts.visible);
                    let entries = &ts.entries;
                    Arc::new(
                        vis.iter()
                            .filter(|&&idx| {
                                entries
                                    .get(idx)
                                    .is_some_and(|e| type_filter_set.contains(&entry_type_label(e)))
                            })
                            .copied()
                            .collect(),
                    )
                };
                (labels, fv)
            } else {
                (vec![], Arc::new(vec![]))
            };
        let count = filtered_visible.len();

        let colors = DetailsColors {
            fg,
            accent,
            accent_fg,
            hover,
            muted,
            header_bg,
            stripe_bg,
            border,
        };

        let header = details_header(
            &layout,
            &view,
            pane_id,
            &sort_col,
            &sort_dir,
            has_type_filter,
            &type_labels,
            dragging,
            drag_x,
            &colors,
            row_font.clone(),
            font_size,
        );

        let total_height = count as f32 * row_h;
        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, rust_i18n::t!("empty_folder").into_owned());
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .on_mouse_down(
                    MouseButton::Left,
                    crate::ui::views::on_pane_rubber_mouse_down(&view, pane_id),
                )
                .children(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let base_scroll = scroll.0.borrow().base_handle.clone();
            let scroll_view = view.clone();
            // First frames after a split often report 0-height scroll bounds (layout
            // not committed yet). Shaping with viewport_h=0 builds an empty canvas and
            // used to stick for the whole seam animation via cache force-reuse.
            let measured_h = f32::from(base_scroll.bounds().size.height);
            let viewport_h = if measured_h > row_h {
                measured_h
            } else {
                self.pane_layout_bounds
                    .get(&pane_id)
                    .map(|b| f32::from(b.size.height))
                    .filter(|h| *h > row_h)
                    .or_else(|| {
                        self.pane_tabs_bounds
                            .get(&pane_id)
                            .map(|b| f32::from(b.size.height))
                            .filter(|h| *h > row_h)
                    })
                    .unwrap_or(row_h * 24.0)
            };
            let scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            let _list_view = view.clone();
            let fv = filtered_visible.clone();
            let row_layout = Arc::clone(&layout);

            // The scroll transform can advance before this virtual Element gets its
            // next render. Keep a generous paint/shaping runway on both sides so a
            // fast wheel/trackpad fling never exposes an unpainted (black) frame.
            let overscan_rows = ((viewport_h * DETAILS_SCROLL_OVERSCAN_VIEWPORTS) / row_h)
                .ceil()
                .max(12.0) as usize;
            let start_idx =
                ((scroll_y / row_h).floor().max(0.0) as usize).saturating_sub(overscan_rows);
            let end_idx = ((scroll_y + viewport_h) / row_h).ceil() as usize + overscan_rows;
            let end_idx = end_idx.min(count);
            prime_details_file_icons_range(self, pane_id, &filtered_visible, start_idx, end_idx);

            let mut overlays = Vec::new();

            let Some(ts) = self.state.tab_states.get(&pane_id) else {
                return div().into_any_element();
            };
            let record_rects =
                self.dock_split_animations.is_empty() && self.pane_layout_animations.is_empty();

            for i in start_idx..end_idx {
                let Some(&entry_idx) = fv.get(i) else {
                    continue;
                };
                let Some(entry) = ts.entries.get(entry_idx) else {
                    continue;
                };

                let rename_input = crate::ui::views::rename::active_input(self, pane_id, entry_idx);
                let selected = ts.selection.selected.contains(&entry_idx)
                    && !crate::ui::views::rename::suppress_row_chrome(self, pane_id, entry_idx);
                let flashing = matches!(entry.kind, FileKind::Dir)
                    && self.drag_hover.blink_id.as_ref()
                        == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                let odd = i % 2 == 1;
                let icon_tex = self
                    .file_icon_for_entry(&entry.path, &entry.kind, entry.extension.as_deref())
                    .texture();

                if let Some(input) = rename_input {
                    let overlay = render_rename_row(
                        entry,
                        entry_idx,
                        odd,
                        selected,
                        pane_id,
                        &view,
                        &colors,
                        input,
                        flashing,
                        record_rects,
                        icon_tex,
                        &row_layout,
                        window,
                        font_size,
                    );
                    overlays.push(
                        div()
                            .absolute()
                            .top(px(i as f32 * row_h))
                            .left_0()
                            .w_full()
                            .h(px(row_h))
                            .child(overlay),
                    );
                }
            }

            // Body: custom Element (ring + paint + input).
            let body_el = if USE_DETAILS_BODY_ELEMENT {
                crate::ui::views::details_element::DetailsBodyElement::new(
                    pane_id,
                    view.downgrade(),
                    fv.clone(),
                    row_h,
                    scroll_y,
                    viewport_h,
                    start_idx,
                    end_idx,
                    (*layout).clone(),
                    colors,
                    row_font.clone(),
                    font_size,
                )
                .absolute()
                .inset_0()
                .into_any_element()
            } else {
                // A/B only: empty body when flag off (set true for production path).
                div().absolute().inset_0().into_any_element()
            };

            let details_drag_layer = div()
                .id(("details-drag-layer", pane_id.0))
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(total_height))
                .opacity(0.0)
                .on_drag(
                    crate::ui::drag::file_drag_value(Vec::new()),
                    crate::ui::drag::file_drag_handler_for_details_content(view.clone(), pane_id),
                );
            let details_tooltip = self
                .details_hover
                .and_then(|(hover_pane, entry_idx)| (hover_pane == pane_id).then_some(entry_idx))
                .and_then(|entry_idx| {
                    self.state
                        .tab_states
                        .get(&pane_id)
                        .and_then(|ts| ts.entries.get(entry_idx))
                })
                .map(|entry| {
                    (
                        entry.name.clone(),
                        entry.path.clone(),
                        matches!(entry.kind, FileKind::Dir),
                        entry.size,
                        entry.modified,
                    )
                });

            let tooltip_target_id = self
                .details_hover
                .filter(|(hover_pane, _)| *hover_pane == pane_id)
                .map(|(_, entry_idx)| entry_idx as u64)
                .unwrap_or(u64::MAX);

            div()
                // Re-key only the viewport-level hover target when the hit-tested row
                // changes. The scroll child keeps its stable id and external scroll state.
                .id(SharedString::from(format!(
                    "details-tooltip-target-{}-{tooltip_target_id}",
                    pane_id.0
                )))
                .size_full()
                .relative()
                .overflow_hidden()
                .min_w(px(0.))
                .min_h(px(0.))
                .when_some(
                    details_tooltip,
                    |el, (name, path, is_dir, size, modified)| {
                        el.tooltip(move |window, cx| {
                            crate::ui::tooltips::entry_hover_tooltip(
                                name.clone(),
                                path.clone(),
                                is_dir,
                                size,
                                modified,
                                window,
                                cx,
                            )
                        })
                    },
                )
                .child(
                    div()
                        .id(SharedString::from(format!("details-scroll-{}", pane_id.0)))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&base_scroll)
                        .on_scroll_wheel(move |ev, window, cx| {
                            ctrl_scroll(ev, pane_id, &scroll_view, window, cx)
                        })
                        .child(
                            div()
                                .w_full()
                                .h(px(total_height))
                                .relative()
                                .child(body_el)
                                .child(details_drag_layer)
                                .children(overlays),
                        )
                        .pr(crate::ui::rem_px(16.)),
                )
                .vertical_scrollbar(&base_scroll)
                .into_any_element()
        };
        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);

        let zone_view = view.clone();
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let pane_view = view;
        crate::ui::views::pane_background_interactions(
            v_flex()
                .id(SharedString::from(format!("pane-{}", pane_id.0)))
                .size_full()
                .overflow_hidden(),
            &pane_view,
            pane_id,
        )
        .child(crumb_strip)
        .child(header)
        .child(inner)
        .child(filter_pill)
        .child(controls_pill)
        .on_prepaint({
            let v = prepaint_view;
            move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                crate::perf::count("prepaint.details_pane");
                let new_w = f32::from(bounds.size.width);
                let new_h = f32::from(bounds.size.height);
                let should_update = {
                    if let Some(ts) = v.read(cx).state.tab_states.get(&pane_id) {
                        (ts.last_pane_width - new_w).abs() > 1.5
                            || (ts.last_pane_height - new_h).abs() > 1.5
                    } else {
                        false
                    }
                };
                if should_update {
                    v.update(cx, |this, cx| {
                        if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                            ts.last_pane_width = new_w;
                            ts.last_pane_height = new_h;
                            // The responsive column projection depends on this
                            // measured width. One notification updates the next
                            // frame; the bounds comparison prevents a loop.
                            cx.notify();
                        }
                    });
                }
            }
        })
        .when(cfg!(windows), |el| {
            el.on_prepaint(zone_recorder(pane_id, &zone_view))
        })
        .on_prepaint(pane_bounds_recorder(pane_id, &bounds_view))
        .into_any_element()
    }
}

/// Semantic category label for the Type column and filter overlay.
pub fn entry_type_label(entry: &FileEntry) -> String {
    if !entry.display_type.is_empty() {
        return entry.display_type.to_string();
    }
    atlas_model::file_entry::file_category(&entry.kind, entry.extension.as_deref())
}

/// Fill the `display_*` fields of a fresh listing. Called on the background
/// worker (inside the producing task), never on the UI thread: formatting a
/// 500-entry listing during `App::render` cost 5-14ms per drain.
pub(crate) fn format_entries(entries: &mut [FileEntry]) {
    for e in entries.iter_mut() {
        e.display_size = if matches!(e.kind, FileKind::Dir) {
            "".into()
        } else {
            e.size
                .map(atlas_core::util::display::format_size)
                .unwrap_or_default()
                .into()
        };
        e.display_date = e
            .modified
            .map(atlas_core::util::display::format_date)
            .unwrap_or_default()
            .into();
        e.display_type = entry_type_label(e).into();
    }
}

impl App {
    fn details_widths_for_pane(&self, pane_id: TabId, zoom_scale: f32) -> crate::app::ColWidths {
        let available_width = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|state| state.last_pane_width)
            .filter(|width| width.is_finite() && *width > 1.0);
        fit_details_widths(self.col_widths, available_width, zoom_scale)
    }

    /// Live drag-reorder: place `dragged` at the slot the cursor is over (`local_x`
    /// is header-relative). Every position is reachable, including the last (when the
    /// cursor passes every other column's centre). On a real change we snapshot the
    /// previous order so the columns *slide* to their new slots instead of snapping.
    pub(crate) fn reorder_details_live(
        &mut self,
        pane_id: TabId,
        dragged: DetailsColumn,
        local_x: f32,
        zoom_scale: f32,
    ) {
        self.col_dragging = Some(dragged);
        self.col_drag_x = local_x;
        let old = self.details_column_order;

        // Insertion index = how many of the *other* columns the cursor has passed
        // (compared against each column's centre in the current left-anchored layout).
        // Matches `details_row_layout`'s `name_w` computation.
        let widths = self.details_widths_for_pane(pane_id, zoom_scale);
        let mut x = widths.name.max(ROW_NAME_MIN_W * zoom_scale);
        let mut passed = 0usize;
        for column in old {
            let w = details_column_width_raw(column, widths.size, widths.modified, widths.type_w);
            if column != dragged && local_x > x + w * 0.5 {
                passed += 1;
            }
            x += w;
        }

        let mut order: Vec<DetailsColumn> = old.iter().copied().filter(|c| *c != dragged).collect();
        order.insert(passed.min(order.len()), dragged);
        if let Ok(order) = <[DetailsColumn; 3]>::try_from(order) {
            if order != old {
                self.details_column_order = order;
                self.details_reorder = Some(crate::app::DetailsReorderAnim::new(old));
            }
        }
    }

    /// End a drag-reorder: drop the floating state and persist the final order.
    pub(crate) fn finish_details_reorder(&mut self) {
        if self.col_dragging.take().is_some() {
            self.persist_details_columns();
        }
    }
}

/// The Details header: one absolutely-positioned strip sharing the exact `x`/`w`
/// geometry of the canvas rows below it, so labels and data never drift and the
/// continuous collapse hides the same columns in lockstep.
fn details_header(
    layout: &DetailsRowLayout,
    view: &Entity<App>,
    pane_id: TabId,
    sort_col: &SortColumn,
    sort_dir: &SortDir,
    has_type_filter: bool,
    type_labels: &[String],
    dragging: Option<DetailsColumn>,
    drag_x: f32,
    colors: &DetailsColors,
    font: gpui::Font,
    font_size: gpui::Pixels,
) -> gpui::AnyElement {
    crate::ui::views::details_header_element::DetailsHeaderElement::new(
        pane_id,
        view.downgrade(),
        layout.clone(),
        colors.clone(),
        font,
        font_size,
        sort_col.clone(),
        sort_dir.clone(),
        has_type_filter,
        type_labels.to_vec(),
        dragging,
        drag_x,
    )
    .into_any_element()
}

fn details_column_label(column: DetailsColumn) -> String {
    match column {
        DetailsColumn::Size => rust_i18n::t!("column_size").into_owned(),
        DetailsColumn::Modified => rust_i18n::t!("column_modified").into_owned(),
        DetailsColumn::Type => rust_i18n::t!("column_type").into_owned(),
    }
}

/// One absolutely-positioned metadata header cell: sort/type content (clipped),
/// reorder drag source (the floating box is the gpui drag image), and a right-edge
/// resize handle. While this column is the one being dragged it's hidden in place,
/// leaving a gap the others slide around.
fn details_header_cell(
    col: DetailsColumnLayout,
    view: &Entity<App>,
    pane_id: TabId,
    sort_col: &SortColumn,
    sort_dir: &SortDir,
    has_type_filter: bool,
    type_labels: &[String],
    is_dragging: bool,
    colors: &DetailsColors,
) -> AnyElement {
    let column = col.column;
    let content = match column {
        DetailsColumn::Size | DetailsColumn::Modified => {
            let sort = if column == DetailsColumn::Size {
                SortColumn::Size
            } else {
                SortColumn::Modified
            };
            sort_cell(
                view,
                pane_id,
                sort,
                &details_column_label(column),
                sort_col,
                sort_dir,
                colors,
            )
            .into_any_element()
        }
        DetailsColumn::Type => type_header_cell(
            view,
            pane_id,
            sort_col,
            sort_dir,
            has_type_filter,
            type_labels.to_vec(),
            colors,
        )
        .into_any_element(),
    };
    div()
        .id(SharedString::from(format!(
            "details-col-{}-{}",
            pane_id.0,
            column.key()
        )))
        .absolute()
        .left(px(col.x))
        .top_0()
        .bottom_0()
        .w(px(col.w))
        .flex()
        .items_center()
        // While dragging, hide the source in place so its slot reads as the gap
        // the floating box will land in.
        .when(is_dragging, |s| s.opacity(0.0))
        // Clipped content, inset by CELL_PAD to match the row data alignment.
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .pl(crate::ui::rem_px(CELL_PAD))
                .overflow_hidden()
                .child(content),
        )
        .on_drag(
            DetailsColumnDrag { column },
            move |drag, _pos, _window, cx| {
                cx.new(|_| DetailsColumnDrag {
                    column: drag.column,
                })
            },
        )
        .into_any_element()
}

/// Wide (10px) grab zone straddling the column's RIGHT edge — easy to hit from
/// either side — with an always-visible 1px centred separator and a subtle accent
/// wash on hover so it reads as draggable. Drag right = wider (Explorer-style);
/// the global `rubber_layer` applies the delta on move (see `interactions.rs`).
fn col_resize_handle(
    col: &'static str,
    cur_width: f32,
    edge_x: f32,
    pane_id: TabId,
    view: &Entity<App>,
    border: Hsla,
    accent: Hsla,
) -> Stateful<Div> {
    let v = view.clone();
    div()
        .id(SharedString::from(format!("rh-{col}-{}", pane_id.0)))
        .absolute()
        .left(px(edge_x - 5.))
        .top_0()
        .bottom_0()
        .w(px(10.))
        .flex()
        .justify_center()
        .cursor_col_resize()
        .occlude()
        .hover(|s| s.bg(accent.opacity(0.18)))
        .child(div().h_full().w(px(1.)).bg(border.opacity(0.55)))
        .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
            let start_x = f32::from(ev.position.x);
            v.update(cx, |this, _cx| {
                this.col_resize = Some(ColResizeState {
                    col,
                    pane: pane_id,
                    start_x,
                    start_width: cur_width,
                });
            });
            cx.stop_propagation();
        })
}

/// Clickable column header that toggles sort column/direction.
/// Phosphor ArrowUp/Down icons indicate the active sort direction.
fn sort_cell(
    view: &Entity<App>,
    pane_id: TabId,
    col: SortColumn,
    label: &str,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
    colors: &DetailsColors,
) -> Stateful<Div> {
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if is_active {
        colors.accent
    } else {
        colors.muted
    };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, colors.accent))
    } else {
        None
    };
    let v = view.clone();
    div()
        .id(SharedString::from(format!("hdr-{}-{:?}", pane_id.0, col)))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .text_color(label_color)
        .child(label.to_string())
        .when_some(sort_icon, |el, ico| el.child(ico))
        .on_click(move |_ev, _window, cx| {
            let col = col.clone();
            let dir = next_dir.clone();
            v.update(cx, |this, cx| {
                this.process_actions(vec![Action::SetSort { col, dir }]);
                cx.notify();
            });
        })
}

/// Type column header: left-click sorts by extension, right-click opens the
/// searchable multi-select filter overlay.
fn type_header_cell(
    view: &Entity<App>,
    pane_id: TabId,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
    has_filter: bool,
    type_labels: Vec<String>,
    colors: &DetailsColors,
) -> Stateful<Div> {
    let col = SortColumn::Extension;
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if has_filter || is_active {
        colors.accent
    } else {
        colors.muted
    };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, colors.accent))
    } else {
        None
    };
    let v = view.clone();
    let vr = view.clone();
    div()
        .id(SharedString::from(format!("hdr-type-{}", pane_id.0)))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .text_color(label_color)
        .child(rust_i18n::t!("column_type"))
        // Active filter indicator: small dot next to the label.
        .when(has_filter, |el| {
            el.child(
                div()
                    .w(crate::ui::rem_px(5.))
                    .h(crate::ui::rem_px(5.))
                    .rounded_full()
                    .bg(colors.accent),
            )
        })
        .when_some(sort_icon, |el, ico| el.child(ico))
        .on_click(move |_ev, _window, cx| {
            let col = col.clone();
            let dir = next_dir.clone();
            v.update(cx, |this, cx| {
                this.process_actions(vec![Action::SetSort { col, dir }]);
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            let pos = ev.position;
            let labels = type_labels.clone();
            vr.update(cx, |this, cx| {
                this.open_type_filter_menu(pane_id, pos, labels, window, cx);
            });
            cx.stop_propagation();
        })
}

/// Fixed height for canvas rows — eliminates per-row flex layout cost.
pub(crate) const DETAILS_ROW_H: f32 = 24.0;
pub(crate) const ROW_PAD_X: f32 = 8.0;
const ROW_ICON_W: f32 = 20.0;
pub(crate) const ROW_ICON_SIZE: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const ROW_NAME_X: f32 = ROW_PAD_X + ROW_ICON_W + ROW_GAP;
const ROW_NAME_MIN_W: f32 = 88.0;
pub(crate) const ROW_TEXT_Y_NUDGE: f32 = -1.0;
/// Left inset of metadata text from its column's left edge (separator), so text
/// sits *inside* the column instead of hugging the divider. Applied identically
/// to the header label and the row data → they stay aligned.
pub(crate) const CELL_PAD: f32 = 11.0;
/// Practical minimums for responsive Details columns. Values are real pixels,
/// like the persisted user widths.
const DETAILS_SIZE_MIN_W: f32 = 46.0;
const DETAILS_MODIFIED_MIN_W: f32 = 92.0;
const DETAILS_TYPE_MIN_W: f32 = 56.0;

/// Extra virtualized rows retained on either side of the current Details viewport.
/// This is deliberately expressed in viewports, not a fixed row count, so it also
/// covers high-DPI / zoomed layouts and large trackpad deltas.
const DETAILS_SCROLL_OVERSCAN_VIEWPORTS: f32 = 1.5;

/// A/B: `true` = `DetailsBodyElement` (custom Element + ring); `false` = legacy dual-canvas.
pub(crate) const USE_DETAILS_BODY_ELEMENT: bool = true;

#[derive(Clone, Copy)]
pub(crate) struct DetailsColumnLayout {
    pub column: DetailsColumn,
    /// Left edge, row/header-relative (origin = pane left).
    pub x: f32,
    /// Column width.
    pub w: f32,
}

#[derive(Clone)]
pub(crate) struct DetailsRowLayout {
    pub name_x: f32,
    /// Width of the Name text area (Name column width minus the icon gutter).
    pub name_col_w: f32,
    /// Right edge of the Name column = left edge of the first metadata column.
    pub name_right: f32,
    /// Metadata columns in visual order (left→right, after Name).
    pub columns: Vec<DetailsColumnLayout>,
}

pub(crate) fn hsla_key_bits(c: Hsla) -> u32 {
    c.h.to_bits()
        ^ c.s.to_bits().rotate_left(8)
        ^ c.l.to_bits().rotate_left(16)
        ^ c.a.to_bits().rotate_left(24)
}

fn prime_details_file_icons_range(
    app: &mut App,
    pane_id: TabId,
    visible: &[usize],
    start: usize,
    end: usize,
) {
    let files = {
        let Some(ts) = app.state.tab_states.get(&pane_id) else {
            return;
        };
        let end = end.min(visible.len());
        (start..end)
            .filter_map(|i| {
                let entry = ts.entries.get(*visible.get(i)?)?;
                Some(crate::ui::file_icons::FileIconRef {
                    path: entry.path.clone(),
                    kind: entry.kind.clone(),
                    extension: entry.extension.as_ref().map(|s| s.to_string()),
                })
            })
            .collect::<Vec<_>>()
    };
    for file in &files {
        app.maybe_request_file_icon(file);
    }
}

fn details_column_width_raw(
    column: DetailsColumn,
    size_w: f32,
    modified_w: f32,
    type_w: f32,
) -> f32 {
    match column {
        DetailsColumn::Size => size_w,
        DetailsColumn::Modified => modified_w,
        DetailsColumn::Type => type_w,
    }
}

/// Fit Details columns to a narrow pane without measuring rows or mutating the
/// persisted preferences. When there is enough room, the user's exact widths are
/// preserved. When there is not, the available shrink is distributed in proportion
/// to each column's flexible surplus above its minimum; below the minimum total,
/// clipping is intentional and stable.
fn fit_details_widths(
    widths: crate::app::ColWidths,
    available_width: Option<f32>,
    zoom_scale: f32,
) -> crate::app::ColWidths {
    let zoom_scale = if zoom_scale.is_finite() && zoom_scale > 0.0 {
        zoom_scale
    } else {
        1.0
    };
    let mins = [
        ROW_NAME_MIN_W * zoom_scale,
        DETAILS_SIZE_MIN_W,
        DETAILS_MODIFIED_MIN_W,
        DETAILS_TYPE_MIN_W,
    ];
    let mut values = [
        widths.name.max(mins[0]),
        widths.size.max(mins[1]),
        widths.modified.max(mins[2]),
        widths.type_w.max(mins[3]),
    ];

    let Some(available_width) = available_width.filter(|width| width.is_finite()) else {
        return crate::app::ColWidths {
            name: values[0],
            size: values[1],
            modified: values[2],
            type_w: values[3],
        };
    };

    let total: f32 = values.iter().sum();
    if available_width >= total {
        return crate::app::ColWidths {
            name: values[0],
            size: values[1],
            modified: values[2],
            type_w: values[3],
        };
    }

    let min_total: f32 = mins.iter().sum();
    let target = available_width.max(min_total).min(total);
    let shrink = total - target;
    let flexible = [
        (values[0] - mins[0]).max(0.0),
        (values[1] - mins[1]).max(0.0),
        (values[2] - mins[2]).max(0.0),
        (values[3] - mins[3]).max(0.0),
    ];
    let flexible_total: f32 = flexible.iter().sum();
    if flexible_total > f32::EPSILON {
        for i in 0..values.len() {
            values[i] = (values[i] - shrink * flexible[i] / flexible_total).max(mins[i]);
        }
    }

    crate::app::ColWidths {
        name: values[0],
        size: values[1],
        modified: values[2],
        type_w: values[3],
    }
}

/// Explorer-style layout: columns anchored to the LEFT.
/// Name comes first (icon + text), then metadata in order, each at its own width.
/// Resizing a column shifts the ones to its right into the trailing space — the
/// handle follows the cursor and nothing "jumps the other way". Narrow panes use
/// the already-computed responsive widths; only panes below the minimum total clip.
/// `zoom_scale` scales the icon-gutter/min-width chrome constants (`ROW_NAME_X`,
/// `ROW_NAME_MIN_W`) to real pixels before combining them with `widths` — which
/// are already real, zoom-independent pixels (user drag-resize state, like a
/// Zed panel width). Everything in the returned layout is real pixels.
fn details_row_layout(
    widths: crate::app::ColWidths,
    order: &[DetailsColumn; 3],
    zoom_scale: f32,
) -> DetailsRowLayout {
    let row_name_x = ROW_NAME_X * zoom_scale;
    let row_name_min_w = ROW_NAME_MIN_W * zoom_scale;
    let name_w = widths.name.max(row_name_min_w);
    let mut x = name_w;
    let mut columns = Vec::with_capacity(3);
    for column in order {
        let w = details_column_width_raw(*column, widths.size, widths.modified, widths.type_w);
        columns.push(DetailsColumnLayout {
            column: *column,
            x,
            w,
        });
        x += w;
    }
    DetailsRowLayout {
        name_x: row_name_x,
        name_col_w: (name_w - row_name_x).max(0.0),
        name_right: name_w,
        columns,
    }
}

/// Interpolate column x-positions from `from` (pre-swap order) to `to` (current
/// order) by `t`, keeping `to`'s widths/order. Drives the reorder slide so headers
/// and rows glide to their new slots.
fn blend_layouts(from: &DetailsRowLayout, to: &DetailsRowLayout, t: f32) -> DetailsRowLayout {
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let columns = to
        .columns
        .iter()
        .map(|c| {
            let from_x = from
                .columns
                .iter()
                .find(|f| f.column == c.column)
                .map_or(c.x, |f| f.x);
            DetailsColumnLayout {
                x: lerp(from_x, c.x),
                ..*c
            }
        })
        .collect();
    DetailsRowLayout {
        name_x: to.name_x,
        name_col_w: to.name_col_w,
        name_right: to.name_right,
        columns,
    }
}

/// Canvas-based row: 2 layout nodes (outer div + canvas) vs 7 in render_row.
/// Shapes text in prepaint (cached by gpui's text system), paints via GPU
/// commands in paint — no per-column div/flex layout.
/// Falls back to render_row when renaming (input element required).
fn render_rename_row(
    entry: &FileEntry,
    entry_idx: usize,
    odd: bool,
    _selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    colors: &DetailsColors,
    rename_input: Entity<InputState>,
    flashing: bool,
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
    layout: &DetailsRowLayout,
    window: &mut Window,
    font_size: Pixels,
) -> AnyElement {
    // Same reasoning as `canvas_row`: these are chrome constants, scaled once
    // here; `col.x`/`col.w`/`layout.name_col_w` are already real-px (user column
    // widths / `details_row_layout`), so they stay plain `px(...)`.
    let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
    let row_h = DETAILS_ROW_H * zoom_scale;
    let row_icon_size = ROW_ICON_SIZE * zoom_scale;
    let row_pad_x = ROW_PAD_X * zoom_scale;
    let row_name_x = ROW_NAME_X * zoom_scale;
    let row_name_min_w = ROW_NAME_MIN_W * zoom_scale;
    let cell_pad = CELL_PAD * zoom_scale;

    let name = entry.name.clone();
    let fallback_icon = FileIcon::Builtin {
        kind: entry.kind.clone(),
        extension: entry.extension.as_ref().map(|s| s.to_string()),
    };
    let meta_cells = layout.columns.iter().map(|col| {
        let text = match col.column {
            DetailsColumn::Size => entry.display_size.clone(),
            DetailsColumn::Modified => entry.display_date.clone(),
            DetailsColumn::Type => entry.display_type.clone(),
        };
        div()
            .absolute()
            .left(px(col.x))
            .top_0()
            .bottom_0()
            .w(px(col.w))
            .flex()
            .items_center()
            .pl(px(cell_pad))
            .overflow_hidden()
            .text_color(colors.muted)
            .child(SharedString::from(text))
    });

    div()
        .id(("row", entry_idx))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w_full()
        .h(px(row_h))
        .relative()
        .overflow_hidden()
        .rounded_sm()
        .cursor_pointer()
        .when(flashing, |s| s.bg(colors.accent.opacity(0.38)))
        .when(!flashing && odd, |s| s.bg(colors.stripe_bg))
        .child(
            div()
                .absolute()
                .left(px(row_pad_x))
                .top(px((row_h - row_icon_size) * 0.5))
                .w(px(row_icon_size))
                .h(px(row_icon_size))
                .child(match icon_tex {
                    Some(tex) => img(tex)
                        .w(px(row_icon_size))
                        .h(px(row_icon_size))
                        .flex_none()
                        .into_any_element(),
                    None => fallback_icon.element(ROW_ICON_SIZE, colors.muted),
                }),
        )
        .child(
            div()
                .absolute()
                .left(px(row_name_x))
                .top_0()
                .w(px(crate::ui::views::rename::input_width(
                    &name,
                    layout.name_col_w.max(row_name_min_w),
                    font_size,
                    window,
                )))
                .child(crate::ui::views::rename::input_element(
                    &rename_input,
                    view,
                    px(row_h),
                    font_size,
                    window,
                )),
        )
        .children(meta_cells)
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DetailsHitModel;
    use gpui::{point, size};

    fn hit_model() -> DetailsHitModel {
        DetailsHitModel {
            bounds: gpui::Bounds {
                origin: point(px(10.0), px(100.0)),
                size: size(px(300.0), px(96.0)),
            },
            scroll_y: 48.0,
            row_h: 24.0,
            visible: Arc::new(vec![3, 5, 8, 13, 21, 34, 55]),
            start_idx: 2,
            end_idx: 6,
            total_count: 7,
        }
    }

    #[test]
    fn details_hit_row_from_window_point_with_scroll() {
        let model = hit_model();
        let pos = point(px(24.0), px(101.0));

        assert_eq!(model.hit_visible_pos(pos), Some(2));
        assert_eq!(model.hit_entry(pos), Some(8));
    }

    #[test]
    fn details_hit_background_returns_none() {
        let model = hit_model();

        assert_eq!(model.hit_entry(point(px(9.0), px(120.0))), None);
        assert_eq!(model.hit_entry(point(px(24.0), px(197.0))), None);
    }

    #[test]
    fn details_hit_clips_to_visible_range() {
        let mut model = hit_model();

        assert_eq!(model.hit_entry(point(px(24.0), px(125.0))), Some(13));
        model.end_idx = 5;
        assert_eq!(model.hit_entry(point(px(24.0), px(195.0))), None);
    }

    #[test]
    fn details_row_bounds_are_window_space() {
        let model = hit_model();
        let bounds = model.row_bounds(3);

        assert_eq!(bounds.origin.x, px(10.0));
        assert_eq!(bounds.origin.y, px(124.0));
        assert_eq!(bounds.size.height, px(24.0));
    }

    #[test]
    fn details_content_offset_hits_entry_for_drag_layer() {
        let model = hit_model();

        assert_eq!(
            model.hit_entry_at_content_offset(point(px(30.0), px(0.0))),
            Some(3)
        );
        assert_eq!(
            model.hit_entry_at_content_offset(point(px(30.0), px(72.0))),
            Some(13)
        );
        assert_eq!(
            model.hit_entry_at_content_offset(point(px(30.0), px(200.0))),
            None
        );
    }

    #[test]
    fn details_columns_shrink_to_fit_without_crossing_minimums() {
        let raw = crate::app::ColWidths::default();
        let fitted = fit_details_widths(raw, Some(500.0), 1.0);
        let total = fitted.name + fitted.size + fitted.modified + fitted.type_w;

        assert!((total - 500.0).abs() < 0.01);
        assert!(fitted.name >= ROW_NAME_MIN_W);
        assert!(fitted.size >= DETAILS_SIZE_MIN_W);
        assert!(fitted.modified >= DETAILS_MODIFIED_MIN_W);
        assert!(fitted.type_w >= DETAILS_TYPE_MIN_W);
    }

    #[test]
    fn details_columns_clip_only_below_the_minimum_total() {
        let fitted = fit_details_widths(crate::app::ColWidths::default(), Some(180.0), 1.0);
        let total = fitted.name + fitted.size + fitted.modified + fitted.type_w;
        let minimum_total =
            ROW_NAME_MIN_W + DETAILS_SIZE_MIN_W + DETAILS_MODIFIED_MIN_W + DETAILS_TYPE_MIN_W;

        assert!((total - minimum_total).abs() < 0.01);
    }
}
