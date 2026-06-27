#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
//! UI-facing state for the cloud extension (feature `cloud`).
//!
//! Mirrors what the cloud worker reports via `BgResult`; the worker owns the
//! authoritative rclone state, this is just what the sidebar/dialogs render.

use crate::cloud_model::{AccessKind, CloudRemote, RemoteId, TransferSnapshot};

/// Which backend the add-remote dialog is configuring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddRemoteKind {
    #[default]
    Sftp,
    Ftp,
    Smb,
    GoogleDrive,
    OneDrive,
}

impl AddRemoteKind {
    /// OAuth kinds need no host/credentials — the dialog hides those fields.
    pub const fn is_oauth(&self) -> bool {
        matches!(self, Self::GoogleDrive | Self::OneDrive)
    }
}

/// Scratch fields for the add-remote dialog.
#[derive(Debug, Clone)]
pub struct AddRemoteForm {
    pub kind: AddRemoteKind,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub access: AccessKind,
}

impl Default for AddRemoteForm {
    fn default() -> Self {
        Self {
            kind: AddRemoteKind::Sftp,
            name: String::new(),
            host: String::new(),
            port: String::new(),
            user: String::new(),
            password: String::new(),
            access: AccessKind::Mount,
        }
    }
}

#[derive(Default)]
pub struct CloudState {
    pub remotes: Vec<CloudRemote>,
    /// Remotes whose mount was user-initiated, so the focused pane navigates into
    /// them once mounted (auto-mounted remotes are absent → no navigation hijack).
    pub navigate_on_mount: std::collections::HashSet<RemoteId>,
    pub transfers: TransferSnapshot,
    /// rclone binary download progress `(done, total)` while fetching.
    pub rclone_progress: Option<(u64, u64)>,
    pub ready: bool,
    pub last_error: Option<String>,
    pub show_add_dialog: bool,
    pub add_form: AddRemoteForm,
    pub show_transfers: bool,
    /// ID of the remote currently being renamed (`None` = overlay closed).
    pub renaming_remote_id: Option<RemoteId>,
}

impl CloudState {
    #[allow(dead_code)] // symmetric accessor; used by later-phase read paths
    pub fn remote(&self, id: &str) -> Option<&CloudRemote> {
        self.remotes.iter().find(|r| r.id == id)
    }
    pub fn remote_mut(&mut self, id: &str) -> Option<&mut CloudRemote> {
        self.remotes.iter_mut().find(|r| r.id == id)
    }
}
