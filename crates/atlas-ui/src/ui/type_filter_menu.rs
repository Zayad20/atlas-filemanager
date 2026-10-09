//! Searchable type-filter overlay for the Details view.
//!
//! Opened when the user clicks the "Tipo" header cell. Renders as an
//! absolutely-positioned panel anchored below the click point, with a search
//! input that filters the list of unique type labels for the current pane.
//!
//! Follows the same structural pattern as `shell_context_menu.rs`:
//!   - State lives on `App` (`type_filter_menu`, `type_filter_input`, `type_filter_sub`).
//!   - `open_type_filter_menu` / `close_type_filter_menu` manage the lifecycle.
//!   - `render_type_filter_layer` paints the overlay layer (called from `overlays/mod.rs`).
//!   - Focus is claimed by `overlay_focus.rs` → `overlay_focus_target`.

use crate::app::App;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use atlas_model::file_entry::FileKind;
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    div, px, AnimationExt, AnyElement, Context, MouseButton, Pixels, Point, SharedString, Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable as _};
use std::collections::HashMap;

const SCROLLBAR_GUTTER: f32 = 16.0;

/// State for the open type-filter overlay.
pub(crate) struct TypeFilterMenuState {
    pub pane: TabId,
    /// Window-relative position of the click that opened the overlay.
    pub anchor: Point<Pixels>,
    /// All unique type rows for the current pane (pre-computed on open).
    pub items: Vec<TypeFilterItem>,
}

#[derive(Clone)]
pub(crate) struct TypeFilterItem {
    pub label: String,
    pub count: usize,
    pub kind: FileKind,
    pub ext: Option<String>,
}

impl App {
    /// Open the type-filter overlay for `pane`, anchored below `anchor`.
    pub(crate) fn open_type_filter_menu(
        &mut self,
        pane: TabId,
        anchor: Point<Pixels>,
        labels: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_ctx_menu(cx);
        let items = self.type_filter_items(pane, labels);
        let placeholder = rust_i18n::t!("type_filter_placeholder", count = items.len()).to_string();
        let input = crate::ui::text_input::single_line(window, cx, &placeholder, "");
        self.type_filter_sub = Some(cx.subscribe(
            &input,
            |_this, _input, ev: &gpui_component::input::InputEvent, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        self.type_filter_input = Some(input);
        self.type_filter_menu = Some(TypeFilterMenuState {
            pane,
            anchor,
            items,
        });
        cx.notify();
    }

    fn type_filter_items(&self, pane: TabId, labels: Vec<String>) -> Vec<TypeFilterItem> {
        let mut by_label: HashMap<String, TypeFilterItem> = labels
            .into_iter()
            .map(|label| {
                (
                    label.clone(),
                    TypeFilterItem {
                        label,
                        count: 0,
                        kind: FileKind::File,
                        ext: None,
                    },
                )
            })
            .collect();
        if let Some(ts) = self.state.tab_states.get(&pane) {
            for entry in ts.entries.iter() {
                let label = crate::ui::views::details::entry_type_label(entry);
                let item = by_label
                    .entry(label.clone())
                    .or_insert_with(|| TypeFilterItem {
                        label,
                        count: 0,
                        kind: entry.kind.clone(),
                        ext: entry.extension.clone(),
                    });
                item.count += 1;
                if item.ext.is_none() {
                    item.kind = entry.kind.clone();
                    item.ext = entry.extension.clone();
                }
            }
        }
        let mut items: Vec<_> = by_label
            .into_values()
            .filter(|item| item.count > 0)
            .collect();
        items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
        items
    }

    /// Close the type-filter overlay.
    pub(crate) fn close_type_filter_menu(&mut self, cx: &mut Context<Self>) {
        self.type_filter_menu = None;
        self.type_filter_input = None;
        self.type_filter_sub = None;
        cx.notify();
    }

    /// Render the type-filter overlay layer (called from `render_overlays`).
    pub(crate) fn render_type_filter_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.type_filter_menu.as_ref() else {
            return div().into_any_element();
        };
        let pane = state.pane;
        let items = state.items.clone();
        let anchor = state.anchor;

        let (popover, fg, muted, border, accent, accent_fg) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
            )
        };

        let query = self
            .type_filter_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string().to_lowercase())
            .unwrap_or_default();

        let type_filter = self.type_filters.get(&pane).cloned().unwrap_or_default();
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
        self.type_filter_scroll
            .set_smooth(smooth_ease, smooth_boost);

        // Clamp the panel into the window.
        let win = window.bounds().size;
        const W: f32 = 340.;
        let row_h = 30_f32;
        let visible_items: Vec<TypeFilterItem> = items
            .iter()
            .filter(|item| query.is_empty() || item.label.to_lowercase().contains(&query))
            .cloned()
            .collect();
        const MAX_H: f32 = 640.;
        const V_MARGIN: f32 = 72.;
        const INPUT_H: f32 = 40.;
        let content_h = (visible_items.len() as f32 + 1.0) * row_h + 8.0;
        let ax = f32::from(anchor.x);
        let ay = f32::from(anchor.y);
        let ww = f32::from(win.width);
        let wh = f32::from(win.height);
        let max_h = MAX_H.min((wh - V_MARGIN * 2.0).max(180.0));
        let panel_h = (content_h + INPUT_H).min(max_h);
        let list_h = panel_h - INPUT_H;
        let left = ax.min((ww - W - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - panel_h - 6.0).max(6.0)).max(6.0);

        let input = self.type_filter_input.clone();
        let view = cx.entity();

        // Build type-filter rows.
        let mut rows: Vec<AnyElement> = Vec::new();

        // "Todos los tipos" row.
        let all_checked = type_filter.is_empty();
        let v_all = view.clone();
        let total_count = items.iter().map(|item| item.count).sum();
        rows.push(
            type_filter_row(
                SharedString::from("tf-all"),
                None,
                &rust_i18n::t!("all_types"),
                total_count,
                all_checked,
                fg,
                muted,
                border,
                accent,
                accent_fg,
            )
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                v_all.update(cx, |this, cx| {
                    this.type_filters.remove(&pane);
                    this.close_type_filter_menu(cx);
                });
            })
            .into_any_element(),
        );

        // One row per matching label.
        for item in &visible_items {
            let label = item.label.clone();
            let checked = type_filter.contains(&label);
            let v = view.clone();
            let lc = label.clone();
            let icon_el = Some(file_type_icon(&item.kind, item.ext.as_deref(), 16.));

            rows.push(
                type_filter_row(
                    SharedString::from(format!("tf-{}", lc)),
                    icon_el,
                    &label,
                    item.count,
                    checked,
                    fg,
                    muted,
                    border,
                    accent,
                    accent_fg,
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    let ls = lc.clone();
                    v.update(cx, |this, cx| {
                        let set = this.type_filters.entry(pane).or_default();
                        if set.contains(&ls) {
                            set.remove(&ls);
                            if set.is_empty() {
                                this.type_filters.remove(&pane);
                            }
                        } else {
                            set.insert(ls);
                        }
                        cx.notify();
                    });
                })
                .into_any_element(),
            );
        }

        let input_row = input.as_ref().map(|state| {
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(border)
                .items_center()
                .child(icon(AtlasIcon::Search, 14., muted))
                .child(div().flex_1().child(crate::ui::text_input::flat(state)))
        });

        let panel = v_flex()
            .id("type-filter-panel")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(W))
            .h(px(panel_h))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .overflow_hidden()
            .children(input_row)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .max_h(px(list_h))
                    .child(
                        v_flex()
                            .id("type-filter-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.type_filter_scroll)
                            .py_1()
                            .pr(px(SCROLLBAR_GUTTER))
                            .children(rows),
                    )
                    .vertical_scrollbar(&self.type_filter_scroll),
            )
            .with_animation(
                SharedString::from("type-filter-panel-appear"),
                crate::ui::motion::animation(crate::ui::motion::MICRO_MS),
                |this, t| this.opacity(t).mt(px(5.0 * (1.0 - t))),
            );

        let v = view.clone();
        gpui::deferred(
            div()
                .id("type-filter-scrim")
                .absolute()
                .inset_0()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    v.update(cx, |this, cx| {
                        this.close_type_filter_menu(cx);
                    });
                })
                .on_mouse_down(MouseButton::Right, {
                    let v = view.clone();
                    move |_, _, cx| {
                        v.update(cx, |this, cx| {
                            this.close_type_filter_menu(cx);
                        });
                    }
                })
                .child(panel),
        )
        .into_any_element()
    }
}

/// One filter row: optional icon + label + checkmark on the right.
#[allow(clippy::too_many_arguments)]
fn type_filter_row(
    id: SharedString,
    icon: Option<AnyElement>,
    label: &str,
    count: usize,
    checked: bool,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    accent: gpui::Hsla,
    accent_fg: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let (bg, text, count_color) = if checked {
        (accent.opacity(0.92), accent_fg, accent_fg.opacity(0.9))
    } else {
        (gpui::transparent_black(), fg, muted)
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(30.))
        .bg(bg)
        .text_color(text)
        .text_sm()
        .rounded(px(6.))
        .hover(move |s| s.bg(if checked { bg } else { border.opacity(0.5) }))
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if checked {
                    accent_fg.opacity(0.75)
                } else {
                    border
                })
                .when(checked, |el| {
                    el.child(
                        Icon::new(IconName::Check)
                            .with_size(gpui_component::Size::XSmall)
                            .text_color(accent_fg),
                    )
                }),
        )
        .when_some(icon, |el, icon| el.child(icon))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .min_w(px(24.))
                .text_right()
                .text_color(count_color)
                .child(SharedString::from(count.to_string())),
        )
}
