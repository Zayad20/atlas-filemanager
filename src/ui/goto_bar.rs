use crate::input::action::Action;
use crate::ui::fuzzy_overlay::{show_fuzzy_overlay, FuzzyOverlayOptions, FuzzyOverlayState};
use std::path::PathBuf;

pub fn show_goto_bar(
    ctx: &egui::Context,
    state: &mut FuzzyOverlayState,
    results: &[PathBuf],
    focused_tile: egui_tiles::TileId,
    actions: &mut Vec<Action>,
) {
    let display: Vec<String> = results
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    show_fuzzy_overlay(
        ctx,
        egui::Id::new("goto_bar"),
        state,
        &display,
        FuzzyOverlayOptions {
            placeholder: "Search files and folders...",
            empty_group: "Suggestions",
            item_kind: "Folder",
            item_icon: egui_phosphor::regular::FOLDER,
            footer_action: "Open Folder",
            shortcut: "Ctrl K",
        },
        Action::CloseOverlays,
        |path_str| Action::Navigate {
            tab: focused_tile,
            path: PathBuf::from(path_str),
        },
        actions,
    );
}
