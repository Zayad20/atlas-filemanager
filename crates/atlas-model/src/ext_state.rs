//! UI-facing state for the extension system.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ext_model::{CommandRef, RemoteExtension};

/// A command awaiting prompt answers before it runs.
pub struct PendingCommand {
    pub cmd: CommandRef,
    pub paths: Vec<PathBuf>,
    /// `(prompt name, label, current value)` in manifest order.
    pub answers: Vec<(String, String, String)>,
}

/// Which tab of the extension manager is visible.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum ExtManagerTab {
    #[default]
    Installed,
    Marketplace,
}

#[derive(Default)]
pub struct ExtState {
    /// Persisted per-extension enabled overrides (id → enabled).
    pub enabled: HashMap<String, bool>,
    pub show_manager: bool,
    /// Active tab in the extension manager panel.
    pub manager_tab: ExtManagerTab,
    /// Cached marketplace catalogue from R2 registry.json.
    pub remote: Option<Vec<RemoteExtension>>,
    /// True while a registry fetch is in flight.
    pub remote_loading: bool,
    /// Extension IDs currently being downloaded + installed.
    pub installing: HashSet<String>,
    /// A command collecting prompt answers in a modal before running.
    pub pending: Option<PendingCommand>,
    pub last_error: Option<String>,
    pub feedback: Option<String>,
}
