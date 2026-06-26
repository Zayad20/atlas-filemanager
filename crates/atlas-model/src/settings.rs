use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Details,
    Grid,
    List,
    Miller,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum InspectorPlacement {
    #[default]
    Global,
    ActiveTab,
}

/// The subset of view/layout knobs that are saved *per profile* (sizes, view
/// mode, panel visibility, zoom). [`Settings`] holds the live working copy the UI
/// reads; switching profiles swaps this block in and out via
/// [`Settings::workspace`] / [`Settings::apply_workspace`], so call sites keep
/// reading `settings.*` unchanged.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub show_hidden: bool,
    pub show_sidebar: bool,
    pub sidebar_collapsed: bool,
    pub show_inspector: bool,
    pub mica: bool,
    pub inspector_placement: InspectorPlacement,
    pub view_mode: ViewMode,
    pub sidebar_width: f32,
    pub inspector_width: f32,
    pub font_size: f32,
    pub font_family: String,
    pub thumbnail_size: f32,
    pub global_zoom: f32,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            show_hidden: false,
            show_sidebar: true,
            sidebar_collapsed: false,
            show_inspector: false,
            mica: true,
            inspector_placement: InspectorPlacement::Global,
            view_mode: ViewMode::Details,
            sidebar_width: 220.0,
            inspector_width: 280.0,
            font_size: 13.0,
            font_family: "Geist".to_string(),
            thumbnail_size: 128.0,
            global_zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_hidden: bool,
    pub show_sidebar: bool,
    pub sidebar_collapsed: bool,
    pub show_inspector: bool,
    pub inspector_placement: InspectorPlacement,
    pub view_mode: ViewMode,
    pub sidebar_width: f32,
    pub inspector_width: f32,
    pub font_size: f32,
    pub font_family: String,
    pub thumbnail_size: f32,
    pub recent_folders: Vec<PathBuf>,
    pub global_zoom: f32,
    pub language: Option<String>,
    /// When true, all with_animation calls are bypassed: elements render at their
    /// final state immediately. Eliminates the per-frame repaint cost of running
    /// animations across multiple panes.
    pub disable_animations: bool,
    /// Mouse-wheel scroll smoothness, 0.0 (instant, no easing) … 1.0 (long silky
    /// glide). Maps to the gpui scroll handle's ease + per-notch boost. Global, like
    /// `disable_animations`. Trackpad (pixel-precise) input is unaffected.
    pub scroll_smoothness: f32,
    /// When true, the window uses GPUI's blurred background and `apply_active_theme`
    /// transparently swaps to the Glass variant of the active theme so the wallpaper
    /// shows through translucent panels. Saved per profile through [`Workspace`].
    pub mica: bool,
}

impl Settings {
    /// Snapshot the per-profile view/layout knobs out of the live settings.
    pub fn workspace(&self) -> Workspace {
        Workspace {
            show_hidden: self.show_hidden,
            show_sidebar: self.show_sidebar,
            sidebar_collapsed: self.sidebar_collapsed,
            show_inspector: self.show_inspector,
            mica: self.mica,
            inspector_placement: self.inspector_placement.clone(),
            view_mode: self.view_mode.clone(),
            sidebar_width: self.sidebar_width,
            inspector_width: self.inspector_width,
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            thumbnail_size: self.thumbnail_size,
            global_zoom: self.global_zoom,
        }
    }

    /// Overlay a profile's saved knobs onto the live settings. Truly global
    /// fields (`language`, `recent_folders`, `disable_animations`) are left intact.
    pub fn apply_workspace(&mut self, ws: &Workspace) {
        self.show_hidden = ws.show_hidden;
        self.show_sidebar = ws.show_sidebar;
        self.sidebar_collapsed = ws.sidebar_collapsed;
        self.show_inspector = ws.show_inspector;
        self.mica = ws.mica;
        self.inspector_placement = ws.inspector_placement.clone();
        self.view_mode = ws.view_mode.clone();
        self.sidebar_width = ws.sidebar_width;
        self.inspector_width = ws.inspector_width;
        self.font_size = ws.font_size;
        self.font_family.clone_from(&ws.font_family);
        self.thumbnail_size = ws.thumbnail_size;
        self.global_zoom = ws.global_zoom;
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            show_sidebar: true,
            sidebar_collapsed: false,
            show_inspector: false,
            inspector_placement: InspectorPlacement::Global,
            view_mode: ViewMode::Details,
            sidebar_width: 220.0,
            inspector_width: 280.0,
            font_size: 13.0,
            font_family: "Geist".to_string(),
            thumbnail_size: 128.0,
            recent_folders: Vec::new(),
            global_zoom: 1.0,
            language: None,
            disable_animations: false,
            scroll_smoothness: 0.6,
            mica: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn workspace_carries_mica() {
        let mut settings = Settings {
            mica: false,
            ..Settings::default()
        };
        let ws = settings.workspace();

        settings.mica = true;
        settings.apply_workspace(&ws);

        assert!(!settings.mica);
    }
}
