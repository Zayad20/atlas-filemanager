//! Searchable tag-picker overlay for the inspector preview.
//!
//! Opened via `Action::OpenTagPicker` when the user clicks the add-tag button
//! below an image in the inspector. Renders as an absolutely-positioned panel anchored
//! to the button, with a search input that filters existing tags.
//!
//! - Existing tags: shown as rows with a color swatch. Clicking assigns the tag.
//! - A create-tag row appears at the bottom when the query has no match; clicking it
//!   dispatches `Action::CreateTag` (which opens the create-tag modal pre-seeded).
//!
//! Follows the exact structural pattern of `type_filter_menu.rs`:
//!   - State lives on `App` (`tag_picker_menu`, `tag_picker_input`, `tag_picker_sub`).
//!   - `open_tag_picker` / `close_tag_picker` manage lifecycle.
//!   - `render_tag_picker_layer` paints the overlay layer (called from `overlays/mod.rs`).
//!   - Focus is claimed by `overlay_focus.rs` → `overlay_focus_target`.

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::{icon, AtlasIcon};
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    div, px, AnyElement, Context, KeyDownEvent, MouseButton, Pixels, Point, SharedString, Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::StyledExt as _;
use gpui_component::{v_flex, ActiveTheme, Sizable as _};

const SCROLLBAR_GUTTER: f32 = 16.0;

/// State for the open tag-picker overlay.
pub(crate) struct TagPickerState {
    /// Window-relative position used to anchor the panel.
    pub anchor: Point<Pixels>,
    /// Paths that will receive the chosen tag (usually just the focused image).
    pub paths: Vec<std::path::PathBuf>,
    pub selected: usize,
    pub scroll_to_selected: bool,
}

#[derive(Clone)]
enum TagPickerItem {
    Existing {
        id: atlas_model::tag_cache::TagId,
        name: String,
        color: [u8; 3],
        assigned: bool,
    },
    Create {
        name: String,
    },
}

impl App {
    /// Open the tag-picker overlay anchored at `anchor`, targeting `paths`.
    pub(crate) fn open_tag_picker(
        &mut self,
        anchor: Point<Pixels>,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = crate::ui::text_input::single_line(
            window,
            cx,
            rust_i18n::t!("search_tag").into_owned(),
            "",
        );
        self.tag_picker_sub = Some(cx.subscribe(
            &input,
            |_this, _input, ev: &gpui_component::input::InputEvent, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        self.tag_picker_input = Some(input);
        self.tag_picker_menu = Some(TagPickerState {
            anchor,
            paths,
            selected: 0,
            scroll_to_selected: true,
        });
        cx.notify();
    }

    /// Close the tag-picker overlay.
    pub(crate) fn close_tag_picker(&mut self, cx: &mut Context<Self>) {
        self.tag_picker_menu = None;
        self.tag_picker_input = None;
        self.tag_picker_sub = None;
        cx.notify();
    }

    /// Render the tag-picker overlay layer (called from `render_overlays`).
    pub(crate) fn render_tag_picker_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.tag_picker_menu.as_ref() else {
            return div().into_any_element();
        };
        let anchor = state.anchor;
        let paths = state.paths.clone();

        let (fg, muted, border, accent, accent_fg) = {
            let t = cx.theme();
            (
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
            )
        };

        let query = self
            .tag_picker_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string().to_lowercase())
            .unwrap_or_default();
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
        self.tag_picker_scroll.set_smooth(smooth_ease, smooth_boost);

        let profile_name = self.state.profiles.active().name.clone();

        // Paths currently known to have tags (for checkmark display).
        let first_path_tags: Vec<atlas_model::tag_cache::TagId> = paths
            .first()
            .map(|p| self.state.tag_cache.tags_of(p).to_vec())
            .unwrap_or_default();

        // Collect matching tags.
        let mut items: Vec<TagPickerItem> = self
            .state
            .tag_cache
            .visible_defs(&profile_name)
            .filter(|d| query.is_empty() || d.name.to_lowercase().contains(&query))
            .map(|d| {
                let assigned = first_path_tags.contains(&d.id);
                TagPickerItem::Existing {
                    id: d.id,
                    name: d.name.clone(),
                    color: d.color,
                    assigned,
                }
            })
            .collect();

        let any_match = !items.is_empty();
        if !any_match && !query.is_empty() {
            items.push(TagPickerItem::Create {
                name: query.clone(),
            });
        }
        let selected = if items.is_empty() {
            0
        } else {
            let state = self.tag_picker_menu.as_mut().unwrap();
            if state.selected >= items.len() {
                state.selected = items.len() - 1;
                state.scroll_to_selected = true;
            }
            state.selected
        };
        if self
            .tag_picker_menu
            .as_ref()
            .is_some_and(|state| state.scroll_to_selected)
        {
            self.tag_picker_scroll.scroll_to_item(selected);
            self.tag_picker_menu.as_mut().unwrap().scroll_to_selected = false;
        }

        // Panel geometry.
        let win = window.bounds().size;
        const W: f32 = 260.;
        let row_h = 32_f32;
        // +1 for the optional "Crear" row, +1 for search, small padding.
        let est_h = (items.len() as f32) * row_h + row_h /* search */ + 52.0;
        let ax = f32::from(anchor.x);
        let ay = f32::from(anchor.y);
        let ww = f32::from(win.width);
        let wh = f32::from(win.height);
        let max_h = 480.0_f32.min((wh - 72.0 * 2.0).max(140.0));
        let panel_h = est_h.clamp(row_h * 3.0, max_h);
        let list_h = (panel_h - 48.0).max(row_h * 2.0);
        let left = ax.min((ww - W - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - panel_h - 6.0).max(6.0)).max(6.0);

        let input = self.tag_picker_input.clone();
        let view = cx.entity();

        // Build rows.
        let mut rows: Vec<AnyElement> = Vec::new();

        for (index, item) in items.iter().cloned().enumerate() {
            let is_selected = index == selected;
            let v = view.clone();
            rows.push(
                tag_picker_row(
                    index,
                    &item,
                    fg,
                    muted,
                    border,
                    accent,
                    accent_fg,
                    is_selected,
                    row_h,
                )
                .on_mouse_move({
                    let v = view.clone();
                    move |_, _, cx| {
                        v.update(cx, |this, cx| {
                            if let Some(state) = this.tag_picker_menu.as_mut() {
                                state.selected = index;
                                cx.notify();
                            }
                        });
                    }
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    let item = item.clone();
                    v.update(cx, |this, cx| {
                        this.activate_tag_picker_item(item, cx);
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
                    this.close_tag_picker(cx);
                });
            })
            // Panel itself.
            .child(
                v_flex()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(W))
                    .h(px(panel_h))
                    .popover_style(cx)
                    .py_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    // Search input.
                    .when_some(input, |el, input| {
                        el.child(
                            gpui_component::h_flex()
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .border_b_1()
                                .border_color(border)
                                .child(icon(AtlasIcon::Search, 14., muted))
                                .child(div().flex_1().child(crate::ui::text_input::flat(&input))),
                        )
                    })
                    // Scrollable tag list.
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h(px(0.))
                            .max_h(px(list_h))
                            .child(
                                v_flex()
                                    .id("tag-picker-list")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.tag_picker_scroll)
                                    .pr(px(SCROLLBAR_GUTTER))
                                    .children(rows),
                            )
                            .vertical_scrollbar(&self.tag_picker_scroll),
                    ),
            )
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                if this.handle_tag_picker_key(ev, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn handle_tag_picker_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let items = self.visible_tag_picker_items(cx);
        let Some(state) = self.tag_picker_menu.as_mut() else {
            return false;
        };

        match ev.keystroke.key.as_str() {
            "escape" => self.close_tag_picker(cx),
            "down" if !items.is_empty() => {
                state.selected = (state.selected + 1) % items.len();
                state.scroll_to_selected = true;
            }
            "up" if !items.is_empty() => {
                state.selected = (state.selected + items.len() - 1) % items.len();
                state.scroll_to_selected = true;
            }
            "enter" => {
                let Some(item) = items.get(state.selected).cloned() else {
                    return true;
                };
                self.activate_tag_picker_item(item, cx);
            }
            "down" | "up" => {}
            _ => return false,
        }
        true
    }

    fn visible_tag_picker_items(&self, cx: &mut Context<Self>) -> Vec<TagPickerItem> {
        let Some(state) = self.tag_picker_menu.as_ref() else {
            return Vec::new();
        };
        let query = self
            .tag_picker_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string().to_lowercase())
            .unwrap_or_default();
        let profile_name = self.state.profiles.active().name.clone();
        let first_path_tags: Vec<atlas_model::tag_cache::TagId> = state
            .paths
            .first()
            .map(|p| self.state.tag_cache.tags_of(p).to_vec())
            .unwrap_or_default();

        let mut items: Vec<TagPickerItem> = self
            .state
            .tag_cache
            .visible_defs(&profile_name)
            .filter(|d| query.is_empty() || d.name.to_lowercase().contains(&query))
            .map(|d| TagPickerItem::Existing {
                id: d.id,
                name: d.name.clone(),
                color: d.color,
                assigned: first_path_tags.contains(&d.id),
            })
            .collect();
        if items.is_empty() && !query.is_empty() {
            items.push(TagPickerItem::Create { name: query });
        }
        items
    }

    fn activate_tag_picker_item(&mut self, item: TagPickerItem, cx: &mut Context<Self>) {
        match item {
            TagPickerItem::Existing { id, assigned, .. } => {
                let paths = self
                    .tag_picker_menu
                    .as_ref()
                    .map(|state| state.paths.clone())
                    .unwrap_or_default();
                if assigned {
                    self.state.tag_cache.unassign_local(id, &paths);
                    let _ = self
                        .tag_db_tx
                        .send(atlas_core::fs::tag_db::TagDbCmd::UnassignTag { tag: id, paths });
                } else {
                    self.state.tag_cache.assign_local(id, &paths);
                    let _ = self
                        .tag_db_tx
                        .send(atlas_core::fs::tag_db::TagDbCmd::AssignTag { tag: id, paths });
                }
                self.close_tag_picker(cx);
            }
            TagPickerItem::Create { name } => {
                self.close_tag_picker(cx);
                self.process_actions(vec![Action::OpenCreateTagOverlay { global: false }]);
                self.state.profile_ui.new_tag_name = name;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tag_picker_row(
    index: usize,
    item: &TagPickerItem,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    accent: gpui::Hsla,
    accent_fg: gpui::Hsla,
    selected: bool,
    row_h: f32,
) -> gpui::Stateful<gpui::Div> {
    let (bg, text) = if selected {
        (border.opacity(0.5), accent_fg)
    } else {
        (gpui::transparent_black(), fg)
    };
    match item {
        TagPickerItem::Existing {
            name,
            color,
            assigned,
            ..
        } => {
            let [r, g, b] = *color;
            let color_swatch = gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
            div()
                .id(SharedString::from(format!("tp-{index}-{name}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(row_h))
                .bg(bg)
                .text_color(text)
                .text_sm()
                .rounded(px(4.))
                .hover(move |s| s.bg(accent).text_color(accent_fg))
                .cursor_pointer()
                .child(
                    div()
                        .w(px(12.))
                        .h(px(12.))
                        .rounded_full()
                        .flex_none()
                        .bg(color_swatch),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(SharedString::from(name.clone())),
                )
                .when(*assigned, |el| {
                    el.child(
                        gpui_component::Icon::new(gpui_component::IconName::Check)
                            .with_size(gpui_component::Size::XSmall),
                    )
                })
        }
        TagPickerItem::Create { name } => div()
            .id(SharedString::from("tp-create"))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(row_h))
            .bg(bg)
            .text_color(if selected { text } else { muted })
            .text_sm()
            .italic()
            .rounded(px(4.))
            .hover(move |s| s.bg(accent).text_color(accent_fg))
            .cursor_pointer()
            .child(
                gpui_component::Icon::new(gpui_component::IconName::Plus)
                    .with_size(gpui_component::Size::XSmall),
            )
            .child(SharedString::from(rust_i18n::t!(
                "create_tag",
                name = name.clone()
            ))),
    }
}
