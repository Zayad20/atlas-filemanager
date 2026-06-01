use crate::input::action::Action;
use crate::state::app_state::AppState;
use egui::{Key, KeyboardShortcut, Modifiers, PointerButton};

pub fn handle_global_shortcuts(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
    if ctx.wants_keyboard_input() {
        return;
    }
    ctx.input_mut(|i| {
        // Mouse back/forward buttons
        if i.pointer.button_clicked(PointerButton::Extra1) {
            actions.push(Action::NavigateBack(state.focused_tile));
        }
        if i.pointer.button_clicked(PointerButton::Extra2) {
            actions.push(Action::NavigateForward(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::T)) {
            actions.push(Action::NewTabFocused);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::W)) {
            actions.push(Action::CloseTab(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::P,
        )) {
            actions.push(Action::OpenPalette);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::D,
        )) {
            actions.push(Action::ToggleDropShelf);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::P)) {
            actions.push(Action::OpenGotoBar);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::L)) {
            actions.push(Action::EditLocation(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::I)) {
            actions.push(Action::ToggleInspector);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::B)) {
            actions.push(Action::ToggleSidebar);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Period)) {
            actions.push(Action::ToggleHidden);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::Delete)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Delete {
                    paths,
                    to_trash: true,
                });
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::SHIFT, Key::Delete)) {
            let paths = selected_paths(state);
            if !paths.is_empty() {
                actions.push(Action::Delete {
                    paths,
                    to_trash: false,
                });
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::A)) {
            actions.push(Action::SelectAll);
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
                actions.push(Action::Paste {
                    dest: tab.path.clone(),
                });
            }
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Comma)) {
            actions.push(Action::OpenSettings);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft)) {
            actions.push(Action::NavigateBack(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight)) {
            actions.push(Action::NavigateForward(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp)) {
            actions.push(Action::NavigateUp(state.focused_tile));
        }
        if i.consume_shortcut(&KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::Z,
        )) {
            actions.push(Action::Redo);
        }
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Z)) {
            actions.push(Action::Undo);
        }
        if i.key_pressed(Key::Escape) {
            actions.push(Action::CloseOverlays);
        }
        // Space toggles the preview inspector panel
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::Space)) {
            actions.push(Action::ToggleInspector);
        }
    });
}

fn selected_paths(state: &AppState) -> Vec<std::path::PathBuf> {
    state
        .active_tab_state()
        .map(|tab| {
            tab.selected
                .iter()
                .filter_map(|idx| tab.entries.get(*idx))
                .map(|entry| entry.path.clone())
                .collect()
        })
        .unwrap_or_default()
}
