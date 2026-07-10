//! Theme application glue. The active profile stores a theme *name*; here we map
//! it onto gpui-component's global `Theme` via the `ThemeRegistry`. Applying needs
//! `&mut gpui::App` + a `Window`, which `process_actions` doesn't have, so callers
//! (palette, settings, profile switch) just set the name and flip `theme_dirty`;
//! the next render calls [`App::apply_active_theme`].

use crate::app::App;
use atlas_model::profiles::DEFAULT_THEME;
use gpui::{App as GpuiApp, SharedString, Window, WindowBackgroundAppearance};
use gpui_component::scroll::ScrollbarShow;
use gpui_component::{Theme, ThemeRegistry};

impl App {
    /// Suffix that marks a theme as the translucent "Glass" variant of a base
    /// theme. Paired by name: `"Atlas Midnight"` ↔ `"Atlas Midnight Glass"`.
    const GLASS_SUFFIX: &'static str = " Glass";

    /// Record `name` as the active profile's theme, persist, and request that the
    /// next render apply it. Cheap and cx-free so it can run inside `process_actions`.
    pub(crate) fn set_theme(&mut self, name: String) {
        self.theme_preview_before = None;
        self.state.profiles.active_mut().theme = name;
        self.theme_dirty = true;
        self.save();
    }

    pub(crate) fn preview_theme(&mut self, name: String) {
        if self.theme_preview_before.is_none() {
            self.theme_preview_before = Some(self.state.profiles.active().theme.clone());
        }
        if self.state.profiles.active().theme != name {
            self.state.profiles.active_mut().theme = name;
            self.theme_dirty = true;
        }
    }

    pub(crate) fn restore_theme_preview(&mut self) {
        let Some(name) = self.theme_preview_before.take() else {
            return;
        };
        if self.state.profiles.active().theme != name {
            self.state.profiles.active_mut().theme = name;
            self.theme_dirty = true;
        }
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

        let wants_backdrop =
            resolved_name.ends_with(Self::GLASS_SUFFIX) || resolved_name == "Atlas Frost";

        // Enable the OS backdrop for explicit glass themes. This is Atlas'
        // equivalent of Zed's `background.appearance=blurred`.
        window.set_background_appearance(if wants_backdrop {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        });

        // Persist the normalized name so the migration is one-time and sticky.
        if resolved_name != self.state.profiles.active().theme {
            self.state.profiles.active_mut().theme = resolved_name;
            self.save();
        }

        Theme::global_mut(cx).apply_config(&config);
        // Re-apply zoom so Root's per-frame set_rem_size(theme.font_size) picks up
        // the user's scale instead of the theme default (px(16.)).
        Theme::global_mut(cx).font_size = gpui::px(16.0 * self.state.settings.global_zoom);
        // Always show scrollbar on hover (not only while scrolling) and boost thumb
        // opacity so it's actually visible against glass backgrounds.
        {
            let t = Theme::global_mut(cx);
            t.scrollbar_show = ScrollbarShow::Hover;
            t.scrollbar = gpui::transparent_black();
            let base = t.scrollbar_thumb;
            t.scrollbar_thumb = gpui::hsla(base.h, base.s, base.l, 0.45);
            let hover_base = t.scrollbar_thumb_hover;
            t.scrollbar_thumb_hover = gpui::hsla(hover_base.h, hover_base.s, hover_base.l, 0.75);
        }
        window.refresh();
    }
}
