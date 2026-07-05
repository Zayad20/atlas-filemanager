use crate::app::App;
use crate::input::action::Action;
use atlas_model::profiles::ProfileScoped;
use atlas_model::settings::ViewMode;
use gpui::{Context, KeyDownEvent, KeybindingKeystroke, Keystroke, Window};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CommandShortcutPrefs {
    #[serde(default)]
    bindings: HashMap<String, Option<String>>,
}

pub(crate) struct CommandShortcuts;

impl ProfileScoped for CommandShortcuts {
    const KEY: &'static str = "command_shortcuts";
    type State = CommandShortcutPrefs;
}

const COMMAND_SHORTCUTS: &[(&str, &str)] = &[
    ("tab.new", "ctrl-t"),
    ("tab.close", "ctrl-w"),
    ("pane.split-h", "ctrl-shift-s"),
    ("pane.split-v", "ctrl-alt-s"),
    ("pane.close", "ctrl-shift-w"),
    ("nav.back", "alt-left"),
    ("nav.forward", "alt-right"),
    ("nav.goto", "ctrl-k"),
    ("view.hidden", "ctrl-."),
    ("view.inspector", "ctrl-i"),
    ("view.sidebar", "ctrl-b"),
    ("view.zoom-in", "ctrl-="),
    ("view.zoom-out", "ctrl--"),
    ("view.zoom-reset", "ctrl-0"),
    ("edit.select-all", "ctrl-a"),
    ("edit.undo", "ctrl-z"),
    ("edit.redo", "ctrl-shift-z"),
    ("files.trash", "delete"),
    ("files.delete", "shift-delete"),
    ("ext.manager", "ctrl-shift-x"),
    ("sys.settings", "ctrl-,"),
];

const RESERVED_STATIC_SHORTCUTS: &[&str] = &[
    "ctrl-q",
    "up",
    "down",
    "shift-up",
    "shift-down",
    "home",
    "end",
    "pageup",
    "pagedown",
    "enter",
    "f2",
    "ctrl-tab",
    "ctrl-shift-tab",
    "ctrl-shift-p",
    "ctrl-f",
    "space",
    "escape",
    "alt-tab",
    "f5",
    "ctrl-r",
    "ctrl-l",
    "alt-up",
    "ctrl-c",
    "ctrl-x",
    "ctrl-v",
];

pub(crate) fn default_command_shortcut(command_id: &str) -> Option<&'static str> {
    COMMAND_SHORTCUTS
        .iter()
        .find_map(|(id, key)| (*id == command_id).then_some(*key))
}

pub(crate) fn has_default_command_shortcut(command_id: &str) -> bool {
    default_command_shortcut(command_id).is_some()
}

impl App {
    pub(crate) fn handle_global_command_shortcut_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.command_shortcut_capture.is_some() {
            return self.handle_command_shortcut_key(ev);
        }

        let text_input_focused = self.any_text_input_focused(window, cx);
        let pane_filter_active = self
            .state
            .active_tab_state()
            .is_some_and(|ts| ts.search.active);
        let app_modified = ev.keystroke.modifiers.control
            || ev.keystroke.modifiers.alt
            || ev.keystroke.modifiers.platform;

        if text_input_focused && !(pane_filter_active && app_modified) {
            return false;
        }

        if self
            .state
            .active_tab_state()
            .is_some_and(|ts| ts.path_edit.editing)
        {
            return false;
        }

        self.handle_command_shortcut_key(ev)
    }

    pub(crate) fn command_shortcut_string(&self, command_id: &str) -> Option<String> {
        let prefs = self.state.profiles.active().scoped::<CommandShortcuts>();
        match prefs.bindings.get(command_id) {
            Some(Some(keys)) if Keystroke::parse(keys).is_ok() => Some(keys.clone()),
            Some(Some(_)) | Some(None) => None,
            None => default_command_shortcut(command_id).map(str::to_string),
        }
    }

    pub(crate) fn command_shortcut_keystroke(&self, command_id: &str) -> Option<Keystroke> {
        self.command_shortcut_string(command_id)
            .and_then(|keys| Keystroke::parse(&keys).ok())
    }

    pub(crate) fn start_command_shortcut_capture(&mut self, command_id: String) {
        if has_default_command_shortcut(&command_id) {
            self.command_shortcut_capture = Some(command_id);
        }
    }

    pub(crate) fn cancel_command_shortcut_capture(&mut self) {
        self.command_shortcut_capture = None;
    }

    pub(crate) fn set_command_shortcut(&mut self, command_id: String, keystroke: String) {
        if !has_default_command_shortcut(&command_id) || Keystroke::parse(&keystroke).is_err() {
            return;
        }
        if reserved_static_shortcut(&keystroke) {
            self.toast_error(rust_i18n::t!("palette_shortcut_reserved").to_string());
            self.command_shortcut_capture = None;
            return;
        }

        let mut prefs = self.state.profiles.active().scoped::<CommandShortcuts>();
        for (other_id, _) in COMMAND_SHORTCUTS {
            if *other_id != command_id
                && self
                    .command_shortcut_string(other_id)
                    .is_some_and(|keys| keys == keystroke)
            {
                prefs.bindings.insert((*other_id).to_string(), None);
            }
        }
        prefs.bindings.insert(command_id, Some(keystroke));
        self.state
            .profiles
            .active_mut()
            .set_scoped::<CommandShortcuts>(&prefs);
        atlas_core::util::config::save_profiles(&self.state.profiles);
        self.command_shortcut_capture = None;
    }

    pub(crate) fn remove_command_shortcut(&mut self, command_id: String) {
        if !has_default_command_shortcut(&command_id) {
            return;
        }
        let mut prefs = self.state.profiles.active().scoped::<CommandShortcuts>();
        prefs.bindings.insert(command_id, None);
        self.state
            .profiles
            .active_mut()
            .set_scoped::<CommandShortcuts>(&prefs);
        atlas_core::util::config::save_profiles(&self.state.profiles);
        self.command_shortcut_capture = None;
    }

    pub(crate) fn handle_command_shortcut_key(&mut self, ev: &KeyDownEvent) -> bool {
        if let Some(command_id) = self.command_shortcut_capture.clone() {
            match ev.keystroke.key.as_str() {
                "escape" => self.cancel_command_shortcut_capture(),
                "backspace" | "delete" => self.remove_command_shortcut(command_id),
                _ => self.set_command_shortcut(command_id, ev.keystroke.unparse()),
            }
            return true;
        }

        if self.command_shortcuts_blocked_by_app_state() {
            return false;
        }

        for (command_id, _) in COMMAND_SHORTCUTS {
            let Some(keys) = self.command_shortcut_string(command_id) else {
                continue;
            };
            let Ok(target) = Keystroke::parse(&keys) else {
                continue;
            };
            let target = KeybindingKeystroke::from_keystroke(target);
            if ev.keystroke.should_match(&target) {
                if let Some(action) = self.action_for_command_shortcut(command_id) {
                    self.process_actions(vec![action]);
                }
                return true;
            }
        }

        false
    }

    fn action_for_command_shortcut(&self, command_id: &str) -> Option<Action> {
        let focused = self.state.focused_tile;
        match command_id {
            "tab.new" => Some(Action::NewTabFocused),
            "tab.close" => Some(Action::CloseFocusedTab),
            "pane.split-h" => Some(Action::SplitHorizontalFocused),
            "pane.split-v" => Some(Action::SplitVerticalFocused),
            "pane.close" => Some(Action::CloseFocusedPane),
            "nav.back" => Some(Action::NavigateBack(focused)),
            "nav.forward" => Some(Action::NavigateForward(focused)),
            "nav.goto" => Some(Action::OpenGotoBar),
            "view.hidden" => Some(Action::ToggleHidden),
            "view.inspector" => Some(Action::ToggleInspector),
            "view.sidebar" => Some(Action::ToggleSidebar),
            "view.details" => Some(Action::SetViewMode(ViewMode::Details)),
            "view.grid" => Some(Action::SetViewMode(ViewMode::Grid)),
            "view.list" => Some(Action::SetViewMode(ViewMode::List)),
            "view.miller" => Some(Action::SetViewMode(ViewMode::Miller)),
            "view.zoom-in" => Some(Action::ZoomIn),
            "view.zoom-out" => Some(Action::ZoomOut),
            "view.zoom-reset" => Some(Action::ZoomReset),
            "edit.select-all" => Some(Action::SelectAll),
            "edit.undo" => Some(Action::Undo),
            "edit.redo" => Some(Action::Redo),
            "files.trash" => Some(Action::Delete {
                paths: self.selected_paths(),
                to_trash: true,
            }),
            "files.delete" => Some(Action::Delete {
                paths: self.selected_paths(),
                to_trash: false,
            }),
            "ext.manager" => Some(Action::OpenExtManager),
            "sys.settings" => Some(Action::OpenSettings),
            _ => None,
        }
    }

    fn command_shortcuts_blocked_by_app_state(&self) -> bool {
        let o = &self.state.overlay;
        o.show_goto_bar
            || self.palette_closing.is_some()
            || o.show_palette
            || o.show_ext_palette
            || o.show_settings
            || o.show_analyzer
            || o.transfer_conflict.is_some()
            || o.renaming_profile
            || self.state.profile_ui.creating_tag
            || self.state.batch_rename.is_some()
            || self.rename_edit.is_some()
            || self.ctx_menu.is_some()
            || self.type_filter_menu.is_some()
            || self.size_filter_menu.is_some()
            || self.tag_picker_menu.is_some()
            || self.state.ext.show_manager
            || {
                #[cfg(feature = "cloud")]
                {
                    self.state.cloud.show_add_dialog
                        || self.state.cloud.renaming_remote_id.is_some()
                }
                #[cfg(not(feature = "cloud"))]
                {
                    false
                }
            }
    }
}

fn reserved_static_shortcut(keystroke: &str) -> bool {
    let Ok(source) = Keystroke::parse(keystroke) else {
        return true;
    };
    RESERVED_STATIC_SHORTCUTS.iter().any(|reserved| {
        Keystroke::parse(reserved)
            .map(KeybindingKeystroke::from_keystroke)
            .is_ok_and(|target| source.should_match(&target))
    })
}
