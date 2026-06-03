use crate::state::pane::TabData;
use crate::state::settings::Theme;
use std::path::PathBuf;

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
    pub fn label(&self) -> &'static str {
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
}

impl SidebarSection {
    pub fn new(kind: SectionKind) -> Self {
        Self {
            kind,
            collapsed: false,
        }
    }
}

fn default_sections() -> Vec<SidebarSection> {
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
    /// If true this tag lives in global_tags rather than a profile
    pub global: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub name: String,
    pub bookmarks: Vec<Bookmark>,
    pub tags: Vec<Tag>,
    /// Saved pane/tab tree for this profile. None = fresh session on first switch.
    #[serde(default)]
    pub tree: Option<egui_tiles::Tree<TabData>>,
    /// Sidebar section order and collapsed state, per profile.
    #[serde(default = "default_sections")]
    pub sidebar_sections: Vec<SidebarSection>,
    /// Visual theme for this profile.
    #[serde(default)]
    pub theme: Theme,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bookmarks: Vec::new(),
            tags: Vec::new(),
            tree: None,
            sidebar_sections: default_sections(),
            theme: Theme::default(),
        }
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
            if p.sidebar_sections.iter().any(|s| s.kind == SectionKind::Cloud) {
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
    pub new_tag_global: bool,
    pub new_tag_name: String,
    pub new_tag_color: [u8; 3],
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
