use std::path::PathBuf;
use std::time::SystemTime;

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

/// Hover card for a file/folder row.
///
/// Tooltips render on the UI thread, alongside pointer and scroll event handling.
/// Only use information already captured by the row here: filesystem calls such as
/// `read_dir` and `metadata` can block on large folders, network shares, or removable
/// drives, making hover and scrolling appear to update in bursts.
pub(crate) fn entry_hover_tooltip(
    name: impl Into<SharedString>,
    _path: PathBuf,
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

        if !is_dir {
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
