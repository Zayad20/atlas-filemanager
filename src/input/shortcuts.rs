use crate::input::action::Action;
use crate::state::app_state::AppState;
use egui::{Key, KeyboardShortcut, Modifiers, PointerButton};

pub fn handle_global_shortcuts(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
    // Mouse back/forward buttons. These must work regardless of keyboard focus:
    // navigating with the mouse should never be blocked just because a TextEdit
    // (location bar, goto bar, search…) currently holds focus. We also use
    // `button_pressed` (the press edge) rather than `button_clicked`, which needs a
    // clean press+release cycle and is easily missed for the side mouse buttons.
    let mut nav = false;
    ctx.input(|i| {
        if i.pointer.button_pressed(PointerButton::Extra1) {
            actions.push(Action::NavigateBack(state.focused_tile));
            nav = true;
        }
        if i.pointer.button_pressed(PointerButton::Extra2) {
            actions.push(Action::NavigateForward(state.focused_tile));
            nav = true;
        }
    });
    if nav {
        // Guarantee the navigation + async dir-load frames get processed even in
        // reactive repaint mode.
        ctx.request_repaint();
    }

    // Keyboard shortcuts below are suppressed while a text widget has focus, so that
    // e.g. Ctrl+C copies text inside the field instead of files.
    if ctx.wants_keyboard_input() {
        return;
    }
    ctx.input_mut(|i| {
        // 1. Static Actions (No state required)
        let static_shortcuts = [
            (Modifiers::CTRL, Key::T, Action::NewTabFocused),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::P, Action::OpenPalette),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::D, Action::ToggleDropShelf),
            (Modifiers::CTRL, Key::P, Action::OpenGotoBar),
            (Modifiers::CTRL, Key::I, Action::ToggleInspector),
            (Modifiers::CTRL, Key::B, Action::ToggleSidebar),
            (Modifiers::CTRL, Key::Period, Action::ToggleHidden),
            (Modifiers::CTRL, Key::A, Action::SelectAll),
            (Modifiers::CTRL, Key::Comma, Action::OpenSettings),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::W, Action::CloseFocusedPane),
            (Modifiers::ALT, Key::Tab, Action::FocusNextPane),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::S, Action::SplitHorizontalFocused),
            (Modifiers::CTRL | Modifiers::ALT, Key::S, Action::SplitVerticalFocused),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::Z, Action::Redo),
            (Modifiers::CTRL, Key::Z, Action::Undo),
            (Modifiers::NONE, Key::Space, Action::ToggleInspector),
        ];

        for (modifiers, key, action) in static_shortcuts {
            if i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)) {
                actions.push(action.clone());
            }
        }

        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::F5)) {
            actions.push(Action::Refresh(state.focused_tile));
        }
        if i.key_pressed(Key::Escape) {
            actions.push(Action::CloseOverlays);
        }

        // 2. Tab-Scoped Actions (Require focused_tile)
        let tab_shortcuts = [
            (Modifiers::CTRL, Key::W, Action::CloseTab as fn(crate::state::pane::TabId) -> Action),
            (Modifiers::CTRL, Key::L, Action::EditLocation as fn(crate::state::pane::TabId) -> Action),
            (Modifiers::ALT, Key::ArrowLeft, Action::NavigateBack as fn(crate::state::pane::TabId) -> Action),
            (Modifiers::ALT, Key::ArrowRight, Action::NavigateForward as fn(crate::state::pane::TabId) -> Action),
            (Modifiers::ALT, Key::ArrowUp, Action::NavigateUp as fn(crate::state::pane::TabId) -> Action),
            (Modifiers::CTRL, Key::R, Action::Refresh as fn(crate::state::pane::TabId) -> Action),
        ];

        for (modifiers, key, constructor) in tab_shortcuts {
            if i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)) {
                actions.push(constructor(state.focused_tile));
            }
        }

        // 3. Complex Actions (Require selection paths or active tab paths)
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::Delete)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Delete { paths, to_trash: true });
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::SHIFT, Key::Delete)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Delete { paths, to_trash: false });
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::C)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Copy(paths));
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::X)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Cut(paths));
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::V)) {
            if let Some(tab) = state.active_tab_data() {
                actions.push(Action::Paste { dest: tab.path.clone() });
            }
        }
    });
}

fn selected_paths(state: &AppState) -> Vec<std::path::PathBuf> {
    state
        .active_tab_state()
        .map(|tab| {
            tab.selection.selected
                .iter()
                .filter_map(|idx| tab.entries.get(*idx))
                .map(|entry| entry.path.clone())
                .collect()
        })
        .unwrap_or_default()
}
