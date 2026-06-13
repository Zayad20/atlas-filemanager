#![allow(dead_code)]
// reason: retained API surface not exercised by every GPUI flow yet
// reason: small JSON config blobs read at startup / written on settings change;
// one-shot and tiny, not the gpui render hot path the sync-fs ban targets.
#![allow(clippy::disallowed_methods)]
//! On-disk persistence. Plain JSON files under the platform config dir. Each blob
//! is its own file so a single corrupt section can't take the rest down (a failed
//! parse falls back to `Default`).

use atlas_model::pane::TabData;
use atlas_model::profiles::ProfileManager;
use atlas_model::settings::Settings;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;

const APP_NAME: &str = "atlas";

pub fn config_dir() -> PathBuf {
    dirs::config_dir().map_or_else(|| PathBuf::from("."), |p| p.join(APP_NAME))
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("atlas")
}

/// Directory watched for user-supplied theme JSON files (gpui-component
/// `ThemeSet` format). Created on demand by the theme watcher.
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
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

pub fn load_session() -> Option<atlas_model::layout_tree::Tree<TabData>> {
    read_opt("session.json")
}

pub fn save_session(tree: &atlas_model::layout_tree::Tree<TabData>) {
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
