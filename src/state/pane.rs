use crate::state::file_entry::{SortColumn, SortDir};
use crate::state::settings::ViewMode;
use std::path::PathBuf;

pub type PaneId = egui_tiles::TileId;
pub type TabId = egui_tiles::TileId;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TabData {
    pub path: PathBuf,
    pub sort_col: SortColumn,
    pub sort_dir: SortDir,
    pub history: Vec<PathBuf>,
    pub history_forward: Vec<PathBuf>,
    #[serde(default)]
    pub view_mode: ViewMode,
    #[serde(default = "default_thumbnail_size")]
    pub thumbnail_size: f32,
    #[serde(default)]
    pub preview_enabled: bool,
}

impl TabData {
    pub fn with_view(
        path: PathBuf,
        view_mode: ViewMode,
        thumbnail_size: f32,
        preview_enabled: bool,
    ) -> Self {
        Self {
            path,
            sort_col: SortColumn::default(),
            sort_dir: SortDir::default(),
            history: Vec::new(),
            history_forward: Vec::new(),
            view_mode,
            thumbnail_size,
            preview_enabled,
        }
    }
}

fn default_thumbnail_size() -> f32 {
    128.0
}

#[cfg(test)]
mod tests {
    use super::TabData;
    use crate::state::settings::ViewMode;
    use std::path::PathBuf;

    #[test]
    fn persists_view_per_tab() {
        let tab = TabData::with_view(PathBuf::from("/tmp"), ViewMode::Grid, 192.0, true);
        let json = serde_json::to_string(&tab).unwrap();
        let restored: TabData = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.view_mode, ViewMode::Grid);
        assert_eq!(restored.thumbnail_size, 192.0);
        assert!(restored.preview_enabled);
    }

    #[test]
    fn restores_tabs_saved_before_preview_was_per_tab() {
        let json = r#"{
            "path":"/tmp",
            "sort_col":"Name",
            "sort_dir":"Asc",
            "history":[],
            "history_forward":[],
            "view_mode":"Details",
            "thumbnail_size":128.0
        }"#;
        let restored: TabData = serde_json::from_str(json).unwrap();

        assert!(!restored.preview_enabled);
    }
}
