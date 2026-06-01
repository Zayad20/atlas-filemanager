use crate::state::pane::TabData;
use crate::state::profiles::ProfileManager;
use crate::state::settings::Settings;
use std::path::PathBuf;

#[allow(dead_code)]
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("egui-app")
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

pub fn load_session(storage: &dyn eframe::Storage) -> Option<egui_tiles::Tree<TabData>> {
    eframe::get_value(storage, "atlas_session")
}

pub fn save_session(storage: &mut dyn eframe::Storage, tree: &egui_tiles::Tree<TabData>) {
    eframe::set_value(storage, "atlas_session", tree);
}

pub fn load_profiles(storage: &dyn eframe::Storage) -> ProfileManager {
    eframe::get_value(storage, "atlas_profiles").unwrap_or_default()
}

pub fn save_profiles(storage: &mut dyn eframe::Storage, profiles: &ProfileManager) {
    eframe::set_value(storage, "atlas_profiles", profiles);
}
