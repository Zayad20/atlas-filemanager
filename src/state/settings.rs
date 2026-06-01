use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    CatppuccinMocha,
    Nord,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    Details,
    Grid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InspectorPlacement {
    Global,
    ActiveTab,
}

impl Default for InspectorPlacement {
    fn default() -> Self {
        Self::Global
    }
}

impl Default for ViewMode {
    fn default() -> Self {
        Self::Details
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_hidden: bool,
    pub show_sidebar: bool,
    pub show_inspector: bool,
    pub inspector_placement: InspectorPlacement,
    pub view_mode: ViewMode,
    pub sidebar_width: f32,
    pub inspector_width: f32,
    pub font_size: f32,
    pub thumbnail_size: f32,
    pub recent_folders: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            show_sidebar: true,
            show_inspector: false,
            inspector_placement: InspectorPlacement::Global,
            view_mode: ViewMode::Details,
            sidebar_width: 220.0,
            inspector_width: 280.0,
            font_size: 13.0,
            thumbnail_size: 128.0,
            recent_folders: Vec::new(),
        }
    }
}
