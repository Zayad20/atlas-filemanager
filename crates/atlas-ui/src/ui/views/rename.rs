use crate::app::App;
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{div, AnyElement, Entity, Hsla, MouseButton, Pixels, SharedString, TextRun, Window};
use gpui_component::input::InputState;
use std::path::Path;

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

pub(super) fn active_input_for_path(
    app: &App,
    pane_id: TabId,
    path: &Path,
) -> Option<Entity<InputState>> {
    app.rename_edit
        .as_ref()
        .filter(|edit| edit.pane == pane_id && edit.path == path)
        .map(|edit| edit.input.clone())
}

pub(super) fn suppress_row_chrome(app: &App, pane_id: TabId, entry_idx: usize) -> bool {
    active_input(app, pane_id, entry_idx).is_some()
        || app
            .rename_visual_suppress
            .is_some_and(|(pane, idx, _)| pane == pane_id && idx == entry_idx)
}

pub(super) fn dim_other_entry(app: &App, pane_id: TabId, entry_idx: usize) -> bool {
    app.rename_edit
        .as_ref()
        .is_some_and(|edit| edit.pane == pane_id && edit.idx != entry_idx)
}

pub(super) fn dim_other_path(app: &App, pane_id: TabId, path: &Path) -> bool {
    app.rename_edit
        .as_ref()
        .is_some_and(|edit| edit.pane == pane_id && edit.path != path)
}

/// `max_px` and the return value are real pixels — callers pass an already
/// zoom-scaled bound (`font_size` likewise already reflects the current zoom).
/// Only the fixed lower bound (72px at zoom 1) is scaled internally, since it
/// has no other source to inherit the scale from.
pub(super) fn input_width(name: &str, max_px: f32, font_size: Pixels, window: &mut Window) -> f32 {
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
    let scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
    (width + 2.0).clamp(72.0 * scale, max_px)
}

/// `height` is the exact real-pixel box the plain (non-editing) name text
/// occupies at this row — callers pass `window.line_height()` (list/grid) or
/// the row's own already-scaled height (details), never a separate guessed
/// constant. Matching it exactly is what keeps the input from jumping a few
/// px relative to the static text it replaces when rename starts/ends.
pub(super) fn input_element(
    input: &Entity<InputState>,
    view: &Entity<App>,
    height: Pixels,
    font_size: Pixels,
    window: &mut Window,
) -> AnyElement {
    let commit_view = view.clone();
    div()
        .w_full()
        .h(height)
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .child(
            crate::ui::text_input::flat(input)
                .min_h(height)
                .max_h(height)
                .px_0()
                .py_0()
                .text_size(font_size)
                .line_height(height),
        )
        .capture_action(window.listener_for(view, App::rename_next_action))
        .capture_action(window.listener_for(view, App::rename_previous_action))
        .capture_key_down(move |ev, window, cx| match ev.keystroke.key.as_str() {
            "enter" => {
                commit_view.update(cx, App::commit_rename);
                cx.stop_propagation();
            }
            "tab" => {
                let forward = !ev.keystroke.modifiers.shift;
                commit_view.update(cx, |this, cx| {
                    this.commit_rename_and_move(forward, window, cx)
                });
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
