//! Searchable tag-picker overlay for the inspector preview.
//!
//! Opened via `Action::OpenTagPicker` when the user clicks the "Añadir tag" button
//! below an image in the inspector. Renders as an absolutely-positioned panel anchored
//! to the button, with a search input that filters existing tags.
//!
//! - Existing tags: shown as rows with a color swatch. Clicking assigns the tag.
//! - "Crear tag «X»" appears at the bottom when the query has no match; clicking it
//!   dispatches `Action::CreateTag` (which opens the create-tag modal pre-seeded).
//!
//! Follows the exact structural pattern of `type_filter_menu.rs`:
//!   - State lives on `App` (`tag_picker_menu`, `tag_picker_input`, `tag_picker_sub`).
//!   - `open_tag_picker` / `close_tag_picker` manage lifecycle.
//!   - `render_tag_picker_layer` paints the overlay layer (called from `overlays/mod.rs`).
//!   - Focus is claimed by `overlay_focus.rs` → `overlay_focus_target`.

use crate::app::App;
use crate::input::action::Action;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton, Pixels, Point, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme, Sizable as _};

const SCROLLBAR_GUTTER: f32 = 16.0;

/// State for the open tag-picker overlay.
pub(crate) struct TagPickerState {
    /// Window-relative position used to anchor the panel.
    pub anchor: Point<Pixels>,
    /// Paths that will receive the chosen tag (usually just the focused image).
    pub paths: Vec<std::path::PathBuf>,
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
        let input = crate::ui::text_input::single_line(window, cx, "Buscar tag…", "");
        self.tag_picker_sub = Some(cx.subscribe(
            &input,
            |_this, _input, ev: &gpui_component::input::InputEvent, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        self.tag_picker_input = Some(input);
        self.tag_picker_menu = Some(TagPickerState { anchor, paths });
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
            .tag_picker_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string().to_lowercase())
            .unwrap_or_default();

        let profile_name = self.state.profiles.active().name.clone();

        // Paths currently known to have tags (for checkmark display).
        let first_path_tags: Vec<crate::state::tag_cache::TagId> = paths
            .first()
            .map(|p| self.state.tag_cache.tags_of(p).to_vec())
            .unwrap_or_default();

        // Collect matching tags.
        let visible_tags: Vec<(crate::state::tag_cache::TagId, String, [u8; 3], bool)> = self
            .state
            .tag_cache
            .visible_defs(&profile_name)
            .filter(|d| query.is_empty() || d.name.to_lowercase().contains(&query))
            .map(|d| {
                let assigned = first_path_tags.contains(&d.id);
                (d.id, d.name.clone(), d.color, assigned)
            })
            .collect();

        let any_match = !visible_tags.is_empty();

        // Panel geometry.
        let win = window.bounds().size;
        const W: f32 = 260.;
        let row_h = 32_f32;
        // +1 for the optional "Crear" row, +1 for search, small padding.
        let create_row = if !any_match && !query.is_empty() {
            1.0
        } else {
            0.0
        };
        let est_h = (visible_tags.len() as f32 + create_row) * row_h + row_h /* search */ + 52.0;
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

        for (tag_id, name, color, assigned) in &visible_tags {
            let tag_id = *tag_id;
            let assigned = *assigned;
            let paths_c = paths.clone();
            let name_c = name.clone();
            let v = view.clone();
            let color_swatch = {
                let [r, g, b] = *color;
                gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
            };
            rows.push(
                div()
                    .id(SharedString::from(format!("tp-{}", name_c)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(row_h))
                    .text_color(fg)
                    .text_sm()
                    .rounded(px(4.))
                    .hover(|s| s.bg(accent).text_color(accent_fg))
                    .cursor_pointer()
                    // Color swatch.
                    .child(
                        div()
                            .w(px(12.))
                            .h(px(12.))
                            .rounded_full()
                            .flex_none()
                            .bg(color_swatch),
                    )
                    // Tag name.
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(SharedString::from(name.clone())),
                    )
                    // Checkmark when already assigned.
                    .when(assigned, |el| {
                        el.child(
                            gpui_component::Icon::new(gpui_component::IconName::Check)
                                .with_size(gpui_component::Size::XSmall),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        v.update(cx, |this, cx| {
                            if assigned {
                                // Toggle off: unassign.
                                this.state.tag_cache.unassign_local(tag_id, &paths_c);
                                let _ =
                                    this.tag_db_tx
                                        .send(crate::fs::tag_db::TagDbCmd::UnassignTag {
                                            tag: tag_id,
                                            paths: paths_c.clone(),
                                        });
                            } else {
                                // Toggle on: assign.
                                this.state.tag_cache.assign_local(tag_id, &paths_c);
                                let _ =
                                    this.tag_db_tx.send(crate::fs::tag_db::TagDbCmd::AssignTag {
                                        tag: tag_id,
                                        paths: paths_c.clone(),
                                    });
                            }
                            this.close_tag_picker(cx);
                        });
                    })
                    .into_any_element(),
            );
        }

        // "Crear tag «X»" row — only when query has no match.
        if !any_match && !query.is_empty() {
            let label = format!("Crear tag «{}»", query);
            let label_shared = SharedString::from(label.clone());
            let create_name = query.clone();
            let v = view.clone();
            rows.push(
                div()
                    .id(SharedString::from("tp-create"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(row_h))
                    .text_color(muted)
                    .text_sm()
                    .italic()
                    .rounded(px(4.))
                    .hover(|s| s.bg(accent).text_color(accent_fg))
                    .cursor_pointer()
                    .child(
                        gpui_component::Icon::new(gpui_component::IconName::Plus)
                            .with_size(gpui_component::Size::XSmall),
                    )
                    .child(label_shared)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let name = create_name.clone();
                        v.update(cx, |this, cx| {
                            // Close the picker and open the create-tag modal pre-seeded.
                            this.close_tag_picker(cx);
                            this.process_actions(vec![Action::OpenCreateTagOverlay {
                                global: false,
                            }]);
                            // Pre-fill the name in the create-tag form state.
                            this.state.profile_ui.new_tag_name = name;
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
            .into_any_element()
    }
}
