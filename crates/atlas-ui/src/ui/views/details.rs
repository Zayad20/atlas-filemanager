//! Details view (table): `render_pane` (dispatch + Details body) + row/header cells.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::{App, ColResizeState, DetailsColumn};
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use crate::ui::root::{ctrl_scroll, rect_recorder, zone_recorder, DraggedFiles};
use crate::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{
    canvas, div, img, point, px, size, uniform_list, AnyElement, Bounds, Context, Corners, Div,
    Entity, Font, Hsla, MouseButton, Pixels, ShapedLine, SharedString, Stateful, TextAlign,
    TextRun, Window,
};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::sync::Arc;

#[derive(Clone)]
struct DetailsColumnDrag {
    column: DetailsColumn,
}

impl Render for DetailsColumnDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(cx.theme().accent)
            .text_color(cx.theme().accent_foreground)
            .child(details_column_label(self.column))
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
        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        match view_mode {
            ViewMode::Grid => return self.render_grid(pane_id, window, cx),
            ViewMode::List => return self.render_list_compact(pane_id, window, cx),
            ViewMode::Miller => return self.render_miller(pane_id, window, cx),
            ViewMode::Details => {}
        }
        let view = cx.entity();
        let (fg, accent, accent_fg, hover, muted, header_bg, stripe_bg, border) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                t.secondary,
                t.list_even,
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
        let pane_width = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.last_pane_width)
            .unwrap_or_default();
        let header_layout = details_row_layout(
            pane_width.max(ROW_NAME_X + ROW_NAME_MIN_W),
            &column_order,
            col_widths.size,
            col_widths.modified,
            col_widths.type_w,
        );
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

        let mut header = h_flex()
            .w_full()
            .h(px(ROW_H))
            .px_2()
            .pr(px(16.)) // match uniform_list's pr(16) so column separators align with row canvas
            .gap_2()
            .bg(header_bg)
            .text_xs()
            .text_color(muted)
            // Icon placeholder.
            .child(div().w(px(20.)))
            // Name: flex_1, sortable.
            .child(
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Name,
                    "Nombre",
                    &sort_col,
                    &sort_dir,
                    muted,
                    accent,
                )
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden(),
            );
        for column in header_layout.columns.iter().map(|col| col.column) {
            header = header.child(details_header_column(
                column,
                col_widths,
                &view,
                pane_id,
                &sort_col,
                &sort_dir,
                has_type_filter,
                type_labels.clone(),
                muted,
                accent,
                border,
            ));
        }
        let header = header.child(div().w(px(40.)));

        // Empty / loading / error placeholder, or the virtualized list.
        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, "Carpeta vacía");
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
            let list_view = view.clone();
            let fv = filtered_visible.clone();
            div()
                .size_full()
                .relative()
                .child(
                    uniform_list(
                        SharedString::from(format!("list-{}", pane_id.0)),
                        count,
                        move |range, _window, cx| {
                            let app = list_view.read(cx);
                            let Some(ts) = app.state.tab_states.get(&pane_id) else {
                                return Vec::new();
                            };
                            let record_rects =
                                app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
                            let cw = app.col_widths;
                            range
                                .filter_map(|i| {
                                    let entry_idx = *fv.get(i)?;
                                    let entry = ts.entries.get(entry_idx)?;
                                    let selected = ts.selection.selected.contains(&entry_idx);
                                    let rename_input = app
                                        .rename_edit
                                        .as_ref()
                                        .filter(|e| e.pane == pane_id && e.idx == entry_idx)
                                        .map(|e| e.input.clone());
                                    let icon_tex = app.icon_tex_for_entry(
                                        &entry.kind,
                                        entry.extension.as_deref(),
                                    );
                                    Some(canvas_row(
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
                                        record_rects,
                                        icon_tex,
                                        cw.size,
                                        cw.modified,
                                        cw.type_w,
                                        app.details_column_order,
                                        font_size,
                                        row_font.clone(),
                                    ))
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
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            // The pane content is transparent: the hosting TabPanel (fork) is the
            // rounded glass card, with the tab selector inside it.
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
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
                        if let Some(dest) = this
                            .tab_data_for(pane_id)
                            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                        {
                            this.process_actions(vec![Action::Move { paths, dest }]);
                            cx.notify();
                        }
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        if let Some(dest) = this
                            .tab_data_for(pane_id)
                            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                        {
                            this.process_actions(vec![Action::CopyTo { paths, dest }]);
                            cx.notify();
                        }
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
            .on_prepaint(move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                let new_w = f32::from(bounds.size.width);
                prepaint_view.update(cx, |this, cx| {
                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                        if (ts.last_pane_width - new_w).abs() > 0.5 {
                            ts.last_pane_width = new_w;
                            cx.notify();
                        }
                    }
                });
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
                    .child(inner)
                    .child(filter_pill)
                    .child(controls_pill),
            )
            .into_any_element()
    }
}

/// Semantic category label for the Type column and filter overlay.
fn entry_type_label(entry: &FileEntry) -> String {
    file_category(&entry.kind, entry.extension.as_deref()).to_string()
}

fn file_category(kind: &FileKind, ext: Option<&str>) -> &'static str {
    match kind {
        FileKind::Dir => "Carpeta",
        FileKind::Symlink => "Enlace",
        FileKind::File => {
            let Some(e) = ext else { return "Archivo" };
            match e.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
                | "avif" | "heic" | "heif" | "svg" => "Imagen",
                "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => "Audio",
                "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "m4v" | "flv" => "Vídeo",
                "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "go" | "java" | "c" | "cc" | "cpp"
                | "cxx" | "h" | "hpp" | "cs" | "php" | "rb" | "swift" | "kt" | "kts" | "sh"
                | "ps1" | "bat" | "lua" | "dart" | "zig" | "nim" | "r" | "scala" | "ex" | "exs"
                | "clj" | "hs" | "ml" => "Código",
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "zst" => "Comprimido",
                "doc" | "docx" | "odt" | "rtf" | "pdf" | "txt" | "md" | "markdown" | "rst"
                | "tex" => "Documento",
                "xls" | "xlsx" | "csv" | "tsv" | "ods" | "numbers" => "Hoja",
                "ppt" | "pptx" | "odp" | "key" => "Presentación",
                "exe" | "msi" | "app" | "dmg" | "deb" | "rpm" => "Aplicación",
                _ => "Archivo",
            }
        }
    }
}

/// Column wrapper: fixed width + visible 1px right-edge separator + resize grip.
/// The grip also acts as a drag/drop target for metadata-column reordering.
fn col_wrapper(
    width: f32,
    content: AnyElement,
    col: &'static str,
    pane_id: TabId,
    cur_width: f32,
    view: &Entity<App>,
    border: Hsla,
    column: DetailsColumn,
) -> AnyElement {
    let drop_view = view.clone();
    div()
        .id(SharedString::from(format!(
            "details-col-{}-{}",
            pane_id.0,
            column.key()
        )))
        .relative()
        .w(px(width))
        .flex_none()
        .overflow_hidden()
        .border_r_1()
        .border_color(border.opacity(0.3))
        .child(content)
        .on_drag(
            DetailsColumnDrag { column },
            move |drag, _pos, _window, cx| {
                cx.new(|_| DetailsColumnDrag {
                    column: drag.column,
                })
            },
        )
        .drag_over::<DetailsColumnDrag>(|style, _, _, _| style.bg(gpui::blue().opacity(0.14)))
        .on_drop::<DetailsColumnDrag>(move |drag, _window, cx| {
            let target = column;
            let source = drag.column;
            if source == target {
                return;
            }
            drop_view.update(cx, |this, cx| {
                let mut order: Vec<DetailsColumn> = this
                    .details_column_order
                    .iter()
                    .copied()
                    .filter(|col| *col != source)
                    .collect();
                let target_ix = order
                    .iter()
                    .position(|col| *col == target)
                    .unwrap_or(order.len());
                order.insert(target_ix, source);
                if let Ok(order) = <[DetailsColumn; 3]>::try_from(order) {
                    this.details_column_order = order;
                    cx.notify();
                }
            });
        })
        .child(col_resize_handle(col, cur_width, pane_id, view))
        .into_any_element()
}

/// 6px absolute grip centered on the right edge (±3px). Invisible until hover.
/// Mouse-down anchors start_x + start_width; rubber_layer applies dx on move.
fn col_resize_handle(
    col: &'static str,
    cur_width: f32,
    pane_id: TabId,
    view: &Entity<App>,
) -> Stateful<Div> {
    let v = view.clone();
    div()
        .id(SharedString::from(format!("rh-{col}-{}", pane_id.0)))
        .absolute()
        .right(px(-3.))
        .top_0()
        .bottom_0()
        .w(px(6.))
        .cursor_col_resize()
        .hover(|s| s.bg(gpui::blue().opacity(0.5)))
        .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
            let start_x = f32::from(ev.position.x);
            v.update(cx, |this, _cx| {
                this.col_resize = Some(ColResizeState {
                    col,
                    start_x,
                    start_width: cur_width,
                });
            });
            cx.stop_propagation();
        })
}

fn details_column_width(widths: crate::app::ColWidths, column: DetailsColumn) -> f32 {
    match column {
        DetailsColumn::Size => widths.size,
        DetailsColumn::Modified => widths.modified,
        DetailsColumn::Type => widths.type_w,
    }
}

fn details_column_label(column: DetailsColumn) -> &'static str {
    match column {
        DetailsColumn::Size => "Tamaño",
        DetailsColumn::Modified => "Modificado",
        DetailsColumn::Type => "Tipo",
    }
}

#[allow(clippy::too_many_arguments)]
fn details_header_column(
    column: DetailsColumn,
    widths: crate::app::ColWidths,
    view: &Entity<App>,
    pane_id: TabId,
    sort_col: &SortColumn,
    sort_dir: &SortDir,
    has_type_filter: bool,
    type_labels: Vec<String>,
    muted: Hsla,
    accent: Hsla,
    border: Hsla,
) -> AnyElement {
    let width = details_column_width(widths, column);
    let content = match column {
        DetailsColumn::Size => sort_cell(
            view,
            pane_id,
            SortColumn::Size,
            details_column_label(column),
            sort_col,
            sort_dir,
            muted,
            accent,
        )
        .w_full()
        .overflow_hidden()
        .into_any_element(),
        DetailsColumn::Modified => sort_cell(
            view,
            pane_id,
            SortColumn::Modified,
            details_column_label(column),
            sort_col,
            sort_dir,
            muted,
            accent,
        )
        .w_full()
        .overflow_hidden()
        .into_any_element(),
        DetailsColumn::Type => type_header_cell(
            view,
            pane_id,
            sort_col,
            sort_dir,
            has_type_filter,
            type_labels,
            muted,
            accent,
        )
        .w_full()
        .overflow_hidden()
        .into_any_element(),
    };
    col_wrapper(
        width,
        content,
        column.key(),
        pane_id,
        width,
        view,
        border,
        column,
    )
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
        .child("Tipo")
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
const ROW_H: f32 = 24.0;
const ROW_PAD_X: f32 = 8.0;
const ROW_ICON_W: f32 = 20.0;
const ROW_ICON_SIZE: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const ROW_TRAILING_W: f32 = 40.0;
const ROW_NAME_X: f32 = ROW_PAD_X + ROW_ICON_W + ROW_GAP;
const ROW_NAME_MIN_W: f32 = 88.0;

#[derive(Clone, Copy)]
struct DetailsColumnLayout {
    column: DetailsColumn,
    x: f32,
}

#[derive(Clone)]
struct DetailsRowLayout {
    name_col_w: f32,
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

fn details_row_layout(
    width: f32,
    order: &[DetailsColumn; 3],
    size_w: f32,
    modified_w: f32,
    type_w: f32,
) -> DetailsRowLayout {
    let content_right = (width - ROW_PAD_X - ROW_TRAILING_W).max(ROW_NAME_X);
    let mut visible = order.to_vec();

    for collapse in [
        None,
        Some(DetailsColumn::Type),
        Some(DetailsColumn::Modified),
        Some(DetailsColumn::Size),
    ] {
        if let Some(collapse) = collapse {
            visible.retain(|column| *column != collapse);
        }
        let mut x = content_right;
        let mut columns = Vec::with_capacity(visible.len());
        for column in visible.iter().rev().copied() {
            let col_w = details_column_width_raw(column, size_w, modified_w, type_w);
            x -= col_w;
            columns.push(DetailsColumnLayout { column, x });
            x -= ROW_GAP;
        }
        columns.reverse();
        let name_col_w = (x - ROW_NAME_X).max(0.0);
        if name_col_w >= ROW_NAME_MIN_W || visible.is_empty() {
            return DetailsRowLayout {
                name_col_w,
                columns,
            };
        }
    }

    DetailsRowLayout {
        name_col_w: (content_right - ROW_NAME_X).max(0.0),
        columns: Vec::new(),
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
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
    size_w: f32,
    modified_w: f32,
    type_w: f32,
    column_order: [DetailsColumn; 3],
    font_size: Pixels,
    font: Font,
) -> AnyElement {
    if rename_input.is_some() {
        return render_row(
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
            rename_input,
            record_rects,
            icon_tex,
            size_w,
            modified_w,
            type_w,
            column_order,
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

            let layout = details_row_layout(
                f32::from(bounds.size.width),
                &column_order,
                size_w,
                modified_w,
                type_w,
            );
            let columns = layout
                .columns
                .iter()
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
            let name_col_w = px(layout.name_col_w);
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
            let text_y = bounds.origin.y + px((ROW_H - f32::from(font_size)) * 0.5 + 1.0);

            // Icon texture
            if let Some(tex) = &icon_tex_paint {
                let _ = window.paint_image(
                    Bounds {
                        origin: point(
                            bounds.origin.x + px(8.),
                            bounds.origin.y + px((ROW_H - ROW_ICON_SIZE) * 0.5),
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
                        px(ROW_H),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    if let Some(ref es) = state.ellipsis_s {
                        let _ = es.paint(
                            point(name_x + state.name_s.width(), text_y),
                            px(ROW_H),
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
                let col_w = details_column_width_raw(paint_col.column, size_w, modified_w, type_w);
                let align = if paint_col.column == DetailsColumn::Size {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                };
                let align_width = if paint_col.column == DetailsColumn::Size {
                    Some(px(col_w))
                } else {
                    None
                };
                window.paint_layer(
                    Bounds {
                        origin: point(col_x, bounds.origin.y),
                        size: size(px(col_w), bounds.size.height),
                    },
                    |window| {
                        let _ = paint_col.line.paint(
                            point(col_x, text_y),
                            px(ROW_H),
                            align,
                            align_width,
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
    .h(px(ROW_H));

    let v = view.clone();
    let entry_path = entry.path.clone();
    let icon_kind = entry.kind.clone();
    let icon_ext = entry.extension.clone();
    let has_tex = icon_tex.is_some();
    let row_id = SharedString::from(format!("row-{}-{}", pane_id.0, entry_idx));

    let outer = div()
        .id(row_id)
        .w_full()
        .h(px(ROW_H))
        .relative()
        .overflow_hidden()
        .rounded_sm()
        .when(selected, |s| s.bg(accent))
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
                    .top(px((ROW_H - ROW_ICON_SIZE) * 0.5))
                    .w(px(ROW_ICON_SIZE))
                    .h(px(ROW_ICON_SIZE))
                    .child(file_type_icon(
                        &icon_kind,
                        icon_ext.as_deref(),
                        ROW_ICON_SIZE,
                    )),
            )
        });

    outer
        .on_click({
            let v = v.clone();
            move |ev, _window, cx| {
                let m = ev.modifiers();
                let (shift, ctrl) = (m.shift, m.control || m.platform);
                v.update(cx, |this, cx| {
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
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
                    let paths = this.selected_paths();
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
                DraggedFiles {
                    paths: vec![entry_path.clone()],
                },
                crate::ui::drag::file_drag_handler(v.clone(), entry_path.clone()),
            )
        })
        .when(is_dir, |s| {
            let dest_move = entry_path.clone();
            let dest_copy = entry_path.clone();
            let mv = v.clone();
            let cpv = v.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let (paths, dest) = (d.paths.clone(), dest_move.clone());
                    mv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::Move { paths, dest }]);
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let (paths, dest) = (paths.paths().to_vec(), dest_copy.clone());
                    cpv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::CopyTo { paths, dest }]);
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}

/// One file row in the Details list. Selection/activation route through `Action`s,
/// reading click modifiers (shift = range, ctrl/cmd = toggle) and count (≥2 = open).
#[allow(clippy::too_many_arguments)]
fn render_row(
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
    stripe_bg: Hsla,
    rename_input: Option<Entity<InputState>>,
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
    size_w: f32,
    modified_w: f32,
    type_w: f32,
    _column_order: [DetailsColumn; 3],
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let name = entry.name.clone();
    let size = if is_dir {
        String::new()
    } else {
        entry.size.map(format_size).unwrap_or_default()
    };
    let date = entry.modified.map(format_date).unwrap_or_default();
    let type_label = entry_type_label(entry);
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!(
            "row-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w_full()
        .px_2()
        .py(px(3.))
        .gap_2()
        .items_center()
        .rounded_sm()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected && odd, |s| s.bg(stripe_bg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        // Icon
        .child(
            div().w(px(20.)).child(match icon_tex {
                Some(tex) => img(tex)
                    .w(px(16.))
                    .h(px(16.))
                    .flex_none()
                    .into_any_element(),
                None => file_type_icon(&entry.kind, entry.extension.as_deref(), 16.0),
            }),
        )
        // Name
        .child(match &rename_input {
            Some(input) => div()
                .flex_1()
                .min_w(px(0.))
                .child(crate::ui::text_input::bordered(input))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(name)
                .into_any_element(),
        })
        // Size
        .child(
            div()
                .w(px(size_w))
                .text_right()
                .when(!selected, |s| s.text_color(muted))
                .child(size),
        )
        // Modified
        .child(
            div()
                .w(px(modified_w))
                .when(!selected, |s| s.text_color(muted))
                .child(date),
        )
        // Type
        .child(
            div()
                .w(px(type_w))
                .when(!selected, |s| s.text_color(muted))
                .child(type_label),
        )
        // Trailing "dead zone": starts a rubber-band drag from within a row (the row
        // swallows left mouse-down for selection, so this column re-initiates it).
        .child({
            let rv = view.clone();
            div()
                .w(px(40.))
                .h_full()
                .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    rv.update(cx, |this, cx| {
                        this.begin_rubber(pane_id, pos, additive);
                        cx.notify();
                    });
                    cx.stop_propagation();
                })
        })
        .on_click(move |ev, _window, cx| {
            let m = ev.modifiers();
            let shift = m.shift;
            let ctrl = m.control || m.platform;
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if activate {
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
        })
        .on_mouse_down(MouseButton::Right, {
            let rv = view.clone();
            move |ev, window, cx| {
                let pos = ev.position;
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
                    // Right-click opens the Atlas–style menu on the (now
                    // selected) item; native shell items stream in afterwards.
                    let paths = this.selected_paths();
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .when(!selected, |s| {
            let rv = view.clone();
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
                DraggedFiles {
                    paths: vec![entry.path.clone()],
                },
                crate::ui::drag::file_drag_handler(view.clone(), entry.path.clone()),
            )
        })
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            let dest_move = entry.path.clone();
            let dest_copy = entry.path.clone();
            let mv = view.clone();
            let cpv = view.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let (paths, dest) = (d.paths.clone(), dest_move.clone());
                    mv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::Move { paths, dest }]);
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let (paths, dest) = (paths.paths().to_vec(), dest_copy.clone());
                    cpv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::CopyTo { paths, dest }]);
                        cx.notify();
                    });
                })
        })
        // While renaming this row, Enter commits and Esc cancels (capture phase so the
        // focused input doesn't swallow them first).
        .when_some(rename_input, |row, _input| {
            let cv = view.clone();
            row.capture_key_down(move |ev, _window, app| match ev.keystroke.key.as_str() {
                "enter" => {
                    cv.update(app, App::commit_rename);
                    app.stop_propagation();
                }
                "escape" => {
                    cv.update(app, |this, _cx| this.cancel_rename());
                    app.stop_propagation();
                }
                _ => {}
            })
        })
        .into_any_element()
}
