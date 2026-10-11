//! Properties with Unix permission editing. Windows uses its native shell dialog.
//! Directory sizes show "—": inode size is not the size of their contents.

use crate::app::App;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use atlas_core::fs::properties::read_properties;
use atlas_core::util::display::{format_date, format_size};
use atlas_model::file_entry::FileKind;
use atlas_model::overlay::{PropertiesState, PropertyRow};
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{div, px, AnyElement, Context, KeyDownEvent, MouseButton, Window};
use gpui_component::scroll::ScrollableElement;
#[cfg(unix)]
use gpui_component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    Disableable,
};
use gpui_component::{h_flex, v_flex, ActiveTheme};
use std::path::PathBuf;

/// Stat `paths` synchronously (a handful of files/dirs — one `metadata()` /
/// `symlink_metadata()` call each) and build the overlay state.
pub(crate) fn stat_properties(paths: &[PathBuf]) -> PropertiesState {
    let rows = paths.iter().map(|path| read_properties(path)).collect();
    PropertiesState {
        rows,
        saving: false,
    }
}

impl App {
    #[allow(dead_code)] // reason: only called from the cfg(not(windows)) ctx-menu branch
    pub(crate) fn open_properties(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty()
            || self
                .state
                .overlay
                .properties
                .as_ref()
                .is_some_and(|s| s.saving)
        {
            return;
        }
        self.state.overlay.properties = Some(stat_properties(&paths));
    }

    fn close_properties(&mut self) {
        if !self
            .state
            .overlay
            .properties
            .as_ref()
            .is_some_and(|s| s.saving)
        {
            self.state.overlay.properties = None;
        }
    }

    #[cfg(unix)]
    fn apply_property_permissions(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.overlay.properties.as_mut() else {
            return;
        };
        if state.saving {
            return;
        }
        let edits: Vec<_> = state
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let permissions = row.permissions.as_ref()?;
                (!row.is_symlink && permissions.edited_mode != permissions.mode)
                    .then(|| (index, row.path.clone(), permissions.clone()))
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        state.saving = true;
        let task = cx.background_executor().spawn(async move {
            edits
                .into_iter()
                .map(|(index, path, permissions)| {
                    let result = atlas_core::fs::properties::apply_permissions(&path, &permissions)
                        .map(|()| read_properties(&path))
                        .map_err(|error| error.to_string());
                    (index, result)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let results = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(state) = this.state.overlay.properties.as_mut() {
                    for (index, result) in results {
                        if let Some(row) = state.rows.get_mut(index) {
                            match result {
                                Ok(refreshed) => *row = refreshed,
                                Err(error) => row.error = Some(error),
                            }
                        }
                    }
                    state.saving = false;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(unix)]
    fn property_permissions(
        row: &PropertyRow,
        index: usize,
        saving: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(permissions) = row.permissions.as_ref() else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().popover_foreground;
        let mode = permissions.edited_mode;
        let mut section = v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(rust_i18n::t!("properties_permissions").to_string()),
            )
            .child(prop_row(
                rust_i18n::t!("properties_owner").to_string(),
                format!("UID {}", permissions.uid),
                muted,
                fg,
            ))
            .child(prop_row(
                rust_i18n::t!("properties_group").to_string(),
                format!("GID {}", permissions.gid),
                muted,
                fg,
            ))
            .child(prop_row(
                rust_i18n::t!("properties_mode").to_string(),
                format!("{mode:04o}"),
                muted,
                fg,
            ));
        if row.is_symlink {
            return section
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(rust_i18n::t!("properties_symlink_permissions").to_string()),
                )
                .into_any_element();
        }
        for (label, shift) in [
            ("properties_owner", 6),
            ("properties_group", 3),
            ("properties_others", 0),
        ] {
            let mut line = h_flex().gap_2().items_center().child(
                div()
                    .w(px(90.))
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!(label).to_string()),
            );
            for (label, bit) in [
                ("properties_read", 4),
                ("properties_write", 2),
                ("properties_execute", 1),
            ] {
                let mask = bit << shift;
                line = line.child(
                    Checkbox::new(format!("permission-{index}-{mask}"))
                        .label(rust_i18n::t!(label).to_string())
                        .checked(mode & mask != 0)
                        .disabled(saving)
                        .on_click(cx.listener(move |this, checked: &bool, _window, cx| {
                            if let Some(state) = this.state.overlay.properties.as_mut() {
                                if !state.saving {
                                    if let Some(permissions) = state
                                        .rows
                                        .get_mut(index)
                                        .and_then(|row| row.permissions.as_mut())
                                    {
                                        if *checked {
                                            permissions.edited_mode |= mask;
                                        } else {
                                            permissions.edited_mode &= !mask;
                                        }
                                    }
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            section = section.child(line);
        }
        section
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("properties_permissions_scope").to_string()),
            )
            .into_any_element()
    }

    pub(crate) fn render_properties(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.state.overlay.properties.clone() else {
            return div().into_any_element();
        };
        let (popover, fg, muted, border) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
            )
        };
        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        let scroll = window
            .use_keyed_state("properties-scroll", cx, |_, _| gpui::ScrollHandle::new())
            .read(cx)
            .clone();
        let body_height = (window.viewport_size().height - px(168.)).max(px(80.));

        let mut body = v_flex().gap_4();
        if state.rows.len() > 1 {
            body = body.child(multi_summary(&state.rows, fg, muted));
        }
        for (index, row) in state.rows.iter().enumerate() {
            body = body.child(single_row(row, fg, muted));
            #[cfg(unix)]
            {
                body = body.child(Self::property_permissions(row, index, state.saving, cx));
            }
            #[cfg(not(unix))]
            let _ = index;
            if let Some(error) = &row.error {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(rust_i18n::t!("properties_error", detail = error).to_string()),
                );
            }
        }

        let close_view = view.clone();
        let panel = v_flex()
            .w(px(520.))
            .max_w(window.viewport_size().width - px(32.))
            .max_h(window.viewport_size().height - px(48.))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .p_4()
            .gap_3()
            .track_focus(&focus)
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_properties();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(rust_i18n::t!("properties_title").to_string()),
                    )
                    .child(
                        div()
                            .id("properties-close")
                            .size(px(22.))
                            .rounded(px(crate::ui::RADIUS))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |h| h.bg(border).text_color(fg))
                            .child(icon(AtlasIcon::X, 12., muted))
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                close_view.update(cx, |this, cx| {
                                    this.close_properties();
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("properties-body")
                            .max_h(body_height)
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .pr_2()
                            .child(body),
                    )
                    .vertical_scrollbar(&scroll),
            )
            .when(cfg!(unix), |panel| {
                #[cfg(unix)]
                {
                    let dirty = state.rows.iter().any(|row| {
                        !row.is_symlink
                            && row
                                .permissions
                                .as_ref()
                                .is_some_and(|p| p.edited_mode != p.mode)
                    });
                    panel.child(
                        h_flex()
                            .flex_shrink_0()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("properties-cancel")
                                    .label(rust_i18n::t!("properties_close").to_string())
                                    .disabled(state.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_properties();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("properties-apply")
                                    .primary()
                                    .label(
                                        rust_i18n::t!(if state.saving {
                                            "properties_applying"
                                        } else {
                                            "properties_apply"
                                        })
                                        .to_string(),
                                    )
                                    .disabled(!dirty || state.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.apply_property_permissions(cx)
                                    })),
                            ),
                    )
                }
                #[cfg(not(unix))]
                panel
            });

        self.overlay_scrim_custom(
            "scrim-properties",
            view,
            |this| this.close_properties(),
            focus,
            panel.into_any_element(),
            cx,
        )
    }
}

fn prop_row(label: String, value: String, muted: gpui::Hsla, fg: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(div().w(px(90.)).text_xs().text_color(muted).child(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(fg)
                .child(value),
        )
}

fn single_row(row: &PropertyRow, fg: gpui::Hsla, muted: gpui::Hsla) -> AnyElement {
    let kind_label = match row.kind {
        FileKind::Dir => rust_i18n::t!("properties_kind_folder").to_string(),
        FileKind::Symlink => rust_i18n::t!("properties_kind_symlink").to_string(),
        FileKind::File => rust_i18n::t!("properties_kind_file").to_string(),
    };
    let size_label = row.size.map(format_size).unwrap_or_else(|| "—".to_string());
    let created_label = row
        .created
        .map(format_date)
        .unwrap_or_else(|| "—".to_string());
    let modified_label = row
        .modified
        .map(format_date)
        .unwrap_or_else(|| "—".to_string());
    let accessed_label = row
        .accessed
        .map(format_date)
        .unwrap_or_else(|| "—".to_string());
    let attrs = {
        let mut parts = Vec::new();
        if row.readonly {
            parts.push(rust_i18n::t!("properties_readonly").to_string());
        }
        if row.is_symlink {
            parts.push(rust_i18n::t!("properties_symlink").to_string());
        }
        if parts.is_empty() {
            "—".to_string()
        } else {
            parts.join(", ")
        }
    };

    v_flex()
        .gap_3()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(file_type_icon(&row.kind, None, 28.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(row.name.clone()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(row.path.display().to_string()),
        )
        .when_some(row.link_target.as_ref(), |body, target| {
            body.child(prop_row(
                rust_i18n::t!("properties_link_target").to_string(),
                target.display().to_string(),
                muted,
                fg,
            ))
        })
        .child(div().h(px(1.)).w_full().bg(muted.opacity(0.2)))
        .child(v_flex().gap(px(6.)).text_xs().children(vec![
            prop_row(
                rust_i18n::t!("properties_type").to_string(),
                kind_label,
                muted,
                fg,
            )
            .into_any_element(),
            prop_row(
                rust_i18n::t!("properties_size").to_string(),
                size_label,
                muted,
                fg,
            )
            .into_any_element(),
            prop_row(
                rust_i18n::t!("properties_created").to_string(),
                created_label,
                muted,
                fg,
            )
            .into_any_element(),
            prop_row(
                rust_i18n::t!("properties_modified").to_string(),
                modified_label,
                muted,
                fg,
            )
            .into_any_element(),
            prop_row(
                rust_i18n::t!("properties_accessed").to_string(),
                accessed_label, muted, fg,
            )
            .into_any_element(),
            prop_row(
                rust_i18n::t!("properties_attributes").to_string(),
                attrs,
                muted,
                fg,
            )
            .into_any_element(),
        ]))
        .into_any_element()
}

fn multi_summary(rows: &[PropertyRow], fg: gpui::Hsla, muted: gpui::Hsla) -> AnyElement {
    let dir_count = rows.iter().filter(|r| r.kind == FileKind::Dir).count();
    let file_count = rows.len() - dir_count;
    let total_size: u64 = rows.iter().filter_map(|r| r.size).sum();

    v_flex()
        .gap_2()
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(rust_i18n::t!("properties_selection_count", count = rows.len()).to_string()),
        )
        .child(div().h(px(1.)).w_full().bg(muted.opacity(0.2)))
        .child(prop_row(
            rust_i18n::t!("properties_files").to_string(),
            file_count.to_string(),
            muted,
            fg,
        ))
        .child(prop_row(
            rust_i18n::t!("properties_folders").to_string(),
            dir_count.to_string(),
            muted,
            fg,
        ))
        .child(prop_row(
            rust_i18n::t!("properties_size").to_string(),
            format_size(total_size),
            muted,
            fg,
        ))
        .into_any_element()
}
