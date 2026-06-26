use gpui::prelude::*;
use gpui::{px, Context, Render, SharedString, Window};
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
