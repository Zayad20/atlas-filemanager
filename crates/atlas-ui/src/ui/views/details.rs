//! Details view (table): `render_pane` (dispatch + Details body) + row/header cells.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::{App, ColResizeState, DetailsColumn};
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::root::{
    ctrl_scroll, pane_bounds_recorder, rect_recorder, zone_recorder, DraggedFiles,
};
use crate::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{
    canvas, div, img, point, px, size, uniform_list, AnyElement, Bounds, Context, Corners, Div,
    Entity, Font, Hsla, MouseButton, Pixels, ShapedLine, SharedString, Stateful, TextAlign,
    TextRun, Window,
};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme, ElementExt};
use std::sync::Arc;

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
        // Reset this pane's item rects before the view re-records them via
        // prepaint. The rubber-band drag reads these for hit-testing, and with
        // scoped re-renders (notify_pane) only THIS pane's render runs — so the
        // global clear that used to live in App::render is no longer guaranteed
        // to fire on every frame the selection updates. Clearing here keeps the
        // vec from accumulating stale rects across frames.
        self.row_rects.remove(&pane_id);
        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
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
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
        let (sort_col, sort_dir) = self
            .tab_data_for(pane_id)
            .map(|t| (t.sort_col.clone(), t.sort_dir.clone()))
            .unwrap_or_default();

        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let row_font = window.text_style().font();

        let col_widths = self.col_widths;
        let column_order = self.details_column_order;
        // Single source of geometry for header AND rows → pixel-perfect alignment.
        // Left-anchored fixed widths (Explorer-style); layout is independent of the
        // pane width, so resize/reorder touch only the dragged pane (notify_pane).
        // While a reorder slide is running, blend from the pre-swap order so columns
        // glide; request_animation_frame here re-renders only THIS pane (scoped).
        let to_layout = details_row_layout(col_widths, &column_order);
        let layout = Arc::new(match self.details_reorder_anim() {
            Some(anim) => {
                crate::perf::event(|| format!("details.reorder_anim pane={}", pane_id.0));
                window.request_animation_frame();
                blend_layouts(
                    &details_row_layout(col_widths, &anim.from_order),
                    &to_layout,
                    anim.progress(),
                )
            }
            None => to_layout,
        });
        let row_h = DETAILS_ROW_H;
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
            muted,
            accent,
            border,
            header_bg,
        );

        // Empty / loading / error placeholder, or the virtualized list.
        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, rust_i18n::t!("empty_folder").into_owned());
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .children(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let base_scroll = scroll.0.borrow().base_handle.clone();
            let viewport_h = f32::from(base_scroll.bounds().size.height);
            let scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            prime_details_file_icons(self, pane_id, &filtered_visible, scroll_y, viewport_h);
            let list_view = view.clone();
            let fv = filtered_visible.clone();
            let row_layout = Arc::clone(&layout);
            div()
                .size_full()
                .relative()
                .child(
                    uniform_list(
                        SharedString::from(format!("list-{}", pane_id.0)),
                        count,
                        move |range, window, cx| {
                            let start = range.start;
                            let end = range.end;
                            crate::perf::count_by(
                                "details.rows.requested",
                                end.saturating_sub(start) as u64,
                            );
                            let _perf = crate::perf::span_detail("details.rows.build", || {
                                format!("pane={} range={start}..{end}", pane_id.0)
                            });
                            let app = list_view.read(cx);
                            let Some(ts) = app.state.tab_states.get(&pane_id) else {
                                return Vec::new();
                            };
                            let record_rects = true;
                            range
                                .filter_map(|i| {
                                    let entry_idx = *fv.get(i)?;
                                    let entry = ts.entries.get(entry_idx)?;
                                    let rename_input = crate::ui::views::rename::active_input(
                                        app, pane_id, entry_idx,
                                    );
                                    let selected = ts.selection.selected.contains(&entry_idx)
                                        && !crate::ui::views::rename::suppress_row_chrome(
                                            app, pane_id, entry_idx,
                                        );
                                    let icon_tex = app
                                        .file_icon_for_entry(
                                            &entry.path,
                                            &entry.kind,
                                            entry.extension.as_deref(),
                                        )
                                        .texture();
                                    let flashing = matches!(entry.kind, FileKind::Dir)
                                        && app.drag_hover.blink_id.as_ref()
                                            == Some(&DragHoverId::Folder(
                                                pane_id,
                                                entry.path.clone(),
                                            ));
                                    let row = canvas_row(
                                        entry,
                                        entry_idx,
                                        i % 2 == 1,
                                        selected,
                                        pane_id,
                                        &list_view,
                                        accent,
                                        accent_fg,
                                        hover,
                                        muted,
                                        fg,
                                        stripe_bg,
                                        rename_input,
                                        flashing,
                                        record_rects,
                                        icon_tex,
                                        Arc::clone(&row_layout),
                                        font_size,
                                        row_font.clone(),
                                        row_h,
                                        window,
                                    );
                                    Some(row.into_any_element())
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&scroll)
                    .pr(px(16.))
                    .h_full(),
                )
                .vertical_scrollbar(&base_scroll)
                .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let zone_view = view.clone();
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
                        if this.rename_edit.is_some() {
                            this.commit_rename(cx);
                        }
                        this.begin_rubber(pane_id, pos, additive);
                        cx.notify();
                    });
                }
            })
            .on_drop::<DraggedFiles>({
                let v = rubber_view.clone();
                move |d, _window, cx| {
                    let paths = d.paths.clone();
                    v.update(cx, |this, cx| {
                        this.drop_files_on_pane(pane_id, paths, false);
                        cx.notify();
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        this.drop_files_on_pane(pane_id, paths, true);
                        cx.notify();
                    });
                }
            })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.push_focus_history();
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |ev, _window, cx| ctrl_scroll(ev, pane_id, &scroll_view, cx))
            .on_prepaint({
                let v = prepaint_view;
                move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                    crate::perf::count("prepaint.details_pane");
                    let new_w = f32::from(bounds.size.width);
                    let new_h = f32::from(bounds.size.height);
                    v.update(cx, |this, _cx| {
                        if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                            if (ts.last_pane_width - new_w).abs() > 0.5 {
                                ts.last_pane_width = new_w;
                            }
                            if (ts.last_pane_height - new_h).abs() > 0.5 {
                                ts.last_pane_height = new_h;
                            }
                        }
                    });
                }
            })
            // FASE F: register this pane's window-space rect each frame for OLE drop routing.
            .when(cfg!(windows), |el| {
                el.on_prepaint(zone_recorder(pane_id, &zone_view))
            })
            .child(crumb_strip)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .when(figma_glass, |s| s.bg(glass_fill()))
                    .rounded_bl(px(12.0))
                    .rounded_br(px(12.0))
                    .on_prepaint(pane_bounds_recorder(pane_id, &bounds_view))
                    .child(inner)
                    .child(filter_pill)
                    .child(controls_pill),
            )
            .into_any_element()
    }
}

/// Semantic category label for the Type column and filter overlay.
pub fn entry_type_label(entry: &FileEntry) -> String {
    file_category(&entry.kind, entry.extension.as_deref())
}

fn file_category(kind: &FileKind, ext: Option<&str>) -> String {
    match kind {
        FileKind::Dir => rust_i18n::t!("file_kind_folder").into_owned(),
        FileKind::Symlink => rust_i18n::t!("file_kind_symlink").into_owned(),
        FileKind::File => {
            let Some(e) = ext else {
                return rust_i18n::t!("file_kind_file").into_owned();
            };
            match e.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
                | "avif" | "heic" | "heif" | "svg" => rust_i18n::t!("file_kind_image").into_owned(),
                "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => {
                    rust_i18n::t!("file_kind_audio").into_owned()
                }
                "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "m4v" | "flv" => {
                    rust_i18n::t!("file_kind_video").into_owned()
                }
                "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "go" | "java" | "c" | "cc" | "cpp"
                | "cxx" | "h" | "hpp" | "cs" | "php" | "rb" | "swift" | "kt" | "kts" | "sh"
                | "ps1" | "bat" | "lua" | "dart" | "zig" | "nim" | "r" | "scala" | "ex" | "exs"
                | "clj" | "hs" | "ml" => rust_i18n::t!("file_kind_code").into_owned(),
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "zst" => {
                    rust_i18n::t!("file_kind_archive").into_owned()
                }
                "doc" | "docx" | "odt" | "rtf" | "pdf" | "txt" | "md" | "markdown" | "rst"
                | "tex" => rust_i18n::t!("file_kind_document").into_owned(),
                "xls" | "xlsx" | "csv" | "tsv" | "ods" | "numbers" => {
                    rust_i18n::t!("file_kind_spreadsheet").into_owned()
                }
                "ppt" | "pptx" | "odp" | "key" => {
                    rust_i18n::t!("file_kind_presentation").into_owned()
                }
                "exe" | "msi" | "app" | "dmg" | "deb" | "rpm" => {
                    rust_i18n::t!("file_kind_application").into_owned()
                }
                _ => rust_i18n::t!("file_kind_file").into_owned(),
            }
        }
    }
}

impl App {
    /// Live drag-reorder: place `dragged` at the slot the cursor is over (`local_x`
    /// is header-relative). Every position is reachable, including the last (when the
    /// cursor passes every other column's centre). On a real change we snapshot the
    /// previous order so the columns *slide* to their new slots instead of snapping.
    pub(crate) fn reorder_details_live(&mut self, dragged: DetailsColumn, local_x: f32) {
        self.col_dragging = Some(dragged);
        self.col_drag_x = local_x;
        let old = self.details_column_order;

        // Insertion index = how many of the *other* columns the cursor has passed
        // (compared against each column's centre in the current left-anchored layout).
        let mut x = self.col_widths.name.max(ROW_NAME_MIN_W);
        let mut passed = 0usize;
        for column in old {
            let w = details_column_width_raw(
                column,
                self.col_widths.size,
                self.col_widths.modified,
                self.col_widths.type_w,
            );
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
#[allow(clippy::too_many_arguments)]
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
    muted: Hsla,
    accent: Hsla,
    border: Hsla,
    header_bg: Hsla,
) -> AnyElement {
    let move_view = view.clone();
    let drop_view = view.clone();
    let mut root = div()
        .id(SharedString::from(format!("details-header-{}", pane_id.0)))
        .relative()
        .w_full()
        .h(px(DETAILS_ROW_H))
        .bg(header_bg)
        .text_xs()
        .text_color(muted)
        // Swallow mouse-down so starting a header drag/sort never begins a rubber-band
        // selection on the pane underneath.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // Live drag-reorder: as the grabbed header moves over the strip, drop it
        // into the slot under the cursor and slide the others to make room.
        .on_drag_move::<DetailsColumnDrag>(move |e, _window, cx| {
            let dragged = e.drag(cx).column;
            let local_x = f32::from(e.event.position.x - e.bounds.origin.x);
            move_view.update(cx, |this, cx| {
                this.reorder_details_live(dragged, local_x);
                this.notify_pane(pane_id, cx);
            });
        })
        .on_drop::<DetailsColumnDrag>(move |_drag, _window, cx| {
            drop_view.update(cx, |this, cx| {
                this.finish_details_reorder();
                this.notify_all_panes(cx);
            });
        })
        // Name (sortable + resizable), pinned to the shared name slot.
        .child(
            div()
                .absolute()
                .left(px(layout.name_x))
                .top_0()
                .bottom_0()
                .w(px(layout.name_col_w))
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .child(sort_cell(
                            view,
                            pane_id,
                            SortColumn::Name,
                            &rust_i18n::t!("column_name"),
                            sort_col,
                            sort_dir,
                            muted,
                            accent,
                        )),
                ),
        );
    for col in &layout.columns {
        root = root.child(details_header_cell(
            *col,
            view,
            pane_id,
            sort_col,
            sort_dir,
            has_type_filter,
            type_labels,
            dragging == Some(col.column),
            muted,
            accent,
        ));
    }
    // Resize handles as direct children of the header (NOT inside the draggable
    // cells) so grabbing one resizes instead of starting a reorder drag.
    root = root.child(col_resize_handle(
        "name",
        layout.name_right,
        layout.name_right,
        pane_id,
        view,
        border,
        accent,
    ));
    for col in &layout.columns {
        root = root.child(col_resize_handle(
            col.column.key(),
            col.w,
            col.x + col.w,
            pane_id,
            view,
            border,
            accent,
        ));
    }
    // The floating grabbed header: drawn here at a fixed y so it only tracks the
    // cursor horizontally (the gpui drag image itself is invisible).
    if let Some(col) = dragging.and_then(|d| layout.columns.iter().find(|c| c.column == d)) {
        let w = col.w;
        root = root.child(
            div()
                .absolute()
                .top_0()
                .left(px((drag_x - w * 0.5).max(0.0)))
                .h(px(DETAILS_ROW_H))
                .w(px(w))
                .flex()
                .items_center()
                .px(px(CELL_PAD))
                .rounded_md()
                .border_1()
                .border_color(accent)
                .bg(header_bg)
                .text_color(accent)
                .shadow_md()
                .child(details_column_label(col.column)),
        );
    }
    root.into_any_element()
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
#[allow(clippy::too_many_arguments)]
fn details_header_cell(
    col: DetailsColumnLayout,
    view: &Entity<App>,
    pane_id: TabId,
    sort_col: &SortColumn,
    sort_dir: &SortDir,
    has_type_filter: bool,
    type_labels: &[String],
    is_dragging: bool,
    muted: Hsla,
    accent: Hsla,
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
                muted,
                accent,
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
            muted,
            accent,
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
                .pl(px(CELL_PAD))
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
    muted: Hsla,
    accent: Hsla,
) -> Stateful<Div> {
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if is_active { accent } else { muted };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, accent))
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
#[allow(clippy::too_many_arguments)]
fn type_header_cell(
    view: &Entity<App>,
    pane_id: TabId,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
    has_filter: bool,
    type_labels: Vec<String>,
    muted: Hsla,
    accent: Hsla,
) -> Stateful<Div> {
    let col = SortColumn::Extension;
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if has_filter || is_active {
        accent
    } else {
        muted
    };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, accent))
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
            el.child(div().w(px(5.)).h(px(5.)).rounded_full().bg(accent))
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
const ROW_PAD_X: f32 = 8.0;
const ROW_ICON_W: f32 = 20.0;
const ROW_ICON_SIZE: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const ROW_NAME_X: f32 = ROW_PAD_X + ROW_ICON_W + ROW_GAP;
const ROW_NAME_MIN_W: f32 = 88.0;
const ROW_TEXT_Y_NUDGE: f32 = -1.0;
/// Left inset of metadata text from its column's left edge (separator), so text
/// sits *inside* the column instead of hugging the divider. Applied identically
/// to the header label and the row data → they stay aligned.
const CELL_PAD: f32 = 11.0;

#[derive(Clone, Copy)]
struct DetailsColumnLayout {
    column: DetailsColumn,
    /// Left edge, row/header-relative (origin = pane left).
    x: f32,
    /// Column width.
    w: f32,
}

#[derive(Clone)]
struct DetailsRowLayout {
    name_x: f32,
    /// Width of the Name text area (Name column width minus the icon gutter).
    name_col_w: f32,
    /// Right edge of the Name column = left edge of the first metadata column.
    name_right: f32,
    /// Metadata columns in visual order (left→right, after Name).
    columns: Vec<DetailsColumnLayout>,
}

struct DetailsRowCanvasState {
    name_s: ShapedLine,
    ellipsis_s: Option<ShapedLine>,
    columns: Vec<DetailsPaintColumn>,
    layout: DetailsRowLayout,
}

struct DetailsPaintColumn {
    column: DetailsColumn,
    line: ShapedLine,
}

fn prime_details_file_icons(
    app: &mut App,
    pane_id: TabId,
    visible: &[usize],
    scroll_y: f32,
    viewport_h: f32,
) {
    let files = {
        let Some(ts) = app.state.tab_states.get(&pane_id) else {
            return;
        };
        let start = (scroll_y / DETAILS_ROW_H).floor().max(0.0) as usize;
        let end = ((scroll_y + viewport_h) / DETAILS_ROW_H).ceil() as usize + 1;
        let end = end.min(visible.len());
        (start..end)
            .filter_map(|i| {
                let entry = ts.entries.get(*visible.get(i)?)?;
                Some(crate::ui::file_icons::FileIconRef {
                    path: entry.path.clone(),
                    kind: entry.kind.clone(),
                    extension: entry.extension.clone(),
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

/// Explorer-style layout: fixed-width columns anchored to the LEFT.
/// Name comes first (icon + text), then metadata in order, each at its own width.
/// Resizing a column shifts the ones to its right into the trailing space — the
/// handle follows the cursor and nothing "jumps the other way". No flex column,
/// no collapse: when columns overrun the pane they simply clip on the right.
fn details_row_layout(
    widths: crate::app::ColWidths,
    order: &[DetailsColumn; 3],
) -> DetailsRowLayout {
    let name_w = widths.name.max(ROW_NAME_MIN_W);
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
        name_x: ROW_NAME_X,
        name_col_w: (name_w - ROW_NAME_X).max(0.0),
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
#[allow(clippy::too_many_arguments)]
fn canvas_row(
    entry: &FileEntry,
    entry_idx: usize,
    odd: bool,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    muted: Hsla,
    fg: Hsla,
    stripe_bg: Hsla,
    rename_input: Option<Entity<InputState>>,
    flashing: bool,
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
    layout: Arc<DetailsRowLayout>,
    font_size: Pixels,
    font: Font,
    row_h: f32,
    window: &mut Window,
) -> AnyElement {
    if let Some(input) = rename_input {
        return render_rename_row(
            entry,
            entry_idx,
            odd,
            selected,
            pane_id,
            view,
            accent,
            accent_fg,
            hover,
            muted,
            stripe_bg,
            input,
            flashing,
            record_rects,
            icon_tex,
            &layout,
            window,
            font_size,
        );
    }

    let name: SharedString = entry.name.clone().into();
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let file_size = entry.size;
    let modified = entry.modified;
    let kind = entry.kind.clone();
    let extension = entry.extension.clone();

    let name_color = if selected { accent_fg } else { fg };
    let meta_color = if selected { accent_fg } else { muted };

    // Separate clones per closure (prepaint vs paint capture different sets).
    let name_p = name.clone();
    let font_p = font.clone();
    let icon_tex_paint = icon_tex.clone();

    let row_canvas = canvas(
        move |bounds, window, _cx| {
            crate::perf::count("details.row.shape_count");
            let _perf = crate::perf::span("details.row.shape");
            let ts = window.text_system();
            let mk = |s: &SharedString, color: Hsla| TextRun {
                len: s.len(),
                font: font_p.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            // TextRuns must be created before moving the SharedStrings.
            let name_run = mk(&name_p, name_color);

            let mut name_s = ts.shape_line(name_p, font_size, &[name_run], None);

            let layout = (*layout).clone();
            let row_w = f32::from(bounds.size.width);
            let columns: Vec<DetailsPaintColumn> = layout
                .columns
                .iter()
                .filter(|col| col.x < row_w && col.w > 1.0)
                .map(|col| {
                    let text: SharedString = match col.column {
                        DetailsColumn::Size => {
                            if matches!(kind, FileKind::Dir) {
                                SharedString::default()
                            } else {
                                file_size.map(format_size).unwrap_or_default().into()
                            }
                        }
                        DetailsColumn::Modified => {
                            modified.map(format_date).unwrap_or_default().into()
                        }
                        DetailsColumn::Type => file_category(&kind, extension.as_deref())
                            .to_string()
                            .into(),
                    };
                    let run = mk(&text, meta_color);
                    DetailsPaintColumn {
                        column: col.column,
                        line: ts.shape_line(text, font_size, &[run], None),
                    }
                })
                .collect();
            crate::perf::count_by("details.meta_cols.shaped", columns.len() as u64);
            let name_col_w = px(layout.name_col_w.min((row_w - ROW_NAME_X).max(0.0)));
            let ellipsis_s = if name_s.width() > name_col_w {
                let ellipsis: SharedString = "…".into();
                let e_run = mk(&ellipsis, name_color);
                let es = ts.shape_line(ellipsis, font_size, &[e_run], None);
                let cutoff = (name_col_w - es.width()).max(px(0.));
                let split_idx = name_s.index_for_x(cutoff).unwrap_or(0);
                let (prefix, _) = name_s.split_at(split_idx);
                name_s = prefix;
                Some(es)
            } else {
                None
            };

            DetailsRowCanvasState {
                name_s,
                ellipsis_s,
                columns,
                layout,
            }
        },
        move |bounds, state, window, cx| {
            crate::perf::count("details.row.paint_count");
            let _perf = crate::perf::span("details.row.paint");
            // `ShapedLine::paint` already centers the glyphs vertically inside the
            // `line_height` (ROW_H) box we pass below — so origin.y is just the row top
            // (plus a tiny optical nudge). Adding our own (ROW_H - font_size)/2 here too
            // double-counted the centering and pushed text below the icon.
            let text_y = bounds.origin.y + px(ROW_TEXT_Y_NUDGE);

            // Icon texture
            if let Some(tex) = &icon_tex_paint {
                let _ = window.paint_image(
                    Bounds {
                        origin: point(
                            bounds.origin.x + px(8.),
                            bounds.origin.y + px((row_h - ROW_ICON_SIZE) * 0.5),
                        ),
                        size: size(px(ROW_ICON_SIZE), px(ROW_ICON_SIZE)),
                    },
                    Corners::default(),
                    tex.clone(),
                    0,
                    false,
                );
            }

            // Name column — clipped; truncated with "…" when overflowing
            let name_x = bounds.origin.x + px(ROW_NAME_X);
            window.paint_layer(
                Bounds {
                    origin: point(name_x, bounds.origin.y),
                    size: size(px(state.layout.name_col_w), bounds.size.height),
                },
                |window| {
                    let _ = state.name_s.paint(
                        point(name_x, text_y),
                        px(row_h),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    if let Some(ref es) = state.ellipsis_s {
                        let _ = es.paint(
                            point(name_x + state.name_s.width(), text_y),
                            px(row_h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                },
            );

            for paint_col in &state.columns {
                let Some(layout_col) = state
                    .layout
                    .columns
                    .iter()
                    .find(|col| col.column == paint_col.column)
                else {
                    continue;
                };
                let col_x = bounds.origin.x + px(layout_col.x);
                let col_w = layout_col.w;
                // Left-align every column so the data lines up exactly under its
                // (left-aligned) header label, with CELL_PAD breathing room from the
                // separator. Clip to the column so text never bleeds into its neighbour.
                let text_x = col_x + px(CELL_PAD.min(col_w));
                window.paint_layer(
                    Bounds {
                        origin: point(col_x, bounds.origin.y),
                        size: size(px(col_w), bounds.size.height),
                    },
                    |window| {
                        let _ = paint_col.line.paint(
                            point(text_x, text_y),
                            px(row_h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .h(px(row_h));

    let v = view.clone();
    let entry_path = entry.path.clone();
    let fallback_icon = FileIcon::Builtin {
        kind: entry.kind.clone(),
        extension: entry.extension.clone(),
    };
    let has_tex = icon_tex.is_some();
    let row_id = SharedString::from(format!("row-{}-{}", pane_id.0, entry_idx));

    let outer = div()
        .id(row_id)
        .w_full()
        .h(px(row_h))
        .relative()
        .overflow_hidden()
        .rounded_sm()
        .when(selected, |s| s.bg(accent))
        .when(flashing && !selected, |s| s.bg(accent.opacity(0.38)))
        .when(!selected && odd, |s| s.bg(stripe_bg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .when(record_rects, |s| {
            s.on_prepaint(crate::ui::root::rect_recorder(pane_id, entry_idx, view))
        })
        .child(row_canvas)
        // SVG icon fallback when no pre-rasterised texture
        .when(!has_tex, |s| {
            s.child(
                div()
                    .absolute()
                    .left(px(ROW_PAD_X))
                    .top(px((row_h - ROW_ICON_SIZE) * 0.5))
                    .w(px(ROW_ICON_SIZE))
                    .h(px(ROW_ICON_SIZE))
                    .child(fallback_icon.clone().element(ROW_ICON_SIZE, muted)),
            )
        });

    outer
        .on_click({
            let v = v.clone();
            move |ev, _window, cx| {
                let m = ev.modifiers();
                let (shift, ctrl) = (m.shift, m.control || m.platform);
                v.update(cx, |this, cx| {
                    if this.rename_edit.is_some() {
                        this.commit_rename(cx);
                    }
                    if ev.click_count() >= 2 {
                        this.process_actions(vec![Action::Activate {
                            tile: pane_id,
                            idx: entry_idx,
                        }]);
                    } else {
                        this.process_actions(vec![Action::Select {
                            tile: pane_id,
                            idx: entry_idx,
                            shift,
                            ctrl,
                        }]);
                    }
                    cx.notify();
                });
            }
        })
        .on_mouse_down(MouseButton::Right, {
            let v = v.clone();
            move |ev, window, cx| {
                let pos = ev.position;
                v.update(cx, |this, cx| {
                    let paths = this.select_context_menu_paths(pane_id, entry_idx);
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .when(!selected, |s| {
            let rv = v.clone();
            s.on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                let (pos, additive) = (ev.position, ev.modifiers.control || ev.modifiers.platform);
                rv.update(cx, |this, cx| {
                    this.begin_rubber(pane_id, pos, additive);
                    cx.notify();
                });
                cx.stop_propagation();
            })
        })
        .when(selected, |s| {
            s.on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                cx.stop_propagation();
            })
            .on_drag(
                crate::ui::drag::file_drag_value(vec![entry_path.clone()]),
                crate::ui::drag::file_drag_handler_for_item(
                    v.clone(),
                    pane_id,
                    entry_idx,
                    entry_path.clone(),
                ),
            )
        })
        .when(is_dir, |s| {
            let dest = entry_path.clone();
            let hover_id = DragHoverId::Folder(pane_id, dest.clone());
            let dest_internal = dest.clone();
            let dest_external = dest.clone();
            let mv = v.clone();
            let cpv = v.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Folder,
                    DragHoverCommand::NavigatePathIn(pane_id, dest.clone()),
                    DragHoverDropCommand::TransferTo(dest.clone()),
                    view,
                ))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let paths = d.paths.clone();
                    mv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_internal.clone()),
                            paths,
                            true,
                        );
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    cpv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_external.clone()),
                            paths,
                            false,
                        );
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_rename_row(
    entry: &FileEntry,
    entry_idx: usize,
    odd: bool,
    _selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    _accent_fg: Hsla,
    _hover: Hsla,
    muted: Hsla,
    stripe_bg: Hsla,
    rename_input: Entity<InputState>,
    flashing: bool,
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
    layout: &DetailsRowLayout,
    window: &mut Window,
    font_size: Pixels,
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let name = entry.name.clone();
    let fallback_icon = FileIcon::Builtin {
        kind: entry.kind.clone(),
        extension: entry.extension.clone(),
    };
    let meta_cells = layout.columns.iter().map(|col| {
        let text = match col.column {
            DetailsColumn::Size => {
                if is_dir {
                    String::new()
                } else {
                    entry.size.map(format_size).unwrap_or_default()
                }
            }
            DetailsColumn::Modified => entry.modified.map(format_date).unwrap_or_default(),
            DetailsColumn::Type => entry_type_label(entry),
        };
        div()
            .absolute()
            .left(px(col.x))
            .top_0()
            .bottom_0()
            .w(px(col.w))
            .flex()
            .items_center()
            .pl(px(CELL_PAD))
            .overflow_hidden()
            .text_color(muted)
            .child(text)
    });

    div()
        .id(SharedString::from(format!(
            "row-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w_full()
        .h(px(DETAILS_ROW_H))
        .relative()
        .overflow_hidden()
        .rounded_sm()
        .cursor_pointer()
        .when(flashing, |s| s.bg(accent.opacity(0.38)))
        .when(!flashing && odd, |s| s.bg(stripe_bg))
        .child(
            div()
                .absolute()
                .left(px(ROW_PAD_X))
                .top(px((DETAILS_ROW_H - ROW_ICON_SIZE) * 0.5))
                .w(px(ROW_ICON_SIZE))
                .h(px(ROW_ICON_SIZE))
                .child(match icon_tex {
                    Some(tex) => img(tex)
                        .w(px(ROW_ICON_SIZE))
                        .h(px(ROW_ICON_SIZE))
                        .flex_none()
                        .into_any_element(),
                    None => fallback_icon.element(ROW_ICON_SIZE, muted),
                }),
        )
        .child(
            div()
                .absolute()
                .left(px(ROW_NAME_X))
                .top_0()
                .w(px(crate::ui::views::rename::input_width(
                    &name,
                    layout.name_col_w.max(ROW_NAME_MIN_W),
                    font_size,
                    window,
                )))
                .child(crate::ui::views::rename::input_element(
                    &rename_input,
                    view,
                    DETAILS_ROW_H,
                    font_size,
                )),
        )
        .children(meta_cells)
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .into_any_element()
}
