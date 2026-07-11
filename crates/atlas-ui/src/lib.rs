//! `atlas-ui`: the gpui application layer — the `App` entity, views, overlays,
//! input/actions and platform glue. Depends on `atlas-core` (backends) and
//! `atlas-model` (state); the binary (`src/main.rs`) is a thin launcher over
//! [`run`].
//!
// reason: gpui's actions! macro derives PartialEq (not Eq) on every action struct.
#![allow(clippy::derive_partial_eq_without_eq)]

rust_i18n::i18n!("locales", fallback = "en");

mod input;

pub mod app;
pub mod assets;
/// Platform drag-and-drop: neutral types + Windows OLE implementation.
pub(crate) mod dnd;
pub(crate) mod icon_theme;
pub(crate) mod perf;
/// Windows shell context-menu integration (IContextMenu enumeration + invoke).
#[cfg(target_os = "windows")]
pub(crate) mod shell_menu;
/// Windows Shell thumbnail cache access via IShellItemImageFactory.
#[cfg(target_os = "windows")]
pub(crate) mod shell_thumb;
pub mod ui;

use gpui::{
    px, size, App as GpuiApp, AppContext as _, KeyBinding, WindowBackgroundAppearance,
    WindowBounds, WindowOptions,
};
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::time::Instant;

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
        // Phase D: overlays / panels / view toggles.
        OpenPalette,
        OpenGotoBar,
        OpenSearchInPlace,
        TogglePreviewExpanded,
        ToggleInspector,
        ToggleSidebar,
        ToggleHidden,
        OpenSettings,
        OpenExtManager,
        CloseOverlays,
        // Phase D: pane structure.
        CloseFocusedPane,
        FocusNextPane,
        SplitHorizontal,
        SplitVertical,
        // Phase D: history / location / refresh.
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

#[cfg(feature = "provider-host")]
#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct RemoveCloudRemote {
    pub id: String,
}

#[cfg(feature = "provider-host")]
#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct StartRenameCloudRemote {
    pub id: String,
}

#[cfg(all(feature = "provider-host", windows))]
#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct InstallWinFspForRemote {
    pub id: String,
}

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

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct OpenSpaceLensForPath {
    pub path: std::path::PathBuf,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct OpenPropertiesForPath {
    pub path: std::path::PathBuf,
}

#[derive(gpui::Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = atlas, no_json)]
pub struct OpenThisPcProperties;

/// Launch the gpui application. The caller owns the tokio runtime and passes its
/// handle in; gpui drives the UI on the calling (main) thread.
pub fn run(
    handle: tokio::runtime::Handle,
    app_started: Instant,
    on_quit: Option<Box<dyn FnOnce() + 'static>>,
    initial_paths: Vec<std::path::PathBuf>,
) {
    let run_started = Instant::now();
    let settings = atlas_core::util::config::load_settings();
    let lang = settings
        .language
        .clone()
        .unwrap_or_else(|| "en".to_string());
    rust_i18n::set_locale(&lang);
    tracing::info!(
        total_ms = app_started.elapsed().as_millis(),
        phase_ms = run_started.elapsed().as_millis(),
        "startup: settings+locale"
    );

    let application = gpui_platform::application().with_assets(crate::assets::AtlasAssets);
    application.run(move |cx: &mut GpuiApp| {
        let gpui_started = Instant::now();
        gpui_component::init(cx);
        if let Err(err) = cx.text_system().add_fonts(crate::assets::bundled_fonts()) {
            tracing::warn!(?err, "failed to load bundled Geist fonts");
        }
        tracing::info!(
            total_ms = app_started.elapsed().as_millis(),
            phase_ms = gpui_started.elapsed().as_millis(),
            "startup: gpui init"
        );

        // Register the bundled Atlas themes (Mac-minimal set) and watch the user
        // themes dir for custom `ThemeSet` JSON files. The active profile's theme is
        // applied on first render (see `App::apply_active_theme`).
        let themes_started = Instant::now();
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
        tracing::info!(
            total_ms = app_started.elapsed().as_millis(),
            phase_ms = themes_started.elapsed().as_millis(),
            "startup: themes"
        );

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
            KeyBinding::new("f2", RenameSelected, Some("Atlas")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Atlas")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Atlas")),
            // Phase D: overlays / panels / toggles.
            KeyBinding::new("ctrl-f", OpenSearchInPlace, Some("Atlas")),
            KeyBinding::new("space", ToggleInspector, Some("Atlas")),
            KeyBinding::new("escape", CloseOverlays, Some("Atlas")),
            // Phase D: pane structure.
            KeyBinding::new("alt-tab", FocusNextPane, Some("Atlas")),
            // Phase D: history / location / refresh.
            KeyBinding::new("f5", RefreshTab, Some("Atlas")),
            KeyBinding::new("ctrl-r", RefreshTab, Some("Atlas")),
            KeyBinding::new("ctrl-l", EditLocation, Some("Atlas")),
            KeyBinding::new("alt-up", NavParent, Some("Atlas")),
            // Phase D: clipboard / delete.
            KeyBinding::new("ctrl-c", gpui_component::input::Copy, Some("Atlas")),
            KeyBinding::new("ctrl-x", gpui_component::input::Cut, Some("Atlas")),
            KeyBinding::new("ctrl-v", gpui_component::input::Paste, Some("Atlas")),
        ]);
        cx.on_action(|_: &Quit, cx: &mut GpuiApp| {
            // Keep copied files available to other apps after Atlas exits.
            #[cfg(windows)]
            crate::dnd::windows::ole_clipboard::flush();
            cx.quit();
        });

        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(900.), px(560.))),
            ..Default::default()
        };

        let app_settings = settings.clone();
        cx.spawn(async move |cx| {
            let window_started = Instant::now();
            if let Err(err) = cx.open_window(window_options, move |window, cx| {
                tracing::info!(
                    total_ms = app_started.elapsed().as_millis(),
                    phase_ms = window_started.elapsed().as_millis(),
                    "startup: window callback"
                );
                window.set_window_title("Atlas");
                let initial_paths = initial_paths.clone();
                let view = cx.new(|cx| {
                    app::App::new(handle, app_settings, app_started, initial_paths, window, cx)
                });
                // Autosave: persist settings/session/profiles when the app is
                // about to quit, gated by the user's autosave-session setting.
                let weak_app = view.downgrade();
                cx.on_app_quit(move |cx| {
                    weak_app.update(cx, |app, _cx| app.save_on_quit()).ok();
                    std::future::ready(())
                })
                .detach();
                // The top-level window view must be `Root` (mount point for
                // gpui-component overlays, modals and notifications).
                cx.new(|cx| Root::new(view, window, cx))
            }) {
                tracing::error!(?err, "failed to open window");
            }
            tracing::info!(
                total_ms = app_started.elapsed().as_millis(),
                phase_ms = window_started.elapsed().as_millis(),
                "startup: open window"
            );
        })
        .detach();

        if let Some(on_quit) = on_quit {
            let mut on_quit_opt = Some(on_quit);
            cx.on_app_quit(move |_| {
                let cb = on_quit_opt.take();
                async move {
                    if let Some(cb) = cb {
                        cb();
                    }
                }
            })
            .detach();
        }
    });
}
