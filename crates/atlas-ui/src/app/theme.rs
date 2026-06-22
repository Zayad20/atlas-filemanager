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
    /// Used by [`App::apply_active_theme`] to swap in the glass config when the
    /// Mica setting is on, while keeping the base name as the stored/selected
    /// theme so the picker stays coherent.
    const GLASS_SUFFIX: &'static str = " Glass";

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

        // Mica glass swap. When the global `mica` setting is on, transparently
        // apply the Glass variant of the resolved theme — same palette, but with
        // translucent backgrounds so the OS desktop material (Mica Alt) shows
        // through. The *stored* theme name stays the base ("Atlas Midnight"), so
        // the picker keeps showing what the user picked; only the applied config
        // comes from the glass twin. Themes without a glass twin (e.g. a custom
        // opaque theme, or "Atlas Frost" which is already glass) fall through to
        // the base config unchanged.
        let config = if self.state.settings.mica && !resolved_name.ends_with(Self::GLASS_SUFFIX) {
            let glass_name = format!("{resolved_name}{}", Self::GLASS_SUFFIX);
            registry
                .themes()
                .get(&SharedString::from(glass_name.as_str()))
                .cloned()
                .unwrap_or(config)
        } else if !self.state.settings.mica && resolved_name.ends_with(Self::GLASS_SUFFIX) {
            // Edge case: the stored theme *is* a glass variant (can happen if a
            // future picker exposes them, or after a setting flip). With Mica off
            // we want the opaque base, so peel the suffix off and look that up.
            let base_name = &resolved_name[..resolved_name.len() - Self::GLASS_SUFFIX.len()];
            registry
                .themes()
                .get(&SharedString::from(base_name))
                .cloned()
                .unwrap_or(config)
        } else {
            config
        };

        // Mirror the Mica setting onto the window's OS backdrop. MicaAlt is the
        // Windows 11 "tabbed window" material (DWMSBT_TABBEDWINDOW); Opaque falls
        // back to a flat background. Cheap to call every apply (no-op when
        // unchanged at the platform level).
        window.set_background_appearance(if self.state.settings.mica {
            WindowBackgroundAppearance::MicaAltBackdrop
        } else {
            WindowBackgroundAppearance::Opaque
        });

        // Persist the normalized name so the migration is one-time and sticky.
        // NOTE: `resolved_name` is always the base theme — never the glass twin —
        // so the swap above stays idempotent across reboots.
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
