//! Sectioned, virtualized Details/List display. Only viewport rows become elements.
use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag::FileDragSource as _;
use crate::ui::root::{pane_bounds_recorder, rect_recorder, zone_recorder};
use atlas_model::{
    file_entry::FileKind,
    pane::TabId,
    tab::{GroupBy, GroupRow},
};
use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, Context, MouseButton, SharedString, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};

impl App {
    pub(crate) fn render_grouped(
        &mut self,
        pane: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        self.details_hit.remove(&pane);
        self.list_hit.remove(&pane);
        self.grid_hit.remove(&pane);
        let (mode, collapsed) = self
            .tab_data_for(pane)
            .map(|tab| (tab.group_by, tab.collapsed_groups.clone()))
            .unwrap_or_default();
        let rows = self
            .state
            .tab_states
            .get_mut(&pane)
            .map(|tab| {
                tab.set_grouping(mode, collapsed);
                tab.grid_columns = 1;
                tab.grouped_rows_shared()
            })
            .unwrap_or_default();
        let (fg, muted, border, accent, accent_fg, hover, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.list_head,
            )
        };
        let crumbs = self.render_crumb_strip(pane, &view, fg, muted, border, header_bg, cx);
        let scroll = self.scroll_handle_for(pane);
        let no_rows = rows.is_empty();
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let row_h = f32::from(window.line_height()).max(18.0 * zoom) + 10.0 * zoom;
        let rows_view = view.clone();
        let list = uniform_list(
            ("grouped-files", pane.0),
            rows.len(),
            move |range, window, cx| {
                rows_view.update(cx, |app, cx| {
                    range
                        .filter_map(|row_index| {
                            let row = rows.get(row_index)?;
                            match row {
                                GroupRow::Header {
                                    id,
                                    label,
                                    count,
                                    collapsed,
                                } => {
                                    let id = id.clone();
                                    let click_view = rows_view.clone();
                                    Some(
                                        h_flex()
                                            .id(("group-section", row_index))
                                            .h(px(row_h))
                                            .w_full()
                                            .px_3()
                                            .gap_2()
                                            .items_center()
                                            .bg(header_bg)
                                            .text_color(muted)
                                            .border_b_1()
                                            .border_color(border)
                                            .cursor_pointer()
                                            .child(crate::ui::reveal::chevron(
                                                SharedString::from(format!(
                                                    "group-chevron-{}-{id}",
                                                    pane.0
                                                )),
                                                !*collapsed,
                                                muted,
                                            ))
                                            .child(SharedString::from(label.clone()))
                                            .child(SharedString::from(format!("({count})")))
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(move |_, _, cx| {
                                                click_view.update(cx, |app, cx| {
                                                    app.process_actions(vec![
                                                        Action::ToggleGroup {
                                                            tab: pane,
                                                            id: id.clone(),
                                                        },
                                                    ]);
                                                    app.flush_dirty_panes(cx);
                                                });
                                            })
                                            .into_any_element(),
                                    )
                                }
                                GroupRow::Entry(index) => {
                                    let index = *index;
                                    let tab = app.state.tab_states.get(&pane)?;
                                    let entry = tab.entries.get(index)?.clone();
                                    let selected = tab.selection.selected.contains(&index);
                                    let cursor = tab.selection.keyboard_cursor == Some(index);
                                    // Same rule as the canvas views: the gutter exists while
                                    // more than one item is selected, so names never shift
                                    // on hover and the checkbox pops in on selection.
                                    let show_checks = tab.selection.selected.len() > 1;
                                    let file_ref = crate::ui::file_icons::FileIconRef {
                                        path: entry.path(),
                                        kind: entry.kind().clone(),
                                        extension: entry
                                            .extension()
                                            .as_ref()
                                            .map(|ext| SharedString::from(ext.to_string())),
                                    };
                                    app.maybe_request_file_icon(&file_ref);
                                    let file_icon = app.file_icon(&file_ref);
                                    let rename = super::rename::active_input(app, pane, index);
                                    let opacity =
                                        if app.state.system_cut_paths.contains(&entry.path()) {
                                            0.5
                                        } else {
                                            1.0
                                        };
                                    let font_size =
                                        window.text_style().font_size.to_pixels(window.rem_size());
                                    let name = match &rename {
                                        Some(input) => super::rename::input_element(
                                            input,
                                            &rows_view,
                                            px(row_h),
                                            font_size,
                                            window,
                                            cx,
                                        ),
                                        None => div()
                                            .truncate()
                                            .child(SharedString::from(entry.name.to_string()))
                                            .into_any_element(),
                                    };
                                    let row = h_flex()
                                        .id(("group-file", index))
                                        .h(px(row_h))
                                        .w_full()
                                        .px_3()
                                        .gap_2()
                                        .items_center()
                                        .opacity(opacity)
                                        .text_color(if selected { accent_fg } else { fg })
                                        .when(selected, |el| el.bg(accent))
                                        .when(!selected, |el| el.hover(|el| el.bg(hover)))
                                        .when(cursor && !selected, |el| {
                                            el.border_1().border_color(accent)
                                        })
                                        .when(show_checks, |el| {
                                            el.child(
                                                div()
                                                    .size(px(16.0 * zoom))
                                                    .flex_none()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .when(selected, |el| {
                                                        el.child(
                                                            crate::ui::selection_checkbox::SelectionCheckbox::new(
                                                                SharedString::from(format!(
                                                                    "group-check-{}-{index}",
                                                                    pane.0
                                                                )),
                                                                15.0 * zoom,
                                                            ),
                                                        )
                                                    }),
                                            )
                                        })
                                        .child(file_icon.element(16.0 * zoom, accent))
                                        .child(div().flex_1().min_w(px(0.)).child(name))
                                        .child(
                                            div()
                                                .w(px(85.0 * zoom))
                                                .text_right()
                                                .text_color(if selected {
                                                    accent_fg
                                                } else {
                                                    muted
                                                })
                                                .child(SharedString::from(
                                                    crate::ui::views::details::entry_display_size(&entry).to_string(),
                                                )),
                                        )
                                        .child(
                                            div()
                                                .w(px(140.0 * zoom))
                                                .truncate()
                                                .text_color(if selected {
                                                    accent_fg
                                                } else {
                                                    muted
                                                })
                                                .child(SharedString::from(
                                                    crate::ui::views::details::entry_display_date(&entry).to_string(),
                                                )),
                                        )
                                        .on_prepaint_child(rect_recorder(pane, index, &rows_view))
                                        .on_click(super::on_entry_click(&rows_view, pane, index))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            super::on_entry_ctx_menu(&rows_view, pane, index),
                                        )
                                        .when(selected && rename.is_none(), |el| {
                                            el.on_file_drag_item(
                                                rows_view.clone(),
                                                pane,
                                                index,
                                                entry.path(),
                                            )
                                        })
                                        .when(entry.kind() == FileKind::Dir, |el| {
                                            super::folder_drop_target(
                                                el,
                                                &rows_view,
                                                pane,
                                                entry.path(),
                                            )
                                        });
                                    Some(row.into_any_element())
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .track_scroll(&scroll)
        .flex_1()
        .h_full()
        .min_h(px(0.))
        .into_any_element();
        let list = if no_rows {
            v_flex().size_full().items_center().justify_center().text_color(muted)
                .children(self.empty_pane_message(pane, rust_i18n::t!("empty_folder").into_owned()))
                .into_any_element()
        } else { list };

        // A compact, always reachable selector also exposes "None" after all
        // buckets have been collapsed, independent of file selection.
        let selector = h_flex()
            .w_full()
            .gap_2()
            .px_3()
            .py_1()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(SharedString::from(rust_i18n::t!("group_by").into_owned()))
            .children(
                [
                    GroupBy::None,
                    GroupBy::Date,
                    GroupBy::Size,
                    GroupBy::Type,
                    GroupBy::Name,
                ]
                .map(|value| {
                    let selector_view = view.clone();
                    div()
                        .id(SharedString::from(format!("group-mode-{value:?}")))
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(mode == value, |el| el.bg(accent).text_color(accent_fg))
                        .hover(|el| el.bg(hover))
                        .child(SharedString::from(value.label()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            selector_view.update(cx, |app, cx| {
                                app.activate_tab(pane);
                                app.process_actions(vec![Action::SetGroupBy(value)]);
                                app.flush_dirty_panes(cx);
                            })
                        })
                }),
            );
        super::pane_background_interactions(
            v_flex().id(("pane", pane.0)).size_full().overflow_hidden(),
            &view,
            pane,
        )
        .when(cfg!(windows), |el| {
            el.on_prepaint_child(zone_recorder(pane, &view))
        })
        .child(crumbs)
        .child(selector)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .on_prepaint_child(pane_bounds_recorder(pane, &view))
                .child(list),
        )
        .into_any_element()
    }
}
