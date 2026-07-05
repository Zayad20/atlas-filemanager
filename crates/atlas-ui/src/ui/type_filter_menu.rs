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
use crate::ui::icons::file_type_icon;
use atlas_model::file_entry::FileKind;
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton, Pixels, Point, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme, Sizable as _};

const SCROLLBAR_GUTTER: f32 = 16.0;

/// State for the open type-filter overlay.
pub(crate) struct TypeFilterMenuState {
    pub pane: TabId,
    /// Window-relative position of the click that opened the overlay.
    pub anchor: Point<Pixels>,
    /// All unique type labels for the current pane (pre-computed on open).
    pub labels: Vec<String>,
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
        let input = crate::ui::text_input::single_line(window, cx, "Buscar tipo…", "");
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
            labels,
        });
        cx.notify();
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
        let labels = state.labels.clone();
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

        // Clamp the panel into the window.
        let win = window.bounds().size;
        const W: f32 = 260.;
        let row_h = 28_f32;
        let visible_labels: Vec<String> = labels
            .iter()
            .filter(|l| query.is_empty() || l.to_lowercase().contains(&query))
            .cloned()
            .collect();
        // +1 for "Todos", +1 for search input row, small padding.
        let est_h = visible_labels.len() as f32 * row_h + row_h + 52.0;
        let ax = f32::from(anchor.x);
        let ay = f32::from(anchor.y);
        let ww = f32::from(win.width);
        let wh = f32::from(win.height);
        let max_h = 520.0_f32.min((wh - 72.0 * 2.0).max(140.0));
        let panel_h = est_h.clamp(row_h * 4.0, max_h);
        let list_h = (panel_h - 44.0).max(row_h * 2.0);
        let left = ax.min((ww - W - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - panel_h - 6.0).max(6.0)).max(6.0);

        let input = self.type_filter_input.clone();
        let view = cx.entity();

        // Build type-filter rows.
        let mut rows: Vec<AnyElement> = Vec::new();

        // "Todos los tipos" row.
        let all_checked = type_filter.is_empty();
        let v_all = view.clone();
        rows.push(
            type_filter_row(
                SharedString::from("tf-all"),
                None,
                &rust_i18n::t!("all_types"),
                all_checked,
                fg,
                muted,
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

        // Separator.
        rows.push(
            div()
                .h(px(1.))
                .w_full()
                .my(px(2.))
                .bg(border)
                .into_any_element(),
        );

        // One row per matching label.
        for label in &visible_labels {
            let label = label.clone();
            let checked = type_filter.contains(&label);
            let v = view.clone();
            let lc = label.clone();

            // Derive kind + representative ext for the icon.
            let (kind, ext_opt): (FileKind, Option<String>) =
                if label == rust_i18n::t!("file_kind_folder") {
                    (FileKind::Dir, None)
                } else if label == rust_i18n::t!("file_kind_symlink") {
                    (FileKind::Symlink, None)
                } else if label.starts_with('.') {
                    (FileKind::File, Some(label[1..].to_string()))
                } else {
                    let ext = if label == rust_i18n::t!("file_kind_image") {
                        Some("png")
                    } else if label == rust_i18n::t!("file_kind_audio") {
                        Some("mp3")
                    } else if label == rust_i18n::t!("file_kind_video") {
                        Some("mp4")
                    } else if label == rust_i18n::t!("file_kind_code") {
                        Some("rs")
                    } else if label == rust_i18n::t!("file_kind_archive") {
                        Some("zip")
                    } else if label == rust_i18n::t!("file_kind_document") {
                        Some("pdf")
                    } else if label == rust_i18n::t!("file_kind_spreadsheet") {
                        Some("xlsx")
                    } else if label == rust_i18n::t!("file_kind_presentation") {
                        Some("pptx")
                    } else if label == rust_i18n::t!("file_kind_application") {
                        Some("exe")
                    } else {
                        None
                    };
                    (FileKind::File, ext.map(|s| s.to_string()))
                };

            let icon_el = Some(file_type_icon(&kind, ext_opt.as_deref(), 14.));

            rows.push(
                type_filter_row(
                    SharedString::from(format!("tf-{}", lc)),
                    icon_el,
                    &label,
                    checked,
                    fg,
                    muted,
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

        let close_view = view.clone();

        // Full-window backdrop (click-to-close).
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                close_view.update(cx, |this, cx| {
                    this.close_type_filter_menu(cx);
                });
            })
            // The panel itself (stop propagation so clicks inside don't close it).
            .child(
                v_flex()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(W))
                    .h(px(panel_h))
                    .bg(popover)
                    .border_1()
                    .border_color(border)
                    .rounded(px(crate::ui::RADIUS))
                    .shadow_lg()
                    .py_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    // Search input.
                    .when_some(input, |el, input| {
                        el.child(
                            div()
                                .px_2()
                                .py_1()
                                .child(crate::ui::text_input::bordered(&input)),
                        )
                    })
                    // Scrollable list.
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
                                    .pr(px(SCROLLBAR_GUTTER))
                                    .children(rows),
                            )
                            .vertical_scrollbar(&self.type_filter_scroll),
                    ),
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
    checked: bool,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    accent: gpui::Hsla,
    accent_fg: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let (bg, text, check_color) = if checked {
        (accent, accent_fg, accent_fg)
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
        .h(px(28.))
        .bg(bg)
        .text_color(text)
        .text_sm()
        .rounded(px(4.))
        .hover(|s| s.opacity(0.85))
        .cursor_pointer()
        .when_some(icon, |el, icon| el.child(icon))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(SharedString::from(label.to_string())),
        )
        .when(checked, |el| {
            el.child(
                gpui_component::Icon::new(gpui_component::IconName::Check)
                    .with_size(gpui_component::Size::XSmall)
                    .text_color(check_color),
            )
        })
}
