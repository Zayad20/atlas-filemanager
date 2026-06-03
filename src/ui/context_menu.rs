use crate::app::ext::ExtAction;
use crate::ext::manifest::ExtCommand;
use crate::input::action::Action;
use egui_tiles::TileId;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Per-frame snapshot of enabled declarative ext commands, published by the
/// layout (which has `AppState`) so the context menu can read it without the
/// registry being threaded through every render signature.
#[derive(Clone, Default)]
pub struct FrameExtCommands(pub Arc<Vec<(String, ExtCommand)>>);

fn frame_commands_id() -> egui::Id {
    egui::Id::new("frame_ext_commands")
}

/// Called once per frame from the layout.
pub fn publish_ext_commands(ctx: &egui::Context, commands: Vec<(String, ExtCommand)>) {
    ctx.data_mut(|d| d.insert_temp(frame_commands_id(), FrameExtCommands(Arc::new(commands))));
}

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

    show_ext_commands(ui, paths, &query, actions);
}

/// Append extension-contributed commands that apply to `paths`.
fn show_ext_commands(ui: &mut egui::Ui, paths: &[PathBuf], query: &str, actions: &mut Vec<Action>) {
    if paths.is_empty() {
        return;
    }
    let snapshot = ui
        .ctx()
        .data(|d| d.get_temp::<FrameExtCommands>(frame_commands_id()))
        .unwrap_or_default();
    if snapshot.0.is_empty() {
        return;
    }

    // Selection descriptors: (extension, is_dir) for every right-clicked path.
    let selection: Vec<(Option<String>, bool)> = paths
        .iter()
        .map(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            (ext, p.is_dir())
        })
        .collect();
    let query = query.to_lowercase();

    let mut first = true;
    for (ext_id, cmd) in snapshot.0.iter() {
        let applies = selection
            .iter()
            .all(|(e, is_dir)| cmd.accepts.matches(e.as_deref(), *is_dir));
        if !applies {
            continue;
        }
        if !cmd.requires.iter().all(|b| crate::ext::registry::has_binary(b)) {
            continue;
        }
        if !query.is_empty() && !cmd.title.to_lowercase().contains(&query) {
            continue;
        }
        if first {
            ui.separator();
            first = false;
        }
        if ui.selectable_label(false, &cmd.title).clicked() {
            actions.push(Action::Ext(ExtAction::Run {
                ext_id: ext_id.clone(),
                cmd_id: cmd.id.clone(),
            }));
            ui.close();
        }
    }
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
