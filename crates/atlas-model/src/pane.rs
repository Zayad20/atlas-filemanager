use crate::state::file_entry::{SortColumn, SortDir};
use crate::state::settings::ViewMode;
use crate::state::tag_cache::TagId;
use std::path::PathBuf;

pub type PaneId = crate::state::layout_tree::TileId;
pub type TabId = crate::state::layout_tree::TileId;

/// What a tab is showing. Normal tabs list a real directory; virtual tabs (such as
/// tag results) get their entries from somewhere other than `read_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TabSource {
    /// List the real directory at `TabData.path`.
    #[default]
    Directory,
    /// Show every file carrying the given tag. `TabData.path` is a display-only sentinel.
    Tag(TagId),
    /// Browse a cloud/network remote in Access Mode (no FUSE mount). `id` is the
    /// rclone remote name; `path` is the remote-relative directory being shown.
    /// `TabData.path` is a display-only sentinel.
    #[cfg(feature = "cloud")]
    Remote { id: String, path: String },
}

impl TabSource {
    pub fn is_directory(&self) -> bool {
        matches!(self, TabSource::Directory)
    }
}

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
    /// What this tab shows (real directory vs. virtual results). Defaults to
    /// `Directory` so sessions saved before this field deserialize unchanged.
    #[serde(default)]
    pub source: TabSource,
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
            source: TabSource::Directory,
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
