use gpui::prelude::*;
use gpui::{div, px, Context, Render, SharedString, Window};
use gpui_component::{h_flex, ActiveTheme as _};

use crate::state::layout_tree::TileId;

#[derive(Clone)]
pub(crate) struct TabDrag {
    pub(crate) pane_id: TileId,
}

impl Render for TabDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(cx.theme().accent.opacity(0.18))
            .text_color(cx.theme().foreground)
            .child(SharedString::from(format!("Tab {}", self.pane_id.0)))
    }
}

pub(crate) fn tab_drag_entity(pane_id: TileId, cx: &mut gpui::App) -> gpui::Entity<TabDrag> {
    cx.new(|_| TabDrag { pane_id })
}

pub(crate) fn edge_zone() -> gpui::Div {
    div()
        .absolute()
        .bg(gpui::transparent_black())
        .hover(|s| s.bg(gpui::hsla(0.58, 0.75, 0.52, 0.18)))
}
