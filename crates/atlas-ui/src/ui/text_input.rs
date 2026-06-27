//! Shared single-line text input built on `gpui-component`'s `InputState`/`TextInput`.
//!
//! The fuzzy overlays, batch rename fields and inline rename all used to roll their
//! own "capture keys → push into a String" widget, which had no cursor, no selection
//! and explicitly swallowed `ctrl`/`cmd` (so paste/copy/select-all never worked) and
//! was duplicated three times. This module centralizes creation on the library's
//! production input, so every field gets real editing + clipboard (ctrl/cmd a,c,v,x)
//! for free, cross-platform, with one code path.

use crate::app::App;
use gpui::{AppContext, Context, Entity, SharedString, Window};
use gpui_component::input::{Input, InputState};

/// Create a fresh single-line input, optionally pre-filled. Bound to `App`'s context
/// so the caller can hold the returned `Entity` on `App` and read `.value()` each frame.
pub fn single_line(
    window: &mut Window,
    cx: &mut Context<App>,
    placeholder: impl Into<String>,
    initial: impl Into<String>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let initial = initial.into();
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);
        if !initial.is_empty() {
            state.set_value(initial, window, cx);
        }
        state
    })
}

/// Render a borderless input that blends into a surrounding row (the fuzzy search row
/// already draws its own border/background).
pub fn flat(state: &Entity<InputState>) -> Input {
    Input::new(state).appearance(false)
}

/// Render a bordered input for standalone form fields (batch rename, inline rename).
pub fn bordered(state: &Entity<InputState>) -> Input {
    Input::new(state)
}

/// Create a multi-line code-editor `InputState` pre-filled with `initial` text.
/// `language` is the tree-sitter language token (e.g. `"rust"`, `"python"`).
/// `line_numbers` controls the initial line-number gutter visibility.
///
/// Callers own the returned `Entity<InputState>`; subscribe to
/// `gpui_component::input::InputEvent::Change` to track edits.
pub fn code_editor(
    window: &mut Window,
    cx: &mut Context<App>,
    language: impl Into<SharedString>,
    initial: &str,
    line_numbers: bool,
) -> Entity<InputState> {
    let language = language.into();
    let text = initial.to_string();
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .code_editor(language)
            .line_number(line_numbers);
        if !text.is_empty() {
            state.set_value(text, window, cx);
        }
        state
    })
}
