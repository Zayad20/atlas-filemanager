//! `atlas-ui`: the gpui application layer — the `App` entity, views, overlays,
//! input/actions and platform glue. Depends on `atlas-core` (backends) and
//! `atlas-model` (state); the binary (`src/main.rs`) is a thin launcher over
//! [`run`].
//!
//! FASE J: moved here from the binary's `src/{app,ui,input,state,fs,cloud,ext,
//! search,util}`. The `state`/`fs`/`cloud`/`ext`/`search`/`util` modules are the
//! same re-export shims as before, so existing `crate::state::…`, `crate::fs::…`
//! paths inside the moved files keep resolving unchanged.

// reason: gpui's actions! macro derives PartialEq (not Eq) on every action struct.
#![allow(clippy::derive_partial_eq_without_eq)]

rust_i18n::i18n!("locales", fallback = "en");

// Re-export shims (kept crate-internal so moved files' `crate::…` paths resolve).
#[cfg(feature = "cloud")]
mod cloud;
mod ext;
mod fs;
mod input;
mod search;
mod state;
mod util;

pub mod app;
pub mod assets;
/// Platform drag-and-drop: neutral types + Windows OLE implementation.
pub(crate) mod dnd;
pub(crate) mod icon_theme;
/// Windows shell context-menu integration (IContextMenu enumeration + invoke).
#[cfg(target_os = "windows")]
pub(crate) mod shell_menu;
pub mod ui;

use gpui::{
    px, size, App as GpuiApp, AppContext as _, KeyBinding, WindowBackgroundAppearance,
    WindowBounds, WindowOptions,
};
use gpui_component::{Root, ThemeRegistry, TitleBar};

gpui::actions!(
    atlas,
    [
        Quit,
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        NavUp,
        NavDown,
        NavHome,
        NavEnd,
        NavPageUp,
        NavPageDown,
        ExtendUp,
        ExtendDown,
        OpenSelected,
        SelectAllItems,
        RenameSelected,
        // FASE D: overlays / panels / view toggles.
        OpenPalette,
        OpenGotoBar,
        OpenSearchInPlace,
        TogglePreviewExpanded,
        ToggleInspector,
        ToggleSidebar,
        ToggleHidden,
        OpenSettings,
        CloseOverlays,
        // FASE D: pane structure.
        CloseFocusedPane,
        FocusNextPane,
        SplitHorizontal,
        SplitVertical,
        // FASE D: history / location / refresh.
        Undo,
        Redo,
        RefreshTab,
        EditLocation,
        NavBack,
        NavForward,
        NavParent,
        DeleteToTrash,
        DeletePermanent,
        SidebarNewGlobalTag,
        SidebarNewProfileTag,
        // Bottom-bar view selector (per-pane dropdown).
        ViewModeDetails,
        ViewModeGrid,
        ViewModeList,
        ViewModeMiller,
        ZoomIn,
        ZoomOut,
        ZoomReset,
    ]
);

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct SidebarToggleSection {
    pub index: usize,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct SidebarHideSection {
    pub index: usize,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct SidebarMoveSectionUp {
    pub index: usize,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct SidebarMoveSectionDown {
    pub index: usize,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct SidebarResetSections;

/// Launch the gpui application. The caller owns the tokio runtime and passes its
/// handle in; gpui drives the UI on the calling (main) thread.
pub fn run(handle: tokio::runtime::Handle) {
    let settings = crate::util::config::load_settings();
    let lang = settings.language.clone().unwrap_or_else(|| "en".to_string());
    rust_i18n::set_locale(&lang);

    let application = gpui_platform::application().with_assets(crate::assets::AtlasAssets);
    application.run(move |cx: &mut GpuiApp| {

        gpui_component::init(cx);

        // Register the bundled Atlas themes (Mac-minimal set) and watch the user
        // themes dir for custom `ThemeSet` JSON files. The active profile's theme is
        // applied on first render (see `App::apply_active_theme`).
        let json = include_str!("../assets/themes/atlas-themes.json");
        match ThemeRegistry::global_mut(cx).load_themes_from_str(json) {
            Ok(()) => {
                eprintln!(
                    "[atlas] {} themes registered",
                    ThemeRegistry::global(cx).themes().len()
                );
            }
            Err(err) => {
                eprintln!("[atlas] FAILED to load bundled themes: {err:#}");
            }
        }
        // CRITICAL: `watch_dir` kicks off an async `reload()` that `themes.clear()`s and
        // rebuilds the registry from default themes + files in `themes_dir` only — which
        // wipes the in-memory themes we just registered with `load_themes_from_str`
        // (they're not files). So persist the bundle to `themes_dir` first; then the
        // reload re-reads it and our Atlas themes survive (and show in the picker).
        let themes_dir = atlas_core::util::config::themes_dir();
        if let Err(err) = std::fs::create_dir_all(&themes_dir)
            .and_then(|()| std::fs::write(themes_dir.join("atlas-themes.json"), json))
        {
            tracing::error!(?err, "failed to persist bundled themes to themes_dir");
        }
        if let Err(err) = ThemeRegistry::watch_dir(themes_dir, cx, |cx| {
            // New/edited theme files appear in the selector immediately; applying an
            // edited active theme just needs a re-select.
            cx.refresh_windows();
        }) {
            tracing::error!(?err, "failed to watch themes dir");
        }

        cx.bind_keys([
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-q", Quit, None),
            #[cfg(not(target_os = "macos"))]
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("up", NavUp, Some("Atlas")),
            KeyBinding::new("down", NavDown, Some("Atlas")),
            KeyBinding::new("shift-up", ExtendUp, Some("Atlas")),
            KeyBinding::new("shift-down", ExtendDown, Some("Atlas")),
            KeyBinding::new("home", NavHome, Some("Atlas")),
            KeyBinding::new("end", NavEnd, Some("Atlas")),
            KeyBinding::new("pageup", NavPageUp, Some("Atlas")),
            KeyBinding::new("pagedown", NavPageDown, Some("Atlas")),
            KeyBinding::new("enter", OpenSelected, Some("Atlas")),
            KeyBinding::new("ctrl-a", SelectAllItems, Some("Atlas")),
            KeyBinding::new("f2", RenameSelected, Some("Atlas")),
            KeyBinding::new("ctrl-t", NewTab, Some("Atlas")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Atlas")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Atlas")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Atlas")),
            // FASE D: overlays / panels / toggles.
            KeyBinding::new("ctrl-shift-p", OpenPalette, Some("Atlas")),
            KeyBinding::new("ctrl-k", OpenGotoBar, Some("Atlas")),
            KeyBinding::new("ctrl-f", OpenSearchInPlace, Some("Atlas")),
            KeyBinding::new("space", TogglePreviewExpanded, Some("Atlas")),
            KeyBinding::new("ctrl-i", ToggleInspector, Some("Atlas")),
            KeyBinding::new("ctrl-b", ToggleSidebar, Some("Atlas")),
            KeyBinding::new("ctrl-.", ToggleHidden, Some("Atlas")),
            KeyBinding::new("ctrl-,", OpenSettings, Some("Atlas")),
            KeyBinding::new("escape", CloseOverlays, Some("Atlas")),
            // FASE D: pane structure.
            KeyBinding::new("ctrl-shift-w", CloseFocusedPane, Some("Atlas")),
            KeyBinding::new("alt-tab", FocusNextPane, Some("Atlas")),
            KeyBinding::new("ctrl-shift-s", SplitHorizontal, Some("Atlas")),
            KeyBinding::new("ctrl-alt-s", SplitVertical, Some("Atlas")),
            // FASE D: history / location / refresh.
            KeyBinding::new("ctrl-z", Undo, Some("Atlas")),
            KeyBinding::new("ctrl-shift-z", Redo, Some("Atlas")),
            KeyBinding::new("f5", RefreshTab, Some("Atlas")),
            KeyBinding::new("ctrl-r", RefreshTab, Some("Atlas")),
            KeyBinding::new("ctrl-l", EditLocation, Some("Atlas")),
            KeyBinding::new("alt-left", NavBack, Some("Atlas")),
            KeyBinding::new("alt-right", NavForward, Some("Atlas")),
            KeyBinding::new("alt-up", NavParent, Some("Atlas")),
            // FASE D: clipboard / delete.
            KeyBinding::new("delete", DeleteToTrash, Some("Atlas")),
            KeyBinding::new("shift-delete", DeletePermanent, Some("Atlas")),
            KeyBinding::new("ctrl-c", gpui_component::input::Copy, Some("Atlas")),
            KeyBinding::new("ctrl-x", gpui_component::input::Cut, Some("Atlas")),
            KeyBinding::new("ctrl-v", gpui_component::input::Paste, Some("Atlas")),
            KeyBinding::new("ctrl-=", ZoomIn, Some("Atlas")),
            KeyBinding::new("ctrl--", ZoomOut, Some("Atlas")),
            KeyBinding::new("ctrl-0", ZoomReset, Some("Atlas")),
        ]);
        cx.on_action(|_: &Quit, cx: &mut GpuiApp| {
            // Keep copied files available to other apps after Atlas exits.
            #[cfg(windows)]
            crate::dnd::windows::ole_clipboard::flush();
            cx.quit();
        });

        let acrylic_enabled = std::env::var("ATLAS_ACRYLIC").is_ok_and(|v| v != "0");
        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            // Real OS glass: gpui native `Blurred` = ACCENT_ENABLE_ACRYLICBLURBEHIND (the
            // same path Zed uses on Windows). Unlike Mica/systembackdrop (which needs a
            // DWM redirection surface and no-ops against gpui's NoRedirectionBitmap +
            // DirectComposition window), this accent-policy acrylic DOES composite. It
            // only shows when the whole content tree is transparent: the active theme's
            // `background` must have alpha 0 (Atlas Frost) and the root paints no opaque
            // layer — the pastel `glass_backdrop` is kept translucent so the blur shows.
            window_background: if acrylic_enabled {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            },
            ..Default::default()
        };

        let _initial_zoom = settings.global_zoom;
        cx.spawn(async move |cx| {
            if let Err(err) = cx.open_window(window_options, move |window, cx| {
                window.set_window_title("Atlas");
                let view = cx.new(|cx| app::App::new(handle, window, cx));
                // The top-level window view must be `Root` (mount point for
                // gpui-component overlays, modals and notifications).
                cx.new(|cx| Root::new(view, window, cx))
            }) {
                tracing::error!(?err, "failed to open window");
            }
        })
        .detach();
    });
}
