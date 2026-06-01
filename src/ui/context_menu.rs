use crate::input::action::Action;
use egui_tiles::TileId;
use std::path::{Path, PathBuf};

pub fn show_context_menu(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_path: &Path,
    paths: &[PathBuf],
    actions: &mut Vec<Action>,
) {
    let query_id = ui.id().with(("context_menu_query", tile_id));
    let selected_id = ui.id().with(("context_menu_selected", tile_id));
    let mut query = ui
        .data_mut(|data| data.get_temp::<String>(query_id))
        .unwrap_or_default();
    let mut selected = ui
        .data_mut(|data| data.get_temp::<usize>(selected_id))
        .unwrap_or_default();
    let search = ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text("Search actions...")
            .desired_width(180.0),
    );
    search.request_focus();
    ui.data_mut(|data| data.insert_temp(query_id, query.clone()));

    let mut items = menu_items(pane_path, paths);
    if !query.is_empty() {
        let query = query.to_lowercase();
        items.retain(|(label, _)| label.to_lowercase().contains(&query));
    }
    selected = selected.min(items.len().saturating_sub(1));

    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            selected = (selected + 1).min(items.len().saturating_sub(1));
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            selected = selected.saturating_sub(1);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            if let Some((_, Some(action))) = items.get(selected) {
                actions.push(action.clone());
                ui.close();
            }
        }
    });
    ui.data_mut(|data| data.insert_temp(selected_id, selected));
    ui.separator();

    for (index, (label, action)) in items.into_iter().enumerate() {
        let response = ui.selectable_label(index == selected, label);
        if response.hovered() {
            selected = index;
        }
        if response.clicked() {
            if let Some(action) = action {
                actions.push(action);
            }
            ui.close();
        }
    }
    ui.data_mut(|data| data.insert_temp(selected_id, selected));
}

fn menu_items(pane_path: &Path, paths: &[PathBuf]) -> Vec<(&'static str, Option<Action>)> {
    let mut items = Vec::new();
    if !paths.is_empty() {
        items.push(("Open", Some(Action::Open(paths.to_vec()))));
        items.push(("Copy", Some(Action::Copy(paths.to_vec()))));
        items.push(("Cut", Some(Action::Cut(paths.to_vec()))));
    }

    let paste_dest = paths
        .first()
        .map(|path| {
            if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| pane_path.to_path_buf())
            }
        })
        .unwrap_or_else(|| pane_path.to_path_buf());
    items.push(("Paste", Some(Action::Paste { dest: paste_dest })));

    if paths.len() == 1 {
        items.push(("Rename", None));
    } else if paths.len() > 1 {
        items.push(("Batch Rename", Some(Action::OpenBatchRename)));
    }
    if !paths.is_empty() {
        items.push((
            "Delete",
            Some(Action::Delete {
                paths: paths.to_vec(),
                to_trash: true,
            }),
        ));
    }
    if let Some(path) = paths.first() {
        items.push((
            "Copy as Path",
            Some(Action::CopyPathToClipboard(path.clone())),
        ));
    }
    items.push((
        "New Folder",
        Some(Action::CreateFolder {
            parent: pane_path.to_path_buf(),
            name: "New Folder".to_string(),
        }),
    ));
    items
}
