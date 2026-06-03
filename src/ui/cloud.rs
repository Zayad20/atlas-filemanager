//! Cloud/network UI: sidebar section, add-remote dialog, and transfers panel.

use egui_phosphor::regular;

use crate::app::cloud::CloudAction;
use crate::cloud::{AccessKind, RemoteStatus, TransferState};
use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::cloud_state::AddRemoteKind;

fn cloud(a: CloudAction) -> Action {
    Action::Cloud(a)
}

/// Sidebar "Network" section: one row per remote + an add button.
pub fn show_cloud(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    // rclone download progress (first use).
    if let Some((done, total)) = state.cloud.rclone_progress {
        let frac = if total > 0 {
            done as f32 / total as f32
        } else {
            0.0
        };
        ui.add(egui::ProgressBar::new(frac).text("Setting up rclone…"));
    }

    if state.cloud.remotes.is_empty() {
        ui.label(egui::RichText::new("No remotes").small().weak());
    }

    // Snapshot to avoid borrowing `state.cloud` while pushing actions.
    struct Row {
        id: String,
        name: String,
        status: RemoteStatus,
        access: AccessKind,
        quota: Option<(u64, u64)>,
    }
    let rows: Vec<Row> = state
        .cloud
        .remotes
        .iter()
        .map(|r| Row {
            id: r.id.clone(),
            name: r.name.clone(),
            status: r.status.clone(),
            access: r.access,
            quota: r.quota,
        })
        .collect();

    for Row {
        id,
        name,
        status,
        access,
        quota,
    } in rows
    {
        ui.horizontal(|ui| {
            let label = format!("{}  {name}", regular::CLOUD);
            let mut hover = match &status {
                RemoteStatus::Idle => "Idle".to_string(),
                RemoteStatus::Connecting => "Connecting…".to_string(),
                RemoteStatus::Mounting => "Mounting…".to_string(),
                RemoteStatus::Mounted(p) => format!("Mounted at {}", p.display()),
                RemoteStatus::Error(e) => format!("Error: {e}"),
            };
            if let Some((used, total)) = quota {
                if total > 0 {
                    hover = format!(
                        "{hover}\n{} of {} used",
                        crate::util::display::format_size(used),
                        crate::util::display::format_size(total),
                    );
                }
            }
            let resp = ui.selectable_label(false, label).on_hover_text(hover);
            if resp.clicked() {
                match access {
                    AccessKind::Access => actions.push(cloud(CloudAction::OpenAccess(id.clone()))),
                    AccessKind::Mount => match &status {
                        RemoteStatus::Mounted(_) => {
                            actions.push(cloud(CloudAction::OpenMount(id.clone())))
                        }
                        _ => actions.push(cloud(CloudAction::Mount(id.clone()))),
                    },
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Remove.
                if ui
                    .small_button(regular::X)
                    .on_hover_text("Remove remote")
                    .clicked()
                {
                    actions.push(cloud(CloudAction::RemoveRemote(id.clone())));
                }
                // Mount/unmount toggle for Mount-mode remotes.
                if access == AccessKind::Mount {
                    match &status {
                        RemoteStatus::Mounted(_) => {
                            if ui.small_button(regular::EJECT).on_hover_text("Unmount").clicked() {
                                actions.push(cloud(CloudAction::Unmount(id.clone())));
                            }
                        }
                        RemoteStatus::Mounting | RemoteStatus::Connecting => {
                            ui.spinner();
                        }
                        _ => {
                            if ui
                                .small_button(regular::ARROW_LINE_DOWN)
                                .on_hover_text("Mount")
                                .clicked()
                            {
                                actions.push(cloud(CloudAction::Mount(id.clone())));
                            }
                        }
                    }
                }
            });
        });
    }

    ui.add_space(2.0);
    if ui
        .button(format!("{}  Add remote", regular::PLUS))
        .clicked()
    {
        actions.push(cloud(CloudAction::ShowAddDialog(true)));
    }
    if !state.cloud.transfers.is_empty()
        && ui
            .button(format!("{}  Transfers", regular::ARROWS_DOWN_UP))
            .clicked()
    {
        actions.push(cloud(CloudAction::ShowTransfers(true)));
    }
}

/// Floating add-remote dialog. Call once per frame from the layout.
pub fn show_add_remote_window(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    if !state.cloud.show_add_dialog {
        return;
    }
    let mut open = true;
    egui::Window::new("Add remote")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let form = &mut state.cloud.add_form;
            egui::ComboBox::from_label("Type")
                .selected_text(match form.kind {
                    AddRemoteKind::Sftp => "SFTP",
                    AddRemoteKind::Ftp => "FTP",
                    AddRemoteKind::Smb => "SMB",
                    AddRemoteKind::GoogleDrive => "Google Drive",
                    AddRemoteKind::OneDrive => "OneDrive",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut form.kind, AddRemoteKind::Sftp, "SFTP");
                    ui.selectable_value(&mut form.kind, AddRemoteKind::Ftp, "FTP");
                    ui.selectable_value(&mut form.kind, AddRemoteKind::Smb, "SMB");
                    ui.selectable_value(&mut form.kind, AddRemoteKind::GoogleDrive, "Google Drive");
                    ui.selectable_value(&mut form.kind, AddRemoteKind::OneDrive, "OneDrive");
                });

            let oauth = form.kind.is_oauth();
            egui::Grid::new("add_remote_grid").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut form.name);
                ui.end_row();
                // OAuth backends need no host/credentials — authorized in the browser.
                if !oauth {
                    ui.label("Host");
                    ui.text_edit_singleline(&mut form.host);
                    ui.end_row();
                    ui.label("Port");
                    ui.text_edit_singleline(&mut form.port);
                    ui.end_row();
                    ui.label("User");
                    ui.text_edit_singleline(&mut form.user);
                    ui.end_row();
                    ui.label("Password");
                    ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                    ui.end_row();
                }
            });
            if oauth {
                ui.label(
                    egui::RichText::new("Opens your browser to sign in. No password stored.")
                        .small()
                        .weak(),
                );
            }

            ui.horizontal(|ui| {
                ui.label("Mode");
                ui.selectable_value(&mut form.access, AccessKind::Mount, "Mount");
                ui.selectable_value(&mut form.access, AccessKind::Access, "Access");
            });
            ui.label(
                egui::RichText::new(match form.access {
                    AccessKind::Mount => "Appears as a real folder (FUSE). Needs fuse3.",
                    AccessKind::Access => "Browse/upload/download without mounting.",
                })
                .small()
                .weak(),
            );

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Connect").clicked() {
                    actions.push(cloud(CloudAction::SubmitAddRemote));
                }
                if ui.button("Cancel").clicked() {
                    actions.push(cloud(CloudAction::ShowAddDialog(false)));
                }
            });

            if let Some(err) = &state.cloud.last_error {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
            }
        });
    if !open {
        actions.push(cloud(CloudAction::ShowAddDialog(false)));
    }
}

/// Floating transfers panel (downloads/uploads with pause/retry/cancel).
pub fn show_transfers_window(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    if !state.cloud.show_transfers {
        return;
    }
    let mut open = true;
    egui::Window::new("Transfers")
        .collapsible(true)
        .resizable(true)
        .default_width(380.0)
        .open(&mut open)
        .show(ctx, |ui| {
            if state.cloud.transfers.is_empty() {
                ui.label(egui::RichText::new("No transfers").weak());
            } else {
                let active = state
                    .cloud
                    .transfers
                    .iter()
                    .filter(|t| !t.state.is_terminal())
                    .count();
                let done: u64 = state.cloud.transfers.iter().map(|t| t.bytes_done).sum();
                let total: u64 = state.cloud.transfers.iter().map(|t| t.bytes_total).sum();
                ui.label(
                    egui::RichText::new(format!(
                        "{active} active · {} / {}",
                        crate::util::display::format_size(done),
                        crate::util::display::format_size(total),
                    ))
                    .small()
                    .weak(),
                );
                ui.separator();
            }
            for t in &state.cloud.transfers {
                ui.horizontal(|ui| {
                    ui.label(&t.label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match &t.state {
                            TransferState::Queued | TransferState::Running => {
                                if ui.small_button(regular::X).on_hover_text("Cancel").clicked() {
                                    actions.push(cloud(CloudAction::CancelTransfer(t.id)));
                                }
                                if ui.small_button(regular::PAUSE).clicked() {
                                    actions.push(cloud(CloudAction::PauseTransfer(t.id)));
                                }
                            }
                            TransferState::Paused => {
                                if ui.small_button(regular::PLAY).clicked() {
                                    actions.push(cloud(CloudAction::ResumeTransfer(t.id)));
                                }
                            }
                            TransferState::Failed(_) => {
                                if ui
                                    .small_button(regular::ARROW_CLOCKWISE)
                                    .on_hover_text("Retry")
                                    .clicked()
                                {
                                    actions.push(cloud(CloudAction::RetryTransfer(t.id)));
                                }
                            }
                            TransferState::Done => {}
                        }
                    });
                });
                match &t.state {
                    TransferState::Failed(e) => {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
                    }
                    TransferState::Done => {
                        ui.add(egui::ProgressBar::new(1.0).text("Done"));
                    }
                    _ => {
                        ui.add(egui::ProgressBar::new(t.fraction()).show_percentage());
                    }
                }
            }
        });
    if !open {
        state.cloud.show_transfers = false;
    }
}
