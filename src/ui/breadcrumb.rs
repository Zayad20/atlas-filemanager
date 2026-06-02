use std::path::{Component, Path, PathBuf};

use crate::state::tab::TabState;
use crate::state::{
    drag::{DragHoverState, DragHoverTarget},
    pane::TabId,
};
use crate::ui::drag_feedback::paint_drag_hover_confirmation;

pub fn show_breadcrumb(
    ui: &mut egui::Ui,
    tab: TabId,
    path: &Path,
    tab_state: &mut TabState,
    dragging: bool,
    drag_hover: &mut DragHoverState,
) -> Option<PathBuf> {
    let mut result = None;
    let mut enter_edit_mode = false;
    let mut child_button_hovered = false;
    let height = ui.spacing().interact_size.y;

    let bar = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let edit_button = ui.small_button("✎").on_hover_text("Edit location (Ctrl+L)");
            child_button_hovered |= edit_button.hovered();
            if edit_button.clicked() {
                enter_edit_mode = true;
            }

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                if tab_state.path_edit.editing {
                    let mut output = egui::TextEdit::singleline(&mut tab_state.path_edit.text)
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    let response = &output.response;
                    if tab_state.path_edit.focus {
                        response.request_focus();
                        output
                            .state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::default(),
                                egui::text::CCursor::new(tab_state.path_edit.text.chars().count()),
                            )));
                        output.state.store(ui.ctx(), response.id);
                        tab_state.path_edit.focus = false;
                    }
                    let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let tab_pressed = ui.input(|input| input.key_pressed(egui::Key::Tab));
                    if tab_pressed {
                        if let Some(completed) = complete_path(&tab_state.path_edit.text, path) {
                            tab_state.path_edit.text = completed;
                            tab_state.path_edit.error = None;
                        }
                        response.request_focus();
                    } else if enter_pressed {
                        let target = resolve_location(&tab_state.path_edit.text, path);
                        if target.is_dir() {
                            result = Some(target);
                            tab_state.path_edit.editing = false;
                            tab_state.path_edit.error = None;
                        } else {
                            tab_state.path_edit.error =
                                Some("Location does not exist or is not a folder".to_string());
                            response.request_focus();
                        }
                    } else if ui.input(|input| input.key_pressed(egui::Key::Escape))
                        || response.lost_focus()
                    {
                        tab_state.path_edit.editing = false;
                        tab_state.path_edit.error = None;
                    } else if response.changed() {
                        tab_state.path_edit.error = None;
                    }
                    if let Some(error) = &tab_state.path_edit.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    return;
                }

                let mut accumulated = PathBuf::new();
                let components: Vec<_> = path.components().collect();

                for (i, component) in components.iter().enumerate() {
                    match component {
                        Component::RootDir => {
                            accumulated.push("/");
                            let button = ui.small_button("/");
                            child_button_hovered |= button.hovered();
                            if dragging && button.contains_pointer() {
                                let target = DragHoverTarget::Location {
                                    tab,
                                    path: PathBuf::from("/"),
                                };
                                drag_hover.observed_target = Some(target.clone());
                                paint_drag_hover_confirmation(&button, drag_hover, &target);
                            }
                            if button.clicked() {
                                result = Some(PathBuf::from("/"));
                            }
                        }
                        Component::Normal(name) => {
                            accumulated.push(name);
                            let label = name.to_string_lossy();
                            let is_last = i == components.len() - 1;
                            if is_last {
                                ui.label(label.as_ref());
                            } else {
                                let button = ui.small_button(label.as_ref());
                                child_button_hovered |= button.hovered();
                                if dragging && button.contains_pointer() {
                                    let target = DragHoverTarget::Location {
                                        tab,
                                        path: accumulated.clone(),
                                    };
                                    drag_hover.observed_target = Some(target.clone());
                                    paint_drag_hover_confirmation(&button, drag_hover, &target);
                                }
                                if button.clicked() {
                                    result = Some(accumulated.clone());
                                }
                                ui.label("›");
                            }
                        }
                        _ => {}
                    }
                }
            });
        },
    );

    if bar.response.contains_pointer()
        && ui.input(|input| input.pointer.any_click())
        && !child_button_hovered
    {
        enter_edit_mode = true;
    }
    if enter_edit_mode && !tab_state.path_edit.editing {
        tab_state.path_edit.text = path.to_string_lossy().into_owned();
        tab_state.path_edit.editing = true;
        tab_state.path_edit.focus = true;
        tab_state.path_edit.error = None;
    }

    result
}

fn expand_home(text: &str) -> PathBuf {
    if text == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(text));
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

fn resolve_location(text: &str, current_dir: &Path) -> PathBuf {
    let expanded = expand_home(text);
    if expanded.is_absolute() {
        expanded
    } else {
        current_dir.join(expanded)
    }
}

fn complete_path(text: &str, current_dir: &Path) -> Option<String> {
    let expanded = resolve_location(text, current_dir);
    let (parent, prefix) = if expanded.is_dir() && text.ends_with('/') {
        (expanded, String::new())
    } else {
        (
            expanded.parent()?.to_path_buf(),
            expanded.file_name()?.to_string_lossy().into_owned(),
        )
    };
    let mut matches: Vec<String> = std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix))
        .collect();
    matches.sort();
    let completion = common_prefix(&matches)?;

    let typed = PathBuf::from(text);
    let base = typed.parent().unwrap_or_else(|| Path::new(""));
    let mut completed = base.join(completion).to_string_lossy().into_owned();
    if matches.len() == 1 {
        completed.push('/');
    }
    Some(completed)
}

fn common_prefix(matches: &[String]) -> Option<&str> {
    let first = matches.first()?;
    let mut end = first.len();
    for candidate in &matches[1..] {
        end = first
            .chars()
            .zip(candidate.chars())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>()
            .min(end);
    }
    Some(&first[..end])
}

#[cfg(test)]
mod tests {
    use super::{common_prefix, complete_path, resolve_location};
    use std::fs;

    #[test]
    fn resolves_relative_locations_from_visible_directory() {
        assert_eq!(
            resolve_location("../other", std::path::Path::new("/tmp/current")),
            std::path::PathBuf::from("/tmp/current/../other")
        );
    }

    #[test]
    fn completes_directory_prefix_and_adds_slash_for_unique_match() {
        let root =
            std::env::temp_dir().join(format!("Atlas-complete-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("Downloads")).unwrap();

        assert_eq!(complete_path("Down", &root), Some("Downloads/".to_string()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn common_prefix_handles_unicode() {
        let matches = vec!["café".to_string(), "caféteria".to_string()];
        assert_eq!(common_prefix(&matches), Some("café"));
    }
}
