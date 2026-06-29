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
