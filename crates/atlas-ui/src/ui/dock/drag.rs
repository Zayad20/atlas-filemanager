use gpui::prelude::*;
use gpui::{px, Context, Render, SharedString, Window, div, Styled};
use gpui_component::{ActiveTheme as _};

use crate::state::layout_tree::TileId;

#[derive(Clone)]
pub(crate) struct TabDrag {
    pub(crate) pane_id: TileId,
    pub(crate) content: gpui::AnyView,
    pub(crate) size: Option<gpui::Size<gpui::Pixels>>,
}

impl Render for TabDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = if let Some(s) = self.size {
            (s.width, s.height)
        } else {
            (px(400.), px(300.))
        };
        div()
            .w(w)
            .h(h)
            .rounded(px(10.))
            .overflow_hidden()
            .shadow_lg()
            .opacity(0.85) // Less translucent to look like a real window
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().accent)
            .child(self.content.clone())
    }
}

pub(crate) fn tab_drag_entity(
    pane_id: TileId,
    content: gpui::AnyView,
    size: Option<gpui::Size<gpui::Pixels>>,
    cx: &mut gpui::App,
) -> gpui::Entity<TabDrag> {
    cx.new(|_| TabDrag { pane_id, content, size })
}
