use std::path::PathBuf;
use std::time::SystemTime;

use atlas_core::fs::list::folder_hover_stats;
use atlas_core::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{div, px, AnyView, App, Keystroke, SharedString, Window};
use gpui_component::kbd::Kbd;
use gpui_component::tooltip::Tooltip;
use gpui_component::{h_flex, v_flex, ActiveTheme as _};

pub(crate) fn shortcut_tooltip(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    shortcuts: &'static [&'static str],
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let title = title.into();
    let description = description.into();

    Tooltip::element(move |_window, cx| {
        h_flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(div().text_sm().child(title.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description.clone()),
                    ),
            )
            .when(!shortcuts.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .children(shortcuts.iter().filter_map(|shortcut| {
                            Keystroke::parse(shortcut)
                                .ok()
                                .map(|stroke| Kbd::new(stroke).outline().into_any_element())
                        })),
                )
            })
    })
    .build(window, cx)
}

/// Explorer-style hover card for a file/folder row. Folders show
/// Created/Subfolders/Subfiles/Size (computed lazily via a non-recursive `read_dir`);
/// files show Created/Modified/Size using metadata already known by the caller plus
/// one cheap `fs::metadata` call for the creation time. Everything runs inside the
/// tooltip closure, only while actually hovering — no background job needed.
pub(crate) fn entry_hover_tooltip(
    name: impl Into<SharedString>,
    path: PathBuf,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let name = name.into();

    Tooltip::element(move |_window, cx| {
        let muted = cx.theme().muted_foreground;
        let col = v_flex().gap_1().min_w(px(200.)).child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(name.clone()),
        );

        let mut rows = v_flex().text_xs().gap(px(2.)).mt_1();

        if is_dir {
            if let Some(stats) = folder_hover_stats(&path) {
                let count = |n: usize| {
                    if n == 0 {
                        "--".to_string()
                    } else {
                        n.to_string()
                    }
                };
                if let Some(created) = stats.created {
                    rows = rows.child(div().text_color(muted).child(
                        rust_i18n::t!("hover_created", date = format_date(created)).to_string(),
                    ));
                }
                rows = rows
                    .child(
                        div().text_color(muted).child(
                            rust_i18n::t!("hover_subfolders", count = count(stats.subfolders))
                                .to_string(),
                        ),
                    )
                    .child(div().text_color(muted).child(
                        rust_i18n::t!("hover_subfiles", count = count(stats.subfiles)).to_string(),
                    ))
                    .child(div().text_color(muted).child(
                        rust_i18n::t!("hover_size", size = format_size(stats.size)).to_string(),
                    ));
            }
        } else {
            let created = std::fs::metadata(&path).ok().and_then(|m| m.created().ok());
            if let Some(created) = created {
                rows = rows.child(div().text_color(muted).child(
                    rust_i18n::t!("hover_created", date = format_date(created)).to_string(),
                ));
            }
            if let Some(modified) = modified {
                rows = rows.child(div().text_color(muted).child(
                    rust_i18n::t!("inspector_modified", date = format_date(modified)).to_string(),
                ));
            }
            if let Some(size) = size {
                rows = rows.child(
                    div()
                        .text_color(muted)
                        .child(rust_i18n::t!("hover_size", size = format_size(size)).to_string()),
                );
            }
        }

        col.child(rows)
    })
    .build(window, cx)
}
