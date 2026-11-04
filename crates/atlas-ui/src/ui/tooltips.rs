use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use atlas_core::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{div, px, AnyView, App, Context, Keystroke, SharedString, Task, Window};
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

const ENTRY_TOOLTIP_LIFETIME: Duration = Duration::from_secs(5);

/// Keep the native hover session alive after expiry so a stationary pointer
/// cannot repeatedly show the card. Leaving and returning creates a new session.
struct EntryHoverTooltip {
    content: Option<AnyView>,
    _expiry: Task<()>,
}

impl EntryHoverTooltip {
    fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let expiry = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(ENTRY_TOOLTIP_LIFETIME).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.content = None;
                cx.notify();
                window.refresh();
            });
        });
        Self {
            content: Some(content),
            _expiry: expiry,
        }
    }
}

impl Render for EntryHoverTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().children(self.content.clone())
    }
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

    let content = Tooltip::element(move |_window, cx| {
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
    .build(window, cx);
    cx.new(|cx| EntryHoverTooltip::new(content, window, cx))
        .into()
}

#[cfg(all(test, feature = "ui-lab"))]
mod tests {
    use super::*;

    struct Content;

    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child("License.txt")
        }
    }

    #[gpui::test]
    fn file_tooltip_expires_without_pointer_movement(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|window, cx| {
            let content = cx.new(|_| Content).into();
            EntryHoverTooltip::new(content, window, cx)
        });
        cx.run_until_parked();
        cx.dispatcher
            .advance_clock(ENTRY_TOOLTIP_LIFETIME - Duration::from_millis(1));
        cx.run_until_parked();
        window
            .update(cx, |tooltip, _, _| assert!(tooltip.content.is_some()))
            .unwrap();

        cx.dispatcher.advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        window
            .update(cx, |tooltip, _, _| assert!(tooltip.content.is_none()))
            .unwrap();

        // Ordinary redraws and more idle time must not restart the hover card.
        cx.dispatcher.advance_clock(ENTRY_TOOLTIP_LIFETIME);
        cx.run_until_parked();
        window
            .update(cx, |tooltip, window, cx| {
                assert!(tooltip.content.is_none());
                window.refresh();
                let _ = tooltip.render(window, cx);
                assert!(tooltip.content.is_none());
            })
            .unwrap();
    }
}
