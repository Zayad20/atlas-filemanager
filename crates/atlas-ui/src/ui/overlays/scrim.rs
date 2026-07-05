//! Shared overlay scrim/backdrop builders for the bespoke modals.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use gpui::prelude::*;
use gpui::{
    deferred, div, px, AnyElement, Context, Entity, FocusHandle, Hsla, MouseButton, SharedString,
};
use gpui_component::v_flex;

impl App {
    /// Full-window backdrop + centered panel, closing via a fixed `Action` on
    /// backdrop click. `top` positions the panel near the top (fuzzy bars) vs center.
    #[allow(clippy::unused_self)] // method form for call-site symmetry with other overlay_* builders
    pub(crate) fn overlay_scrim(
        &mut self,
        view: Entity<Self>,
        close_action: Action,
        _focus: FocusHandle,
        panel: AnyElement,
        top: bool,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: SharedString = match &close_action {
            Action::ClosePalette => "scrim-palette".into(),
            Action::ResolveTransferConflict(_) => "scrim-paste".into(),
            _ => "scrim-goto".into(),
        };
        let v = view;
        let scrim = scrim_base(id, top)
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                let close = close_action.clone();
                v.update(cx, |this, cx| {
                    this.process_actions(vec![close]);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }

    /// Variant whose close runs an arbitrary mutation (for overlays without a
    /// dedicated close `Action`, e.g. settings / batch rename).
    #[allow(clippy::unused_self)] // method form for call-site symmetry with other overlay_* builders
    pub(crate) fn overlay_scrim_custom(
        &mut self,
        id: &'static str,
        view: Entity<Self>,
        close: impl Fn(&mut Self) + 'static,
        _focus: FocusHandle,
        panel: AnyElement,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let scrim = scrim_base(SharedString::from(id), false)
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                view.update(cx, |this, cx| {
                    close(this);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }

    /// Variant for right-side sheets: the backdrop does NOT center its child.
    /// The child is expected to carry its own `absolute().right_0().top_0().h_full()`
    /// to anchor itself to the right edge of the window.
    #[allow(clippy::unused_self)]
    pub(crate) fn overlay_scrim_right_sheet(
        &mut self,
        id: &'static str,
        view: Entity<Self>,
        close: impl Fn(&mut Self) + 'static,
        _focus: FocusHandle,
        panel: AnyElement,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let scrim = div()
            .id(SharedString::from(id))
            .absolute()
            .inset_0()
            .size_full()
            .bg(scrim_color())
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                view.update(cx, |this, cx| {
                    close(this);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }
}

/// The dimmed, click-catching backdrop that fills the window and centers its child.
fn scrim_base(id: SharedString, top: bool) -> gpui::Stateful<gpui::Div> {
    let mut d = v_flex()
        .id(id)
        .absolute()
        .inset_0()
        .size_full()
        .items_center()
        .bg(scrim_color());
    if top {
        d = d.pt(px(48.));
    } else {
        d = d.justify_center();
    }
    d
}

fn scrim_color() -> Hsla {
    gpui::hsla(0., 0., 0., 0.45)
}
