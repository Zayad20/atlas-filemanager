use egui::{Align, Color32, Layout, RichText};
use egui_phosphor::regular;
use std::path::PathBuf;

use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::drag::{DragHoverTarget, SidebarDragItem};
use crate::state::profiles::{Profile, SectionKind, PRESET_COLORS};
use crate::state::settings::Theme;
use crate::state::tag_cache::TagId;

/// Content only — the SidePanel wrapper is in layout.rs so it can be animated.
pub fn show_sidebar_content(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        show_profile_selector(ui, state, actions);
        ui.add_space(6.0);
        show_sections(ui, state, actions);
    });
}

// ── Profile selector ─────────────────────────────────────────────────────────

fn show_profile_selector(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let active_name = state.profiles.active().name.clone();

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("profile_selector")
            .width(ui.available_width() - 28.0)
            .selected_text(&active_name)
            .show_ui(ui, |ui| {
                for idx in 0..state.profiles.profiles.len() {
                    let name = state.profiles.profiles[idx].name.clone();
                    let selected = idx == state.profiles.active_idx;
                    if ui.selectable_label(selected, &name).clicked() && !selected {
                        actions.push(Action::SwitchProfile(idx));
                    }
                }
            });

        if ui
            .small_button(regular::PLUS)
            .on_hover_text("New profile")
            .clicked()
        {
            let new_name = format!("Profile {}", state.profiles.profiles.len() + 1);
            state.profiles.profiles.push(Profile::new(new_name));
            let new_idx = state.profiles.profiles.len() - 1;
            actions.push(Action::SwitchProfile(new_idx));
        }
    });

    ui.horizontal(|ui| {
        ui.label("Name:");
        let profile = &mut state.profiles.profiles[state.profiles.active_idx];
        ui.text_edit_singleline(&mut profile.name);
    });

    // Theme selector — per profile
    ui.horizontal(|ui| {
        ui.label("Theme:");
        let profile = &mut state.profiles.profiles[state.profiles.active_idx];
        egui::ComboBox::from_id_salt("profile_theme")
            .selected_text(match profile.theme {
                Theme::Dark => "Dark",
                Theme::Light => "Light",
                Theme::CatppuccinMocha => "Catppuccin Mocha",
                Theme::Nord => "Nord",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut profile.theme, Theme::Dark, "Dark");
                ui.selectable_value(&mut profile.theme, Theme::Light, "Light");
                ui.selectable_value(
                    &mut profile.theme,
                    Theme::CatppuccinMocha,
                    "Catppuccin Mocha",
                );
                ui.selectable_value(&mut profile.theme, Theme::Nord, "Nord");
            });
    });

    if state.profiles.profiles.len() > 1
        && ui
            .small_button("Delete profile")
            .on_hover_text("Remove this profile and its session")
            .clicked()
    {
        actions.push(Action::DeleteActiveProfile);
    }
}

// ── Section loop ─────────────────────────────────────────────────────────────

fn show_sections(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let n = state.profiles.active().sidebar_sections.len();


    for i in 0..n {
        let kind = state.profiles.active().sidebar_sections[i].kind.clone();
        let collapsed = state.profiles.active().sidebar_sections[i].collapsed;

        // Section header row
        let header_resp = ui.horizontal(|ui| {
            let arrow = if collapsed {
                regular::CARET_RIGHT
            } else {
                regular::CARET_DOWN
            };
            if ui.small_button(arrow).clicked() {
                actions.push(Action::ToggleSidebarSection(i));
            }
            ui.label(
                RichText::new(format!("{}  {}", section_icon(&kind), kind.label()))
                    .small()
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if i < n - 1
                    && ui
                        .small_button(regular::ARROW_DOWN)
                        .on_hover_text("Move section down")
                        .clicked()
                {
                    actions.push(Action::MoveSidebarSectionDown(i));
                }
                if i > 0
                    && ui
                        .small_button(regular::ARROW_UP)
                        .on_hover_text("Move section up")
                        .clicked()
                {
                    actions.push(Action::MoveSidebarSectionUp(i));
                }
            });
        });
        let _ = header_resp;

        crate::ui::anim::collapse_vertical(
            ui,
            ("sidebar_sec", i, kind.label()),
            !collapsed,
            |ui| match kind {
                SectionKind::Places => show_places(ui, state, actions),
                SectionKind::Bookmarks => show_bookmarks(ui, state, actions),
                SectionKind::Tags => show_tags(ui, state, actions),
                SectionKind::Drives => show_drives(ui, state, actions),
                SectionKind::Recent => show_recent(ui, state, actions),
                SectionKind::Cloud => {
                    #[cfg(feature = "cloud")]
                    if state.ext.registry.is_enabled("cloud") {
                        crate::ui::cloud::show_cloud(ui, state, actions);
                    } else {
                        ui.label(RichText::new("Cloud extension disabled").small().weak());
                    }
                }
            },
        );

        ui.add_space(2.0);
    }

}

// ── Places ───────────────────────────────────────────────────────────────────

fn show_places(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let places: &[(&str, &str, Option<PathBuf>)] = &[
        (regular::HOUSE, "Home", dirs::home_dir()),
        (regular::DESKTOP, "Desktop", dirs::desktop_dir()),
        (regular::DOWNLOAD, "Downloads", dirs::download_dir()),
        (regular::FILE_TEXT, "Documents", dirs::document_dir()),
        (regular::IMAGE, "Pictures", dirs::picture_dir()),
        (regular::VIDEO, "Videos", dirs::video_dir()),
        (regular::MUSIC_NOTE, "Music", dirs::audio_dir()),
    ];

    for (icon, label, path) in places {
        if let Some(p) = path {
            if p.exists() {
                let response = ui
                    .selectable_label(false, format!("{icon}  {label}"))
                    .interact(egui::Sense::click_and_drag());
                observe_drag_location(state, &response, p);
                set_sidebar_drag_source(state, &response, p);
                if response.clicked() {
                    actions.push(Action::Navigate {
                        tab: state.focused_tile,
                        path: p.clone(),
                    });
                }
            }
        }
    }
}

// ── Bookmarks ────────────────────────────────────────────────────────────────

fn show_bookmarks(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {


    let count = state.profiles.active().bookmarks.len();
    for i in 0..count {
        let (name, path) = {
            let b = &state.profiles.active().bookmarks[i];
            (b.name.clone(), b.path.clone())
        };

        ui.horizontal(|ui| {
            let response = ui
                .selectable_label(false, format!("{}  {name}", regular::BOOKMARK))
                .interact(egui::Sense::click_and_drag());
            observe_drag_location(state, &response, &path);
            set_sidebar_drag_source(state, &response, &path);
            if response.clicked() {
                actions.push(Action::Navigate {
                    tab: state.focused_tile,
                    path: path.clone(),
                });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button(regular::X)
                    .on_hover_text("Remove")
                    .clicked()
                {
                    actions.push(Action::RemoveBookmark(i));
                }
            });
        });
        ui.label(
            RichText::new(path.to_string_lossy().as_ref())
                .small()
                .weak(),
        );
    }


    if state.profile_ui.adding_bookmark {
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut state.profile_ui.new_bookmark_name);
        });
        ui.horizontal(|ui| {
            let cur = current_dir(state);
            if ui.button("Add current folder").clicked() {
                let name = if state.profile_ui.new_bookmark_name.trim().is_empty() {
                    crate::util::display::display_name(&cur)
                } else {
                    state.profile_ui.new_bookmark_name.trim().to_string()
                };
                actions.push(Action::AddBookmark {
                    name,
                    path: cur,
                });
                state.profile_ui.adding_bookmark = false;
                state.profile_ui.new_bookmark_name.clear();
            }
            if ui.button("Cancel").clicked() {
                state.profile_ui.adding_bookmark = false;
                state.profile_ui.new_bookmark_name.clear();
            }
        });
    } else if ui
        .small_button(format!("{}  Add bookmark", regular::PLUS))
        .clicked()
    {
        state.profile_ui.adding_bookmark = true;
        let path = current_dir(state);
        state.profile_ui.new_bookmark_name = crate::util::display::display_name(&path);
    }
}

// ── Tags ─────────────────────────────────────────────────────────────────────

fn show_tags(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let active_profile = state.profiles.active().name.clone();
    // Snapshot the visible defs so we can borrow `state` mutably inside the loop.
    let defs: Vec<(TagId, String, [u8; 3], bool)> = state
        .tag_cache
        .visible_defs(&active_profile)
        .map(|d| (d.id, d.name.clone(), d.color, d.profile.is_none()))
        .collect();

    let dragging_files = state.drag_drop.dragged_files.is_some();
    let released = ui.input(|i| i.pointer.any_released());
    let mut global_header_shown = false;

    for (id, name, color, is_global) in defs {
        if is_global && !global_header_shown {
            ui.label(RichText::new("global").small().weak().italics());
            global_header_shown = true;
        }
        let c = Color32::from_rgb(color[0], color[1], color[2]);
        let (response, remove_clicked) = tag_row(ui, &name, c);
        if remove_clicked {
            actions.push(Action::DeleteTag(id));
        }
        // File(s) dropped onto a tag → assign it immediately (no spring-load).
        if dragging_files && response.contains_pointer() {
            crate::ui::drag_feedback::paint_drop_highlight(&response);
            if released {
                if let Some(files) = &state.drag_drop.dragged_files {
                    actions.push(Action::AssignTagToFiles {
                        tag: id,
                        paths: files.paths.clone(),
                    });
                }
            }
        }
        // Drag the tag out of the sidebar → opens its results in a new tab on release.
        if response.drag_started() {
            state.drag_drop.sidebar_drag = Some(SidebarDragItem::Tag(id));
        }
    }

    if state.profile_ui.adding_tag {
        show_add_tag_form(ui, state, actions, &active_profile);
    } else if ui
        .small_button(format!("{}  Add tag", regular::PLUS))
        .clicked()
    {
        state.profile_ui.adding_tag = true;
        state.profile_ui.new_tag_name.clear();
        state.profile_ui.new_tag_global = false;
        state.profile_ui.new_tag_color = PRESET_COLORS[0];
    }
}

/// Renders one tag row. Returns the (draggable) name response and whether ✕ was clicked.
fn tag_row(ui: &mut egui::Ui, name: &str, color: Color32) -> (egui::Response, bool) {
    let mut remove_clicked = false;
    let inner = ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.label(regular::TAG);
        let label = ui.add(egui::Label::new(name).sense(egui::Sense::click_and_drag()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(regular::X).clicked() {
                remove_clicked = true;
            }
        });
        label
    });
    (inner.inner, remove_clicked)
}

fn show_add_tag_form(
    ui: &mut egui::Ui,
    state: &mut AppState,
    actions: &mut Vec<Action>,
    active_profile: &str,
) {
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut state.profile_ui.new_tag_name);
    });
    ui.horizontal(|ui| {
        ui.label("Color:");
        for &color in PRESET_COLORS {
            let c32 = Color32::from_rgb(color[0], color[1], color[2]);
            let selected = state.profile_ui.new_tag_color == color;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
            ui.painter().circle_filled(rect.center(), 7.0, c32);
            if selected {
                ui.painter().circle_stroke(
                    rect.center(),
                    8.5,
                    egui::Stroke::new(1.5, ui.visuals().text_color()),
                );
            }
            if resp.clicked() {
                state.profile_ui.new_tag_color = color;
            }
        }
    });
    ui.checkbox(
        &mut state.profile_ui.new_tag_global,
        "Global (all profiles)",
    );
    ui.horizontal(|ui| {
        let can_add = !state.profile_ui.new_tag_name.trim().is_empty();
        if ui.add_enabled(can_add, egui::Button::new("Add")).clicked() {
            let profile = if state.profile_ui.new_tag_global {
                None
            } else {
                Some(active_profile.to_string())
            };
            actions.push(Action::CreateTag {
                name: state.profile_ui.new_tag_name.trim().to_string(),
                color: state.profile_ui.new_tag_color,
                profile,
            });
            state.profile_ui.adding_tag = false;
            state.profile_ui.new_tag_name.clear();
        }
        if ui.button("Cancel").clicked() {
            state.profile_ui.adding_tag = false;
        }
    });
}

// ── Drives ───────────────────────────────────────────────────────────────────

fn show_drives(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    // Removable devices (udisks2): mount points let us tag fixed mounts with an
    // eject button and surface not-yet-mounted disks as one-click mounts.
    let removable = crate::fs::udisks::list_removable();
    let eject_for = |path: &std::path::Path| -> Option<String> {
        removable
            .iter()
            .find(|d| d.mount_point.as_deref() == Some(path))
            .map(|d| d.dev.clone())
    };

    for mount in crate::fs::mounts::read_mounts() {
        ui.horizontal(|ui| {
            let response = ui
                .selectable_label(
                    false,
                    format!("{}  {}", regular::HARD_DRIVES, mount.to_string_lossy()),
                )
                .interact(egui::Sense::click_and_drag());
            observe_drag_location(state, &response, &mount);
            set_sidebar_drag_source(state, &response, &mount);
            if response.clicked() {
                actions.push(Action::Navigate {
                    tab: state.focused_tile,
                    path: mount.clone(),
                });
            }
            if let Some(dev) = eject_for(&mount) {
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(regular::EJECT).on_hover_text("Eject").clicked() {
                        actions.push(Action::UnmountDevice(dev));
                    }
                });
            }
        });
    }

    // Connected-but-unmounted removable disks → click to mount.
    for dev in removable.iter().filter(|d| d.mount_point.is_none()) {
        let label = if dev.size.is_empty() {
            format!("{}  {}", regular::USB, dev.label)
        } else {
            format!("{}  {}  ({})", regular::USB, dev.label, dev.size)
        };
        if ui
            .selectable_label(false, label)
            .on_hover_text(format!("Mount {}", dev.dev))
            .clicked()
        {
            actions.push(Action::MountDevice(dev.dev.clone()));
        }
    }
}

// ── Recent ───────────────────────────────────────────────────────────────────

fn show_recent(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    if state.settings.recent_folders.is_empty() {
        ui.label(RichText::new("No recent folders").small().weak());
        return;
    }
    let recent: Vec<PathBuf> = state
        .settings
        .recent_folders
        .iter()
        .rev()
        .take(15)
        .cloned()
        .collect();
    for path in recent {
        let label = crate::util::display::display_name(&path);
        let response = ui
            .selectable_label(
                false,
                format!("{}  {label}", regular::CLOCK_COUNTER_CLOCKWISE),
            )
            .interact(egui::Sense::click_and_drag())
            .on_hover_text(path.to_string_lossy().as_ref());
        observe_drag_location(state, &response, &path);
        set_sidebar_drag_source(state, &response, &path);
        if response.clicked() {
            actions.push(Action::Navigate {
                tab: state.focused_tile,
                path,
            });
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// If a sidebar row was just grabbed (not merely clicked), record it as the active
/// sidebar drag so a drop over the view opens it in a new tab.
fn set_sidebar_drag_source(state: &mut AppState, response: &egui::Response, path: &std::path::Path) {
    if response.drag_started() {
        state.drag_drop.sidebar_drag = Some(SidebarDragItem::Path(path.to_path_buf()));
    }
}

fn observe_drag_location(state: &mut AppState, response: &egui::Response, path: &std::path::Path) {
    if response.contains_pointer()
        && (state.drag_drop.dragged_files.is_some()
            || response
                .ctx
                .input(|input| !input.raw.hovered_files.is_empty()))
    {
        let target = DragHoverTarget::Location {
            tab: state.focused_tile,
            path: path.to_path_buf(),
        };
        state.drag_drop.drag_hover.observed_target = Some(target.clone());
        crate::ui::drag_feedback::paint_drag_hover_confirmation(response, &state.drag_drop.drag_hover, &target);
    }
}

fn current_dir(state: &AppState) -> PathBuf {
    state
        .active_tab_data()
        .map(|t| t.path.clone())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

fn section_icon(kind: &SectionKind) -> &'static str {
    match kind {
        SectionKind::Places => regular::HOUSE,
        SectionKind::Bookmarks => regular::BOOKMARKS,
        SectionKind::Tags => regular::TAG,
        SectionKind::Drives => regular::HARD_DRIVES,
        SectionKind::Recent => regular::CLOCK_COUNTER_CLOCKWISE,
        SectionKind::Cloud => regular::CLOUD,
    }
}
