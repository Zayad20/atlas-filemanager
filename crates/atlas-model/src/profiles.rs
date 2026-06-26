#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::pane::TabData;
use std::collections::HashMap;
use std::path::PathBuf;

/// Namespaced, per-profile preference store. The *official* way for features and
/// extensions to persist data scoped to a profile without growing [`Profile`]
/// with one bespoke field per feature.
///
/// Key = stable namespace id (`"icons"`, `"ext:<id>"`, …). Value = arbitrary JSON.
/// Reach for the typed [`ProfilePrefs::get`] / [`ProfilePrefs::set`] helpers, or
/// the [`ProfileScoped`] trait when a feature owns a fixed key + state type.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ProfilePrefs(HashMap<String, serde_json::Value>);

impl ProfilePrefs {
    /// Deserialize the value stored under `key` as `T`. `None` when absent or when
    /// the stored JSON no longer matches `T` (e.g. after a feature schema change),
    /// so callers can fall back to a default without panicking.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_value(self.0.get(key)?.clone()).ok()
    }

    /// Store `val` under `key`, replacing any previous value. A serialization
    /// failure is dropped rather than propagated — prefs are best-effort and must
    /// never take down a save.
    pub fn set<T: serde::Serialize>(&mut self, key: &str, val: &T) {
        if let Ok(v) = serde_json::to_value(val) {
            self.0.insert(key.to_string(), v);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A feature (or extension) that owns a fixed [`ProfilePrefs`] namespace and a
/// serializable state type. Implement on a zero-sized marker to get ergonomic,
/// typed per-profile persistence via [`Profile::scoped`] / [`Profile::set_scoped`].
///
/// This is the contract the "icon selection" and per-extension variable stores
/// build on, and the recommended entry point for any new persisted-per-profile
/// feature.
pub trait ProfileScoped {
    /// Stable namespace key in [`ProfilePrefs`]. Must be unique across features.
    const KEY: &'static str;
    /// Serializable state persisted under [`Self::KEY`].
    type State: serde::Serialize + serde::de::DeserializeOwned + Default;
}

/// Persisted preferences for the preview / inspector panel, scoped per profile.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PreviewPrefs {
    /// Show line numbers in the code editor. Defaults to `true`.
    #[serde(default = "default_true")]
    pub show_line_numbers: bool,
    /// Open files in edit mode (editable) by default. Defaults to `false`.
    #[serde(default)]
    pub edit_mode: bool,
}

fn default_true() -> bool {
    true
}

/// Marker for [`ProfileScoped`] — stores [`PreviewPrefs`] under key `"preview"`.
pub struct PreviewFeature;

impl ProfileScoped for PreviewFeature {
    const KEY: &'static str = "preview";
    type State = PreviewPrefs;
}

/// Name of the theme applied to new profiles. Must match a theme registered in
/// `ThemeRegistry` (see `crates/atlas-ui/assets/themes/atlas-themes.json`).
pub const DEFAULT_THEME: &str = "Atlas Figma Glass";

fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SectionKind {
    Places,
    Bookmarks,
    Tags,
    Drives,
    Recent,
    /// Cloud / network remotes (feature `cloud`). Renders nothing without it.
    #[serde(other)]
    Cloud,
}

impl SectionKind {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Places => "Places",
            Self::Bookmarks => "Bookmarks",
            Self::Tags => "Tags",
            Self::Drives => "Drives",
            Self::Recent => "Recent",
            Self::Cloud => "Network",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SidebarSection {
    pub kind: SectionKind,
    pub collapsed: bool,
    #[serde(default)]
    pub hidden: bool,
}

impl SidebarSection {
    pub const fn new(kind: SectionKind) -> Self {
        Self {
            kind,
            collapsed: false,
            hidden: false,
        }
    }
}

pub fn default_sections() -> Vec<SidebarSection> {
    vec![
        SidebarSection::new(SectionKind::Places),
        SidebarSection::new(SectionKind::Bookmarks),
        SidebarSection::new(SectionKind::Tags),
        SidebarSection::new(SectionKind::Drives),
        #[cfg(feature = "cloud")]
        SidebarSection::new(SectionKind::Cloud),
        SidebarSection::new(SectionKind::Recent),
    ]
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    pub name: String,
    /// RGB color
    pub color: [u8; 3],
    /// If true this tag lives in `global_tags` rather than a profile
    pub global: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub name: String,
    pub bookmarks: Vec<Bookmark>,
    pub tags: Vec<Tag>,
    /// Saved pane/tab tree for this profile. None = fresh session on first switch.
    #[serde(default)]
    pub tree: Option<crate::layout_tree::Tree<TabData>>,
    /// Sidebar section order and collapsed state, per profile.
    #[serde(default = "default_sections")]
    pub sidebar_sections: Vec<SidebarSection>,
    /// Visual theme for this profile, by registered theme name. Legacy enum
    /// values (e.g. "Dark") deserialize as plain strings and are normalized to a
    /// registered theme at apply time, falling back to [`DEFAULT_THEME`].
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When true, the goto bar searches inside file *contents* (grep) instead of
    /// fuzzy-matching folder names. Persisted per profile.
    #[serde(default)]
    pub search_in_content: bool,
    /// Command-palette favorites: stable command ids pinned to the "Quick"
    /// section. Scoped per profile.
    #[serde(default)]
    pub pinned_commands: Vec<String>,
    /// Per-profile view/layout knobs (sizes, view mode, panel visibility, zoom).
    /// Swapped in/out of the live [`crate::settings::Settings`] on profile switch.
    #[serde(default)]
    pub workspace: crate::settings::Workspace,
    /// Namespaced store for feature/extension state scoped to this profile. The
    /// official extension point — see [`ProfilePrefs`] / [`ProfileScoped`].
    #[serde(default)]
    pub prefs: ProfilePrefs,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bookmarks: Vec::new(),
            tags: Vec::new(),
            tree: None,
            sidebar_sections: default_sections(),
            theme: default_theme(),
            search_in_content: false,
            pinned_commands: Vec::new(),
            workspace: crate::settings::Workspace::default(),
            prefs: ProfilePrefs::default(),
        }
    }

    /// Read this profile's persisted state for feature `S`, defaulting when unset
    /// or unparseable.
    pub fn scoped<S: ProfileScoped>(&self) -> S::State {
        self.prefs.get::<S::State>(S::KEY).unwrap_or_default()
    }

    /// Persist this profile's state for feature `S` under its namespace.
    pub fn set_scoped<S: ProfileScoped>(&mut self, state: &S::State) {
        self.prefs.set(S::KEY, state);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileManager {
    pub profiles: Vec<Profile>,
    pub active_idx: usize,
    pub global_tags: Vec<Tag>,
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self {
            profiles: vec![Profile::new("Default")],
            active_idx: 0,
            global_tags: Vec::new(),
        }
    }
}

impl ProfileManager {
    pub fn active(&self) -> &Profile {
        &self.profiles[self.active_idx]
    }

    pub fn active_mut(&mut self) -> &mut Profile {
        &mut self.profiles[self.active_idx]
    }

    /// Ensure every profile exposes the Network/Cloud sidebar section. Profiles
    /// persisted before the cloud feature existed lack it, which would otherwise
    /// hide all configured remotes. Inserts it just before "Recent".
    #[cfg(feature = "cloud")]
    pub fn ensure_cloud_section(&mut self) {
        for p in &mut self.profiles {
            if p.sidebar_sections
                .iter()
                .any(|s| s.kind == SectionKind::Cloud)
            {
                continue;
            }
            let pos = p
                .sidebar_sections
                .iter()
                .position(|s| s.kind == SectionKind::Recent)
                .unwrap_or(p.sidebar_sections.len());
            p.sidebar_sections
                .insert(pos, SidebarSection::new(SectionKind::Cloud));
        }
    }
}

/// Transient UI state for the sidebar profile/bookmark/tag forms.
/// Not persisted.
#[derive(Default)]
pub struct ProfileUiState {
    pub adding_bookmark: bool,
    pub new_bookmark_name: String,
    pub adding_tag: bool,
    pub creating_tag: bool,
    pub editing_tag: Option<crate::tag_cache::TagId>,
    pub new_tag_global: bool,
    pub new_tag_name: String,
    pub new_tag_color: [u8; 3],
    pub confirming_delete_profile: Option<usize>,
    pub atlas_launcher_profiles_open: bool,
}

pub const PRESET_COLORS: &[[u8; 3]] = &[
    [239, 68, 68],   // red
    [249, 115, 22],  // orange
    [234, 179, 8],   // yellow
    [34, 197, 94],   // green
    [59, 130, 246],  // blue
    [168, 85, 247],  // purple
    [236, 72, 153],  // pink
    [148, 163, 184], // slate
];

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct IconSel {
        theme_id: Option<String>,
    }

    struct IconFeature;
    impl ProfileScoped for IconFeature {
        const KEY: &'static str = "icons";
        type State = IconSel;
    }

    #[test]
    fn prefs_round_trip_typed() {
        let mut prefs = ProfilePrefs::default();
        assert!(prefs.is_empty());
        let val = IconSel {
            theme_id: Some("flow-icons".into()),
        };
        prefs.set("icons", &val);
        assert!(prefs.contains("icons"));
        assert_eq!(prefs.get::<IconSel>("icons"), Some(val));
    }

    #[test]
    fn prefs_get_absent_is_none() {
        let prefs = ProfilePrefs::default();
        assert_eq!(prefs.get::<IconSel>("icons"), None);
    }

    #[test]
    fn prefs_schema_mismatch_is_none_not_panic() {
        let mut prefs = ProfilePrefs::default();
        prefs.set("icons", &"a bare string, not IconSel");
        assert_eq!(prefs.get::<IconSel>("icons"), None);
    }

    #[test]
    fn scoped_helpers_default_then_persist() {
        let mut profile = Profile::new("Test");
        // Unset → default.
        assert_eq!(profile.scoped::<IconFeature>(), IconSel::default());
        profile.set_scoped::<IconFeature>(&IconSel {
            theme_id: Some("material".into()),
        });
        assert_eq!(
            profile.scoped::<IconFeature>().theme_id.as_deref(),
            Some("material")
        );
    }

    #[test]
    fn profile_prefs_survive_serde() {
        let mut profile = Profile::new("Persisted");
        profile.set_scoped::<IconFeature>(&IconSel {
            theme_id: Some("flow".into()),
        });
        let json = serde_json::to_string(&profile).unwrap();
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.scoped::<IconFeature>().theme_id.as_deref(),
            Some("flow")
        );
    }

    #[test]
    fn legacy_profile_without_prefs_deserializes() {
        // A profile JSON predating the `prefs` field must still load (serde default).
        let legacy = r#"{"name":"Old","bookmarks":[],"tags":[]}"#;
        let p: Profile = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.name, "Old");
        assert!(p.prefs.is_empty());
    }
}
