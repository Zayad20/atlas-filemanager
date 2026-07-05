//! Cross-platform "Properties" overlay — F3 #15.
//!
//! On Windows, right-click → Properties opens the native shell dialog
//! (`shell_context_menu::show_properties`, `ShellExecuteExW` + "properties" verb).
//! Everywhere else that verb doesn't exist, so this is our own themed modal built
//! from the same metadata the inspector already surfaces (name/path/size/modified),
//! plus what the inspector doesn't stat: created time, read-only/symlink flags.
//!
//! Folder sizes are NOT computed recursively here — `std::fs::metadata` reports a
//! directory's own inode size, not its contents, and walking the tree synchronously
//! on open would block the UI thread. Shows "—" for directories; a future pass
//! could compute this via the existing async size-scan used by the paste-conflict
//! flow.

use crate::app::App;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use atlas_core::util::display::{format_date, format_size};
use atlas_model::file_entry::FileKind;
use atlas_model::overlay::{PropertiesState, PropertyRow};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, KeyDownEvent, MouseButton, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme};
use std::path::{Path, PathBuf};

/// Stat `paths` synchronously (a handful of files/dirs — one `metadata()` /
/// `symlink_metadata()` call each) and build the overlay state.
pub(crate) fn stat_properties(paths: &[PathBuf]) -> PropertiesState {
    let rows = paths.iter().map(|path| stat_one(path)).collect::<Vec<_>>();
    PropertiesState { rows }
}

fn stat_one(path: &Path) -> PropertyRow {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let meta = std::fs::metadata(path).ok();
    let is_symlink = std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    let kind = match &meta {
        Some(m) if m.is_dir() => FileKind::Dir,
        _ if is_symlink => FileKind::Symlink,
        _ => FileKind::File,
    };
    PropertyRow {
        name,
        path: path.to_path_buf(),
        kind,
        size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
        created: meta.as_ref().and_then(|m| m.created().ok()),
        modified: meta.as_ref().and_then(|m| m.modified().ok()),
        readonly: meta.as_ref().is_some_and(|m| m.permissions().readonly()),
        is_symlink,
    }
}

impl App {
    #[allow(dead_code)] // reason: only called from the cfg(not(windows)) ctx-menu branch
    pub(crate) fn open_properties(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.state.overlay.properties = Some(stat_properties(&paths));
    }

    fn close_properties(&mut self) {
        self.state.overlay.properties = None;
    }

    pub(crate) fn render_properties(
        &mut self,
        _window: &mut Window,
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

        let body = if let [row] = state.rows.as_slice() {
            single_row(row, fg, muted)
        } else {
            multi_summary(&state.rows, fg, muted)
        };

        let close_view = view.clone();
        let panel = v_flex()
            .w(px(420.))
            .max_h(px(480.))
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
            .child(body);

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
