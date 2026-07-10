use crate::app::App;
use crate::input::action::{Action, SelectionMove};
use crate::input::commands::{command_remappable, default_command_shortcut, COMMANDS};
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
        if command_remappable(&command_id) {
            self.command_shortcut_capture = Some(command_id);
        }
    }

    pub(crate) fn cancel_command_shortcut_capture(&mut self) {
        self.command_shortcut_capture = None;
    }

    pub(crate) fn set_command_shortcut(&mut self, command_id: String, keystroke: String) {
        if !command_remappable(&command_id) || Keystroke::parse(&keystroke).is_err() {
            return;
        }
        if reserved_static_shortcut(&keystroke) {
            self.toast_error(rust_i18n::t!("palette_shortcut_reserved").to_string());
            self.command_shortcut_capture = None;
            return;
        }

        let mut prefs = self.state.profiles.active().scoped::<CommandShortcuts>();
        for other in COMMANDS.iter().filter(|command| command.remappable) {
            if other.id != command_id
                && self
                    .command_shortcut_string(other.id)
                    .is_some_and(|keys| keys == keystroke)
            {
                prefs.bindings.insert(other.id.to_string(), None);
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
        if !command_remappable(&command_id) {
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

        for command in COMMANDS.iter().filter(|command| command.remappable) {
            let Some(keys) = self.command_shortcut_string(command.id) else {
                continue;
            };
            let Ok(target) = Keystroke::parse(&keys) else {
                continue;
            };
            let target = KeybindingKeystroke::from_keystroke(target);
            if ev.keystroke.should_match(&target) {
                if let Some(action) = self.action_for_command_shortcut(command.id) {
                    self.process_actions(vec![action]);
                }
                return true;
            }
        }

        false
    }

    fn action_for_command_shortcut(&self, command_id: &str) -> Option<Action> {
        let focused = self.state.focused_tile;
        let active_location = self.state.active_tab_data().map(|tab| &tab.location);
        let archive_focused =
            active_location.is_some_and(atlas_model::location::Location::is_archive);
        let recycle_focused = active_location
            .is_some_and(|location| *location == atlas_model::location::Location::RecycleBin);
        match command_id {
            "tab.new" => Some(Action::NewTabFocused),
            "tab.close" => Some(Action::CloseFocusedTab),
            "tab.next" => Some(Action::FocusNextTab),
            "tab.previous" => Some(Action::FocusPreviousTab),
            "tab.move-left" => Some(Action::MoveFocusedTabLeft),
            "tab.move-right" => Some(Action::MoveFocusedTabRight),
            "tab.1" => Some(Action::FocusTabIndex(0)),
            "tab.2" => Some(Action::FocusTabIndex(1)),
            "tab.3" => Some(Action::FocusTabIndex(2)),
            "tab.4" => Some(Action::FocusTabIndex(3)),
            "tab.5" => Some(Action::FocusTabIndex(4)),
            "tab.6" => Some(Action::FocusTabIndex(5)),
            "tab.7" => Some(Action::FocusTabIndex(6)),
            "tab.8" => Some(Action::FocusTabIndex(7)),
            "tab.9" => Some(Action::FocusTabIndex(8)),
            "pane.split-h" => Some(Action::SplitHorizontalFocused),
            "pane.split-v" => Some(Action::SplitVerticalFocused),
            "pane.close" => Some(Action::CloseFocusedPane),
            "nav.back" => Some(Action::NavigateBack(focused)),
            "nav.forward" => Some(Action::NavigateForward(focused)),
            "nav.parent" => Some(Action::NavigateUp(focused)),
            "nav.goto" => Some(Action::OpenGotoBar),
            "nav.refresh" => Some(Action::Refresh(focused)),
            #[cfg(target_os = "windows")]
            "nav.recycle-bin" => Some(Action::NavigateLocation {
                tab: focused,
                location: atlas_model::location::Location::RecycleBin,
            }),
            "view.hidden" => Some(Action::ToggleHidden),
            "view.inspector" => Some(Action::ToggleInspector),
            "view.sidebar" => Some(Action::ToggleSidebar),
            "view.sidebar-collapse" => Some(Action::ToggleSidebarCollapsed),
            "view.details" => Some(Action::SetViewMode(ViewMode::Details)),
            "view.grid" => Some(Action::SetViewMode(ViewMode::Grid)),
            "view.list" => Some(Action::SetViewMode(ViewMode::List)),
            "view.miller" => Some(Action::SetViewMode(ViewMode::Miller)),
            "view.zoom-in" => Some(Action::ZoomIn),
            "view.zoom-out" => Some(Action::ZoomOut),
            "view.zoom-reset" => Some(Action::ZoomReset),
            "view.content-zoom-in" => Some(Action::PaneContentZoomIn),
            "view.content-zoom-out" => Some(Action::PaneContentZoomOut),
            "folder.toggle-folders-first" => Some(Action::ToggleFoldersFirstFocused),
            "folder.toggle-files" => Some(Action::ToggleShowFilesFocused),
            "folder.toggle-folders" => Some(Action::ToggleShowFoldersFocused),
            "folder.search" => Some(Action::OpenSearchInPlace),
            "folder.search-subfolders" => Some(Action::ToggleSearchInSubfolders),
            "selection.previous" => Some(Action::MoveSelection {
                target: SelectionMove::Previous,
                extend: false,
            }),
            "selection.next" => Some(Action::MoveSelection {
                target: SelectionMove::Next,
                extend: false,
            }),
            "selection.first" => Some(Action::MoveSelection {
                target: SelectionMove::First,
                extend: false,
            }),
            "selection.last" => Some(Action::MoveSelection {
                target: SelectionMove::Last,
                extend: false,
            }),
            "selection.page-up" => Some(Action::MoveSelection {
                target: SelectionMove::PageUp,
                extend: false,
            }),
            "selection.page-down" => Some(Action::MoveSelection {
                target: SelectionMove::PageDown,
                extend: false,
            }),
            "selection.extend-previous" => Some(Action::MoveSelection {
                target: SelectionMove::Previous,
                extend: true,
            }),
            "selection.extend-next" => Some(Action::MoveSelection {
                target: SelectionMove::Next,
                extend: true,
            }),
            "selection.extend-first" => Some(Action::MoveSelection {
                target: SelectionMove::First,
                extend: true,
            }),
            "selection.extend-last" => Some(Action::MoveSelection {
                target: SelectionMove::Last,
                extend: true,
            }),
            "selection.extend-page-up" => Some(Action::MoveSelection {
                target: SelectionMove::PageUp,
                extend: true,
            }),
            "selection.extend-page-down" => Some(Action::MoveSelection {
                target: SelectionMove::PageDown,
                extend: true,
            }),
            "selection.open" => Some(Action::OpenCursor),
            "edit.select-all" => Some(Action::SelectAll),
            "edit.invert" => Some(Action::InvertSelection),
            "edit.undo" => Some(Action::Undo),
            "edit.redo" => Some(Action::Redo),
            "edit.batch-rename" => Some(Action::OpenBatchRename),
            "files.rename" if !archive_focused => Some(Action::RenameFocused),
            "files.create-folder" if !archive_focused => Some(Action::CreateFolderFocused),
            "files.create-file" if !archive_focused => Some(Action::CreateFileFocused),
            "files.trash" if !archive_focused => {
                let paths = self.selected_paths();
                (!paths.is_empty()).then_some(Action::Delete {
                    paths,
                    to_trash: true,
                })
            }
            "files.delete" if !archive_focused => {
                let paths = self.selected_paths();
                (!paths.is_empty()).then_some(Action::Delete {
                    paths,
                    to_trash: false,
                })
            }
            "files.compress" if !archive_focused => {
                let paths = self.selected_paths();
                (!paths.is_empty()).then_some(Action::CompressToZip(paths))
            }
            "files.open-dirs-in-tabs" if !archive_focused => {
                Some(Action::OpenSelectedDirsInNewTabs)
            }
            "files.open-in-window" if !archive_focused => Some(Action::OpenSelectedInNewWindow),
            "files.open-dir-right-split" if !archive_focused => {
                Some(Action::OpenSelectedDirInRightSplit)
            }
            "files.open-dir-bottom-split" if !archive_focused => {
                Some(Action::OpenSelectedDirInBottomSplit)
            }
            "recycle.restore" if recycle_focused => Some(Action::RestoreSelectedFromRecycleBin),
            "recycle.purge" if recycle_focused => Some(Action::PurgeSelectedFromRecycleBin),
            "recycle.empty" => Some(Action::EmptyRecycleBin),
            "profile.new" => Some(Action::CreateProfile(String::new())),
            "profile.rename" => Some(Action::OpenRenameProfile),
            "profile.delete" => {
                (self.state.profiles.profiles.len() > 1).then_some(Action::DeleteActiveProfile)
            }
            "ext.manager" => Some(Action::OpenExtManager),
            "sys.palette" => Some(Action::OpenPalette),
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
            || self.breadcrumb_menu.is_some()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_shortcuts_reject_static_input_bindings() {
        assert!(reserved_static_shortcut("ctrl-c"));
        assert!(reserved_static_shortcut("escape"));
    }

    #[test]
    fn non_reserved_shortcuts_are_allowed() {
        assert!(!reserved_static_shortcut("ctrl-alt-1"));
    }
}
