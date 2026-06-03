#[allow(dead_code)]
use crate::state::pane::TabData;
use crate::state::profiles::ProfileManager;
use crate::state::settings::Settings;
use std::path::PathBuf;

const APP_NAME: &str = "Atlas";

#[allow(dead_code)]
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|p| p.join(APP_NAME))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[allow(dead_code)]
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("egui-app")
}

pub fn load_settings(storage: &dyn eframe::Storage) -> Settings {
    eframe::get_value(storage, "atlas_settings").unwrap_or_default()
}

pub fn save_settings(storage: &mut dyn eframe::Storage, settings: &Settings) {
    eframe::set_value(storage, "atlas_settings", settings);
}

pub fn load_session(
    storage: &dyn eframe::Storage,
) -> Option<crate::state::layout_tree::Tree<TabData>> {
    eframe::get_value(storage, "atlas_session")
}

pub fn save_session(
    storage: &mut dyn eframe::Storage,
    tree: &crate::state::layout_tree::Tree<TabData>,
) {
    eframe::set_value(storage, "atlas_session", tree);
}

pub fn load_profiles(storage: &dyn eframe::Storage) -> ProfileManager {
    eframe::get_value(storage, "atlas_profiles").unwrap_or_default()
}

pub fn save_profiles(storage: &mut dyn eframe::Storage, profiles: &ProfileManager) {
    eframe::set_value(storage, "atlas_profiles", profiles);
}

/// Per-extension enabled overrides (id → enabled).
pub fn load_ext_enabled(storage: &dyn eframe::Storage) -> std::collections::HashMap<String, bool> {
    eframe::get_value(storage, "atlas_ext_enabled").unwrap_or_default()
}

pub fn save_ext_enabled(
    storage: &mut dyn eframe::Storage,
    enabled: &std::collections::HashMap<String, bool>,
) {
    eframe::set_value(storage, "atlas_ext_enabled", enabled);
}

/// Configured cloud remotes (non-secret definitions only; credentials live in
/// rclone's own config). Empty when the feature is off or none are set.
#[cfg(feature = "cloud")]
pub fn load_cloud_remotes(storage: &dyn eframe::Storage) -> Vec<crate::cloud::CloudRemote> {
    eframe::get_value(storage, "atlas_cloud_remotes").unwrap_or_default()
}

#[cfg(feature = "cloud")]
pub fn save_cloud_remotes(storage: &mut dyn eframe::Storage, remotes: &[crate::cloud::CloudRemote]) {
    eframe::set_value(storage, "atlas_cloud_remotes", &remotes.to_vec());
}
