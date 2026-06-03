use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::settings::ViewMode;
use crate::ui::fuzzy_overlay::{show_fuzzy_overlay, FuzzyOverlayOptions, FuzzyOverlayState};

/// Static commands that don't require runtime state.
#[allow(clippy::type_complexity)]
const STATIC_COMMANDS: &[(&str, fn() -> Action)] = &[
    ("New Tab", || Action::NewTabFocused),
    ("Close Tab", || Action::CloseFocusedTab),
    ("Split Horizontal", || Action::SplitHorizontalFocused),
    ("Split Vertical", || Action::SplitVerticalFocused),
    ("Close Pane", || Action::CloseFocusedPane),
    ("Toggle Hidden Files", || Action::ToggleHidden),
    ("Toggle Inspector / Preview", || Action::ToggleInspector),
    ("Toggle Sidebar", || Action::ToggleSidebar),
    ("Open Settings", || Action::OpenSettings),
    ("Toggle Drop Shelf", || Action::ToggleDropShelf),
    ("Select All", || Action::SelectAll),
    ("Invert Selection", || Action::InvertSelection),
    ("Undo", || Action::Undo),
    ("Redo", || Action::Redo),
    ("Open Goto Bar", || Action::OpenGotoBar),
    ("Batch Rename", || Action::OpenBatchRename),
    ("View: Details", || Action::SetViewMode(ViewMode::Details)),
    ("View: Thumbnails", || Action::SetViewMode(ViewMode::Grid)),
    ("View: List", || Action::SetViewMode(ViewMode::List)),
    ("View: Columns (Miller)", || {
        Action::SetViewMode(ViewMode::Miller)
    }),
];

pub fn show_command_palette(
    ctx: &egui::Context,
    app_state: &AppState,
    overlay_state: &mut FuzzyOverlayState,
    actions: &mut Vec<Action>,
) {
    // Build dynamic command list
    let selected_paths: Vec<std::path::PathBuf> = app_state
        .active_tab_state()
        .map(|tab| {
            tab.selection
                .selected
                .iter()
                .filter_map(|idx| tab.entries.get(*idx))
                .map(|e| e.path.clone())
                .collect()
        })
        .unwrap_or_default();
    let has_selection = !selected_paths.is_empty();

    let focused = app_state.focused_tile;
    let profiles = &app_state.profiles.profiles;

    // Collect all labels + action factories into owned Vecs so lifetimes work out
    let mut labels: Vec<String> = Vec::new();
    let mut action_builders: Vec<Box<dyn Fn() -> Action>> = Vec::new();

    for (label, f) in STATIC_COMMANDS {
        let f = *f;
        labels.push(label.to_string());
        action_builders.push(Box::new(f));
    }

    labels.push("Navigate Back".to_string());
    action_builders.push(Box::new(move || Action::NavigateBack(focused)));

    labels.push("Navigate Forward".to_string());
    action_builders.push(Box::new(move || Action::NavigateForward(focused)));

    if has_selection {
        let trash_paths = selected_paths.clone();
        labels.push("Move to Trash".to_string());
        action_builders.push(Box::new(move || Action::Delete {
            paths: trash_paths.clone(),
            to_trash: true,
        }));

        let perm_paths = selected_paths.clone();
        labels.push("Delete Permanently".to_string());
        action_builders.push(Box::new(move || Action::Delete {
            paths: perm_paths.clone(),
            to_trash: false,
        }));
    }

    for (idx, profile) in profiles.iter().enumerate() {
        let name = format!("Switch Profile: {}", profile.name);
        labels.push(name);
        action_builders.push(Box::new(move || Action::SwitchProfile(idx)));
    }

    #[cfg(feature = "cloud")]
    if app_state.ext.registry.is_enabled("cloud") {
        labels.push("Connect to Server / Cloud (SFTP, Drive, …)".to_string());
        action_builders.push(Box::new(|| {
            Action::Cloud(crate::app::cloud::CloudAction::ShowAddDialog(true))
        }));
    }

    // Extension-contributed commands for the current selection.
    let selection: Vec<(Option<String>, bool)> = selected_paths
        .iter()
        .map(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            (ext, p.is_dir())
        })
        .collect();
    for cref in app_state.ext.registry.commands_for(&selection) {
        let (ext_id, cmd_id) = (cref.ext_id.clone(), cref.command.id.clone());
        labels.push(cref.command.title.clone());
        action_builders.push(Box::new(move || {
            Action::Ext(crate::app::ext::ExtAction::Run {
                ext_id: ext_id.clone(),
                cmd_id: cmd_id.clone(),
            })
        }));
    }

    labels.push("Manage Extensions".to_string());
    action_builders.push(Box::new(|| {
        Action::Ext(crate::app::ext::ExtAction::OpenManager(true))
    }));

    let query_lower = overlay_state.query.to_lowercase();
    let filtered_labels: Vec<&str> = if query_lower.is_empty() {
        labels.iter().map(|s| s.as_str()).collect()
    } else {
        labels
            .iter()
            .filter(|l| l.to_lowercase().contains(&query_lower))
            .map(|s| s.as_str())
            .collect()
    };

    show_fuzzy_overlay(
        ctx,
        egui::Id::new("command_palette"),
        overlay_state,
        &filtered_labels,
        FuzzyOverlayOptions {
            placeholder: "Search for commands...",
            empty_group: "Commands",
            item_kind: "Command",
            item_icon: egui_phosphor::regular::COMMAND,
            footer_action: "Run Command",
            shortcut: "Ctrl Shift P",
        },
        Action::ClosePalette,
        |label| {
            if let Some(pos) = labels.iter().position(|l| l == label) {
                action_builders[pos]()
            } else {
                Action::ClosePalette
            }
        },
        actions,
    );
}
