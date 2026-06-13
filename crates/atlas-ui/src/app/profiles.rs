use crate::app::App;
use crate::state::layout_tree::{Tile, Tree};
use crate::state::pane::TabData;
use std::path::PathBuf;

impl App {
    pub(crate) fn switch_profile(&mut self, idx: usize) {
        if idx == self.state.profiles.active_idx || idx >= self.state.profiles.profiles.len() {
            return;
        }

        // Snapshot current tree + view/layout knobs into the active profile
        // before leaving, so they restore when we come back.
        let leaving_ws = self.state.settings.workspace();
        {
            let leaving = self.state.profiles.active_mut();
            leaving.tree = Some(self.state.pane_tree.clone());
            leaving.workspace = leaving_ws;
        }

        // Drop all FS watchers from the current session
        self.watchers.clear();

        self.state.profiles.active_idx = idx;
        // Overlay the entering profile's saved view/layout knobs onto live settings.
        let entering_ws = self.state.profiles.active().workspace.clone();
        self.state.settings.apply_workspace(&entering_ws);
        // Swap the icon-theme mappings to the entering profile's selection.
        self.apply_profile_icons();
        // Each profile carries its own theme — apply it on next render.
        self.theme_dirty = true;
        // Re-apply the entering profile's sidebar width to the left dock next render.
        self.sidebar_width_dirty = true;
        self.state.profile_ui.adding_bookmark = false;
        self.state.profile_ui.adding_tag = false;
        self.state.profile_ui.creating_tag = false;

        // Restore the target profile's tree, or create a fresh single-tab home pane
        let new_tree = self
            .state
            .profiles
            .active()
            .tree
            .clone()
            .unwrap_or_else(|| {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                let tab_data = TabData::with_view(
                    home,
                    self.state.settings.view_mode.clone(),
                    self.state.settings.thumbnail_size,
                    self.state.settings.show_inspector,
                );
                let mut tree = Tree::empty();
                let pane_id = tree.tiles.insert_pane(tab_data);
                let tabs_id = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(tabs_id);
                tree
            });

        self.state.pane_tree = new_tree;
        // The Dock caches FilePanels keyed by TileId; without advancing the layout
        // revision `ensure_dock` would keep rendering the previous profile's panes
        // (now dangling tile ids → "Carpeta vacía" / blank). Force a center rebuild.
        self.layout_revision += 1;
        self.state.tab_states.clear();
        self.state.preview.clear_all();
        self.pending_preview = None;

        let pane_ids = self.state.all_pane_tile_ids();
        if let Some(&first) = pane_ids.first() {
            self.state.focused_tile = first;
        }

        // Kick off directory loads for every pane in the restored tree
        let show_hidden = self.state.settings.show_hidden;
        for tile_id in pane_ids {
            let (loc, sort_col, sort_dir, folders_first) =
                match self.state.pane_tree.tiles.get(tile_id) {
                    Some(Tile::Pane(p)) => (
                        p.location.clone(),
                        p.sort_col.clone(),
                        p.sort_dir.clone(),
                        p.folders_first,
                    ),
                    _ => continue,
                };
            let path = match loc.fs_dir() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };
            let op_id = self.state.next_op_id();
            let ts = self.state.tab_states.entry(tile_id).or_default();
            ts.load_state = crate::state::tab::LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = self.tx.clone();
            self.rt.spawn(async move {
                match crate::fs::list::list_dir(&path, show_hidden, sort_col, sort_dir, folders_first)
                    .await
                {
                    Ok(entries) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }

        // Persist the swap immediately so a crash before window close keeps the
        // active profile and its restored view/layout.
        crate::util::config::save_profiles(&self.state.profiles);
        crate::util::config::save_settings(&self.state.settings);
    }

    /// Create a new profile seeded from the *current* view/layout and theme, then
    /// switch to it. The fresh profile starts with no saved tree, so the switch
    /// opens a clean home pane.
    pub(crate) fn create_profile(&mut self, name: impl Into<String>) {
        let name = name.into();
        let name = if name.trim().is_empty() {
            self.unique_profile_name()
        } else {
            name.trim().to_string()
        };
        let mut profile = crate::state::profiles::Profile::new(name);
        // Inherit the current look so a new profile doesn't snap to hard defaults.
        profile.workspace = self.state.settings.workspace();
        profile.theme = self.state.profiles.active().theme.clone();
        self.state.profiles.profiles.push(profile);
        let new_idx = self.state.profiles.profiles.len() - 1;
        self.switch_profile(new_idx);
    }

    /// Build a fresh profile name like "Perfil 2" that doesn't collide with an
    /// existing one.
    fn unique_profile_name(&self) -> String {
        let taken: std::collections::HashSet<&str> = self
            .state
            .profiles
            .profiles
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        (1..)
            .map(|n| rust_i18n::t!("profile_default_name", n = n).to_string())
            .find(|name| !taken.contains(name.as_str()))
            .unwrap_or_else(|| rust_i18n::t!("profile_default_name", n = 1).to_string())
    }

    /// Rename the active profile. Empty/whitespace names are ignored.
    pub(crate) fn rename_active_profile(&mut self, name: impl Into<String>) {
        let name = name.into();
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return;
        }
        self.state.profiles.active_mut().name = trimmed.to_string();
        crate::util::config::save_profiles(&self.state.profiles);
    }

    pub(crate) fn delete_active_profile(&mut self) {
        if self.state.profiles.profiles.len() <= 1 {
            return;
        }
        let del_idx = self.state.profiles.active_idx;
        self.state.profiles.profiles.remove(del_idx);
        let new_idx = if del_idx >= self.state.profiles.profiles.len() {
            self.state.profiles.profiles.len() - 1
        } else {
            del_idx
        };
        // active_idx must differ from new_idx to trigger a real switch,
        // so temporarily place it out of range before calling switch_profile.
        self.state.profiles.active_idx = usize::MAX;
        self.switch_profile(new_idx);
    }
}

/// Official per-profile icon-theme selection: the `extension → SVG-path` mappings
/// that override the built-in file-type icons. Produced by the `icon-manager`
/// extension, persisted per profile through `ProfilePrefs`, and rebuilt into the
/// live `IconThemeIndex` (resvg pipeline) on every profile switch.
///
/// First concrete consumer of the `ProfileScoped` contract — the blueprint any
/// other "persist this per profile" feature should copy.
pub(crate) struct IconSelection;

impl crate::state::profiles::ProfileScoped for IconSelection {
    const KEY: &'static str = "icons";
    type State = std::collections::HashMap<String, String>;
}

impl App {
    /// Build an [`IconThemeIndex`] from the active profile's saved icon mappings,
    /// falling back to VSCode auto-discovery when the profile has no selection.
    /// Shared by startup and live switching so both paths agree.
    pub(crate) fn icon_theme_for_active_profile(
        &self,
    ) -> Option<crate::icon_theme::IconThemeIndex> {
        let map = self.state.profiles.active().scoped::<IconSelection>();
        if map.is_empty() {
            crate::icon_theme::find_vscode_theme()
        } else {
            let paths = map
                .into_iter()
                .map(|(ext, path)| (ext, std::path::PathBuf::from(path)))
                .collect();
            crate::icon_theme::IconThemeIndex::from_mappings(paths)
        }
    }

    /// Swap the live icon theme to the active profile's selection and drop the
    /// rasterised cache so icons re-render through the resvg pipeline (the same
    /// one VSCode themes use). Callers re-list affected dirs to re-warm the cache.
    pub(crate) fn apply_profile_icons(&mut self) {
        self.icon_theme = self.icon_theme_for_active_profile();
        self.icon_cache.clear();
    }

    /// Record icon mappings just produced by an extension into the active profile,
    /// persist, and apply them live.
    pub(crate) fn persist_icon_selection(
        &mut self,
        map: std::collections::HashMap<String, String>,
    ) {
        self.state
            .profiles
            .active_mut()
            .set_scoped::<IconSelection>(&map);
        crate::util::config::save_profiles(&self.state.profiles);
        self.apply_profile_icons();
    }

    /// Official per-profile variable store for an extension. Variables live as a
    /// JSON object under the `ext:<id>` namespace in the active profile's
    /// [`ProfilePrefs`], so each profile keeps its own extension configuration.
    ///
    /// This is the sanctioned way for an extension to persist settings that should
    /// follow the profile (the icon-manager's mappings are the worked example via
    /// [`App::persist_icon_selection`]). The WASM host-function bridge that lets a
    /// plugin call these is the remaining wiring.
    #[allow(dead_code)] // reason: official API surface; WASM bridge wires callers next.
    pub(crate) fn ext_var(&self, ext_id: &str, key: &str) -> Option<serde_json::Value> {
        self.state
            .profiles
            .active()
            .prefs
            .get::<serde_json::Map<String, serde_json::Value>>(&format!("ext:{ext_id}"))
            .and_then(|vars| vars.get(key).cloned())
    }

    #[allow(dead_code)] // reason: official API surface; WASM bridge wires callers next.
    pub(crate) fn set_ext_var(&mut self, ext_id: &str, key: &str, value: serde_json::Value) {
        let ns = format!("ext:{ext_id}");
        let mut vars = self
            .state
            .profiles
            .active()
            .prefs
            .get::<serde_json::Map<String, serde_json::Value>>(&ns)
            .unwrap_or_default();
        vars.insert(key.to_string(), value);
        self.state.profiles.active_mut().prefs.set(&ns, &vars);
        crate::util::config::save_profiles(&self.state.profiles);
    }
}
