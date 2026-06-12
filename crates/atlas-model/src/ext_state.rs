//! UI-facing state for the extension system.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ext::{CommandRef, Registry};

/// A command awaiting prompt answers before it runs.
pub struct PendingCommand {
    pub cmd: CommandRef,
    pub paths: Vec<PathBuf>,
    /// `(prompt name, label, current value)` in manifest order.
    pub answers: Vec<(String, String, String)>,
}

#[derive(Default)]
pub struct ExtState {
    pub registry: Registry,
    /// Persisted per-extension enabled overrides (id → enabled).
    pub enabled: HashMap<String, bool>,
    pub show_manager: bool,
    /// A command collecting prompt answers in a modal before running.
    pub pending: Option<PendingCommand>,
    pub last_error: Option<String>,
    pub feedback: Option<String>,
}

impl ExtState {
    /// Rebuild the registry from disk using the current enabled overrides.
    pub fn reload(&mut self) {
        self.registry = Registry::load(&self.enabled);
    }
}
