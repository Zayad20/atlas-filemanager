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
            inspector_placement: InspectorPlacement::Global,
            view_mode: ViewMode::Details,
            sidebar_width: 220.0,
            inspector_width: 560.0,
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
    /// Legacy global recent folders. Migrated into per-profile prefs at startup.
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
    /// Show Atlas' custom top window bar. Turn off when the platform/window manager
    /// already provides chrome and the extra in-app strip is unwanted.
    pub show_titlebar: bool,
    /// When true, evicts thumbnails and shrinks collections when the window is inactive.
    pub eco_mode: bool,
    /// When true, the pane/tab layout is persisted to `session.json` when the
    /// window closes, so the next launch restores it. Off = the last explicit
    /// save (theme change, manual "Save Session" command) still applies.
    pub autosave_session: bool,
    /// Visually de-emphasize panes other than the focused pane.
    pub dim_inactive_panes: bool,
    /// Background opacity for Glass themes (0.20 to 1.00, default 0.80).
    #[serde(default = "default_glass_opacity")]
    pub glass_opacity: f32,
    /// True after the first-run setup has been dismissed or completed.
    pub getting_started_seen: bool,
}

pub fn default_glass_opacity() -> f32 {
    0.80
}

impl Settings {
    /// Snapshot the per-profile view/layout knobs out of the live settings.
    pub fn workspace(&self) -> Workspace {
        Workspace {
            show_hidden: self.show_hidden,
            show_sidebar: self.show_sidebar,
            sidebar_collapsed: self.sidebar_collapsed,
            show_inspector: self.show_inspector,
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
    /// fields (`language`, `disable_animations`) are left intact.
    pub fn apply_workspace(&mut self, ws: &Workspace) {
        self.show_hidden = ws.show_hidden;
        self.show_sidebar = ws.show_sidebar;
        self.sidebar_collapsed = ws.sidebar_collapsed;
        self.show_inspector = ws.show_inspector;
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
            inspector_width: 560.0,
            font_size: 13.0,
            font_family: "Geist".to_string(),
            thumbnail_size: 128.0,
            recent_folders: Vec::new(),
            global_zoom: 1.0,
            language: None,
            disable_animations: false,
            scroll_smoothness: 0.6,
            show_titlebar: true,
            eco_mode: true,
            autosave_session: true,
            dim_inactive_panes: false,
            glass_opacity: default_glass_opacity(),
            getting_started_seen: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_glass_opacity_when_absent() {
        let json = r#"{"font_size": 13.0}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!((s.glass_opacity - 0.80).abs() < 1e-4);
    }

    #[test]
    fn settings_preserves_custom_glass_opacity() {
        let json = r#"{"glass_opacity": 0.55}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!((s.glass_opacity - 0.55).abs() < 1e-4);
    }
}

