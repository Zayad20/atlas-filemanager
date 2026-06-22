use gpui::prelude::*;
use gpui::{div, px, AnyElement, Entity, MouseButton};
use gpui_component::ActiveTheme as _;

use crate::app::{App, DockResizeState};
use crate::state::layout_tree::LinearDir;
use crate::state::layout_tree::TileId;

pub(crate) fn splitter(
    container: TileId,
    divider: usize,
    dir: LinearDir,
    view: &Entity<App>,
    cx: &mut gpui::App,
) -> AnyElement {
    let border = cx.theme().border.opacity(0.7);
    let view = view.clone();
    let el = match dir {
        LinearDir::Horizontal => div()
            .w(px(5.))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .child(div().w(px(1.)).h_full().mx(px(2.)).bg(border)),
        LinearDir::Vertical => div()
            .h(px(5.))
            .w_full()
            .flex_none()
            .cursor_row_resize()
            .child(div().h(px(1.)).w_full().my(px(2.)).bg(border)),
    };
    el.on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
        view.update(cx, |this, _cx| {
            let Some(bounds) = this.dock_linear_bounds.get(&container).copied() else {
                return;
            };
            let Some(crate::state::layout_tree::Tile::Container(
                crate::state::layout_tree::Container::Linear(linear),
            )) = this.state.pane_tree.tiles.get(container)
            else {
                return;
            };
            let span_px = match dir {
                LinearDir::Horizontal => f32::from(bounds.size.width),
                LinearDir::Vertical => f32::from(bounds.size.height),
            };
            this.dock_resize = Some(DockResizeState {
                container,
                divider,
                dir,
                start_pos: match dir {
                    LinearDir::Horizontal => f32::from(ev.position.x),
                    LinearDir::Vertical => f32::from(ev.position.y),
                },
                span_px,
                start_shares: linear.shares.clone(),
            });
        });
        cx.stop_propagation();
    })
    .into_any_element()
}
