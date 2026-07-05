use crate::app::App;
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Entity, Hsla, MouseButton, Pixels, SharedString, TextRun, Window};
use gpui_component::input::InputState;

pub(super) const INLINE_INPUT_H: f32 = 20.0;

pub(super) fn active_input(
    app: &App,
    pane_id: TabId,
    entry_idx: usize,
) -> Option<Entity<InputState>> {
    app.rename_edit
        .as_ref()
        .filter(|edit| edit.pane == pane_id && edit.idx == entry_idx)
        .map(|edit| edit.input.clone())
}

pub(super) fn suppress_row_chrome(app: &App, pane_id: TabId, entry_idx: usize) -> bool {
    active_input(app, pane_id, entry_idx).is_some()
        || app
            .rename_visual_suppress
            .is_some_and(|(pane, idx, _)| pane == pane_id && idx == entry_idx)
}

pub(super) fn input_width(name: &str, max: f32, font_size: Pixels, window: &mut Window) -> f32 {
    let text: SharedString = name.to_string().into();
    let style = window.text_style();
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line(text, font_size, &[run], None)
        .width()
        .as_f32();
    (width + 2.0).clamp(72.0, max)
}

pub(super) fn input_element(
    input: &Entity<InputState>,
    view: &Entity<App>,
    height: f32,
    font_size: Pixels,
) -> AnyElement {
    let commit_view = view.clone();
    div()
        .w_full()
        .h(px(height))
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .child(
            crate::ui::text_input::flat(input)
                .h(px(height))
                .px_0()
                .py_0()
                .text_size(font_size)
                .line_height(px(height)),
        )
        .capture_key_down(move |ev, _window, cx| match ev.keystroke.key.as_str() {
            "enter" => {
                commit_view.update(cx, App::commit_rename);
                cx.stop_propagation();
            }
            "escape" => {
                commit_view.update(cx, |this, _cx| this.cancel_rename());
                cx.stop_propagation();
            }
            _ => {}
        })
        .into_any_element()
}
