//! Theme application glue. The active profile stores a theme *name*; here we map
//! it onto gpui-component's global `Theme` via the `ThemeRegistry`. Applying needs
//! `&mut gpui::App` + a `Window`, which `process_actions` doesn't have, so callers
//! (palette, settings, profile switch) just set the name and flip `theme_dirty`;
//! the next render calls [`App::apply_active_theme`].

use crate::app::App;
use atlas_model::profiles::DEFAULT_THEME;
use gpui::{App as GpuiApp, SharedString, Window};
use gpui_component::{Theme, ThemeRegistry};

impl App {
    /// Record `name` as the active profile's theme, persist, and request that the
    /// next render apply it. Cheap and cx-free so it can run inside `process_actions`.
    pub(crate) fn set_theme(&mut self, name: String) {
        self.state.profiles.active_mut().theme = name;
        self.theme_dirty = true;
        self.save();
    }

    /// Apply the active profile's theme to the global gpui-component `Theme`.
    /// Falls back to [`DEFAULT_THEME`] when the stored name isn't registered (e.g.
    /// legacy enum values, or a custom theme file that was removed), rewriting the
    /// profile so the UI reflects what's actually shown.
    pub(crate) fn apply_active_theme(&mut self, window: &mut Window, cx: &mut GpuiApp) {
        let mut wanted = self.state.profiles.active().theme.clone();
        let registry = ThemeRegistry::global(cx);

        // gpui-component registers two generic built-ins ("Default Light"/"Default Dark")
        // that duplicate our Atlas Light/Graphite. A stale profile (or a reset) pointing
        // at those resolves to the opaque built-in — no glass. Redirect just those two
        // names to the Atlas default so the app boots glass; every Atlas/custom theme is
        // still honored and selectable.
        if wanted == "Default Dark" || wanted == "Default Light" {
            wanted = DEFAULT_THEME.to_string();
        }

        // Try to look up the (possibly redirected) name, falling back to DEFAULT_THEME
        // when the stored name isn't registered (legacy enum values, removed theme, …).
        let (resolved_name, config) =
            if let Some(cfg) = registry.themes().get(&SharedString::from(wanted.as_str())) {
                (wanted, cfg.clone())
            } else if let Some(cfg) = registry.themes().get(&SharedString::from(DEFAULT_THEME)) {
                (DEFAULT_THEME.to_string(), cfg.clone())
            } else {
                // No themes registered at all — nothing to apply.
                return;
            };

        // Persist the normalized name so the migration is one-time and sticky.
        if resolved_name != self.state.profiles.active().theme {
            self.state.profiles.active_mut().theme = resolved_name;
            self.save();
        }

        Theme::global_mut(cx).apply_config(&config);
        window.refresh();
    }
}
