//! On-disk persistence. Plain JSON files under the platform config dir — replaces
//! eframe's storage so the app no longer depends on the egui/eframe runtime for
//! saving state. Each blob is its own file so a single corrupt section can't take
//! the rest down (a failed parse falls back to `Default`).

use crate::state::pane::TabData;
use crate::state::profiles::ProfileManager;
use crate::state::settings::Settings;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;

const APP_NAME: &str = "Atlas";

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|p| p.join(APP_NAME))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("egui-app")
}

/// Read a JSON blob, falling back to `Default` if it's missing or unparseable.
fn read_or_default<T: DeserializeOwned + Default>(file: &str) -> T {
    read_opt(file).unwrap_or_default()
}

/// Read an optional JSON blob — `None` if the file is missing or unparseable.
fn read_opt<T: DeserializeOwned>(file: &str) -> Option<T> {
    let path = config_dir().join(file);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!("failed to parse {}: {e}", path.display());
            None
        }
    }
}

fn write_json<T: Serialize>(file: &str, value: &T) {
    let dir = config_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("failed to create config dir {}: {e}", dir.display());
        return;
    }
    match serde_json::to_vec_pretty(value) {
        Ok(json) => {
            if let Err(e) = std::fs::write(dir.join(file), json) {
                tracing::warn!("failed to write {file}: {e}");
            }
        }
        Err(e) => tracing::warn!("failed to serialize {file}: {e}"),
    }
}

pub fn load_settings() -> Settings {
    read_or_default("settings.json")
}

pub fn save_settings(settings: &Settings) {
    write_json("settings.json", settings);
}

pub fn load_session() -> Option<crate::state::layout_tree::Tree<TabData>> {
    read_opt("session.json")
}

pub fn save_session(tree: &crate::state::layout_tree::Tree<TabData>) {
    write_json("session.json", tree);
}

pub fn load_profiles() -> ProfileManager {
    read_or_default("profiles.json")
}

pub fn save_profiles(profiles: &ProfileManager) {
    write_json("profiles.json", profiles);
}

/// Per-extension enabled overrides (id → enabled).
pub fn load_ext_enabled() -> std::collections::HashMap<String, bool> {
    read_or_default("ext_enabled.json")
}

pub fn save_ext_enabled(enabled: &std::collections::HashMap<String, bool>) {
    write_json("ext_enabled.json", enabled);
}

/// Configured cloud remotes (non-secret definitions only; credentials live in
/// rclone's own config). Empty when the feature is off or none are set.
#[cfg(feature = "cloud")]
pub fn load_cloud_remotes() -> Vec<crate::cloud::CloudRemote> {
    read_or_default("cloud_remotes.json")
}

#[cfg(feature = "cloud")]
pub fn save_cloud_remotes(remotes: &[crate::cloud::CloudRemote]) {
    write_json("cloud_remotes.json", &remotes.to_vec());
}
