//! `App::new`: construct the app, restore session/profiles, kick off loads.
#![allow(clippy::too_many_lines)] // reason: cohesive startup/drain kept whole

use atlas_model::layout_tree::Tile;
use std::collections::{HashMap, HashSet};

use gpui::{Context, Window};
use smol::Timer;

use super::*;
use atlas_core::fs::list::list_dir;
use atlas_core::search::nucleo_pool::NucleoPool;
use atlas_core::util::config::{load_profiles, load_session};
use atlas_model::app_state::{AppState, BgResult};
use atlas_model::tab::LoadState;

impl App {
    pub fn new(
        rt: tokio::runtime::Handle,
        settings: atlas_model::settings::Settings,
        startup_started: std::time::Instant,
        initial_paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_new_started = std::time::Instant::now();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let rubber_escape_listener = cx.listener(|this, ev: &gpui::KeystrokeEvent, window, cx| {
            if ev.keystroke.key == "escape" && this.rubber.is_some() {
                let pane = this.rubber.as_ref().and_then(|r| match r.target {
                    RubberTarget::Pane(pane) | RubberTarget::MillerColumn(pane, _) => Some(pane),
                    RubberTarget::Dropzone => None,
                });
                this.cancel_rubber();
                if let Some(pane) = pane {
                    this.notify_pane(pane, cx);
                }
                cx.notify();
                window.refresh();
                cx.stop_propagation();
            }
        });
        let rubber_escape_sub = cx.intercept_keystrokes(rubber_escape_listener);
        let profiles = load_profiles();
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = app_new_started.elapsed().as_millis(),
            "startup: profiles"
        );

        let (tx, rx) = flume::unbounded();
        let (icon_tx, icon_rx) = flume::unbounded();

        // Restore previous session tree if available, else create fresh state
        let session_started = std::time::Instant::now();
        let restored_tree = load_session();
        let (mut state, _initial_tile_id) = if let Some(tree) = restored_tree {
            let (s, id) = AppState::from_tree(tree, settings);
            // Discard a corrupted/empty session and start fresh
            if s.all_pane_tile_ids().is_empty() {
                AppState::new(s.settings)
            } else {
                (s, id)
            }
        } else {
            AppState::new(settings)
        };
        if let Some(path) = initial_paths.first().cloned() {
            let path = if path.is_dir() {
                Some(path)
            } else if path.is_file() {
                path.parent().map(std::path::Path::to_path_buf)
            } else {
                tracing::warn!(path = %path.display(), "startup path ignored: not found");
                None
            };
            if let Some(path) = path {
                if let Some(tile) = state.all_pane_tile_ids().first().copied() {
                    if let Some(atlas_model::layout_tree::Tile::Pane(tab)) =
                        state.pane_tree.tiles.get_mut(tile)
                    {
                        tab.location = atlas_model::location::Location::Directory(path);
                        state.focused_tile = tile;
                        state.pane_tree.make_active(|id, _| id == tile);
                    }
                }
            }
        }
        state.profiles = profiles;
        migrate_global_recent_folders(&mut state);
        if !state.settings.getting_started_seen {
            state.overlay.show_getting_started = true;
        }
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = session_started.elapsed().as_millis(),
            "startup: session"
        );
        // Profiles saved before the cloud feature lack the Network section, which
        // would hide configured remotes — backfill it.
        #[cfg(windows)]
        state.profiles.ensure_linux_section();

        // Resolve the startup icon theme from the active profile's saved selection
        // (official per-profile store), falling back to VSCode auto-discovery.
        let icon_theme = {
            let icon_started = std::time::Instant::now();
            let map = state
                .profiles
                .active()
                .scoped::<crate::app::profiles::IconSelection>();
            let theme = if map.is_empty() {
                crate::icon_theme::find_vscode_theme()
            } else {
                let paths = map
                    .into_iter()
                    .map(|(ext, path)| (ext, std::path::PathBuf::from(path)))
                    .collect();
                crate::icon_theme::IconThemeIndex::from_mappings(paths)
            };
            tracing::info!(
                total_ms = startup_started.elapsed().as_millis(),
                phase_ms = icon_started.elapsed().as_millis(),
                "startup: icon theme"
            );
            theme
        };

        // Spawn the SQLite tag-db worker. On a brand-new database it imports the tags
        // that previously lived only in profile config (global + per-profile).
        let migration = collect_tag_migration(&state.profiles);
        let tag_db_tx = atlas_core::fs::tag_db::spawn(
            &rt,
            atlas_core::util::config::data_dir().join("tags.db"),
            migration,
            tx.clone(),
        );
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            "startup: tag db spawned"
        );

        // Trigger directory loads for all panes (focused first, rest on demand)
        let list_started = std::time::Instant::now();
        let pane_ids = state.all_pane_tile_ids();
        let pane_count = pane_ids.len();
        let show_hidden = state.settings.show_hidden;
        for tile_id in pane_ids {
            let (location, sort_col, sort_dir, folders_first) =
                match state.pane_tree.tiles.get(tile_id) {
                    Some(Tile::Pane(p)) => (
                        p.location.clone(),
                        p.sort_col.clone(),
                        p.sort_dir.clone(),
                        p.folders_first,
                    ),
                    _ => continue,
                };
            // A restored Access-mode remote tab can't reconnect before rclone is up.
            #[cfg(feature = "provider-host")]
            if matches!(location, atlas_model::location::Location::Remote { .. }) {
                continue;
            }
            // A restored virtual tag tab re-queries its paths instead of listing a dir.
            if let atlas_model::location::Location::Tag(tag) = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = tag_db_tx.send(atlas_core::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
                continue;
            }
            // A restored archive tab lists its inner directory on startup.
            if let atlas_model::location::Location::Archive { archive, inner } = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let tx_clone = tx.clone();
                let archive2 = archive.clone();
                let inner2 = inner.clone();
                rt.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let provider = atlas_core::fs::archive::open(&archive)
                            .ok_or_else(|| rust_i18n::t!("app_archive_unsupported").to_string())?;
                        provider.list(&inner).map_err(|e| e.to_string())
                    })
                    .await;
                    let send_result = match result {
                        Ok(Ok(entries)) => BgResult::ArchiveListed {
                            tab: tile_id,
                            op_id,
                            archive: archive2,
                            inner: inner2,
                            entries,
                        },
                        Ok(Err(e)) => BgResult::ArchiveListError {
                            tab: tile_id,
                            op_id,
                            error: e,
                        },
                        Err(e) => BgResult::ArchiveListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        },
                    };
                    let _ = tx_clone.send(send_result);
                });
                continue;
            }
            let path = match location.fs_dir() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };
            let op_id = state.next_op_id();
            let ts = state.tab_states.entry(tile_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = tx.clone();
            rt.spawn(async move {
                match list_dir(&path, show_hidden, sort_col, sort_dir, folders_first).await {
                    Ok(mut entries) => {
                        crate::ui::views::details::format_entries(&mut entries);
                        let _ = tx_clone.send(BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = list_started.elapsed().as_millis(),
            panes = pane_count,
            "startup: pane loads queued"
        );

        let update_tx = tx.clone();
        rt.spawn(async move {
            match atlas_core::update::check_for_update(env!("CARGO_PKG_VERSION")).await {
                Ok(Some(update)) => {
                    let _ = update_tx.send(BgResult::AppUpdateAvailable {
                        version: update.version,
                        url: update.windows.url,
                    });
                }
                Ok(None) => {}
                Err(err) => tracing::debug!(?err, "update check failed"),
            }
        });

        // ── Windows OLE drag-and-drop setup ────────────────────────────────
        // Extract the HWND from gpui's HasWindowHandle impl. Stored as isize
        // so it is Send/clone-able; reconstructed to HWND only in dnd::windows.
        #[cfg(windows)]
        let (hwnd_opt, dnd_opt) = {
            use raw_window_handle::HasWindowHandle;
            let hwnd_raw = window.window_handle().ok().and_then(|h| {
                if let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw() {
                    Some(w.hwnd.get())
                } else {
                    None
                }
            });
            let dnd = hwnd_raw.and_then(|h| {
                // HiDPI scale from gpui (no cx needed for scale_factor).
                let scale = window.scale_factor();
                crate::dnd::windows::setup_dnd(h, scale)
            });
            (hwnd_raw, dnd)
        };

        // Pre-warm the shell-menu STA worker so the first right-click is instant.
        #[cfg(windows)]
        let shell_menu = Some(crate::shell_menu::spawn());

        // Extensions: seed bundled ones, load enabled overrides, build the registry.
        let _ = atlas_core::ext::install::ensure_bundled(
            "file-tools",
            crate::app::ext::FILE_TOOLS_MANIFEST,
        );
        let _ = atlas_core::ext::install::ensure_asset(
            "file-tools",
            "icon.svg",
            crate::app::ext::FILE_TOOLS_ICON,
        );
        let _ = atlas_core::ext::install::ensure_bundled(
            "media-thumbs",
            crate::app::ext::MEDIA_THUMBS_MANIFEST,
        );
        let _ = atlas_core::ext::install::ensure_asset(
            "media-thumbs",
            "icon.svg",
            crate::app::ext::MEDIA_THUMBS_ICON,
        );
        let _ = atlas_core::ext::install::ensure_bundled(
            "document-previews",
            crate::app::ext::DOCUMENT_PREVIEWS_MANIFEST,
        );
        let _ = atlas_core::ext::install::ensure_asset(
            "document-previews",
            "icon.svg",
            crate::app::ext::DOCUMENT_PREVIEWS_ICON,
        );
        let runtime_tools_src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/runtime-tools");
        let _ = atlas_core::ext::install::ensure_bundled_with_assets(
            "runtime-tools",
            crate::app::ext::RUNTIME_TOOLS_MANIFEST,
            &runtime_tools_src,
        );
        state.ext.enabled = atlas_core::util::config::load_ext_enabled();
        let ext_started = std::time::Instant::now();
        let ext_registry = atlas_core::ext::Registry::load(&state.ext.enabled);
        #[cfg(feature = "provider-host")]
        if ext_registry.has_sidebar_section("cloud") {
            state.profiles.ensure_cloud_section();
        }
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = ext_started.elapsed().as_millis(),
            "startup: extensions"
        );

        // Spawn the cloud worker (rclone sidecar is started lazily on first use)
        // and restore previously-configured remotes for the sidebar.
        #[cfg(feature = "provider-host")]
        let cloud_tx = atlas_core::cloud::spawn(&rt, tx.clone());
        #[cfg(feature = "provider-host")]
        {
            state.cloud.remotes = atlas_core::util::config::load_cloud_remotes();
            // Auto-mount remotes the user flagged. Not added to `navigate_on_mount`,
            // so they mount silently without hijacking the startup tab.
            if ext_registry.is_enabled("cloud") {
                for r in &state.cloud.remotes {
                    if r.auto_mount && r.access == atlas_core::cloud::AccessKind::Mount {
                        let _ =
                            cloud_tx.send(atlas_core::cloud::CloudCmd::Mount { id: r.id.clone() });
                    }
                }
            }
        }

        // Drain background results on a ~60 Hz foreground loop, repainting only
        // when the drain actually changed state. Breaks when the entity is gone.
        cx.spawn(async move |this, cx| loop {
            Timer::after(DRAIN_INTERVAL).await;
            let alive = this
                .update(cx, |app, cx| {
                    let mut dirty = app.drain_bg_results(cx);
                    // Drain Windows OLE drop payloads enqueued by our IDropTarget.
                    #[cfg(windows)]
                    {
                        dirty |= app.drain_dnd_queue();
                        dirty |= app.drain_shell_menu_events(cx);
                    }
                    // Finalize a palette's exit animation once its fade-out has elapsed.
                    dirty |= app.tick_palette_close();
                    // Finalize pane close-out animations: queue their dock removal.
                    dirty |= app.tick_pane_close();
                    dirty |= app.tick_tab_close();
                    dirty |= app.tick_toasts();
                    if dirty {
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();

        // Restore per-profile Details column widths + order (falls back to defaults
        // when the active profile has never customized them).
        let details_prefs = state
            .profiles
            .active()
            .scoped::<crate::app::profiles::DetailsColumns>();

        let nucleo = NucleoPool::new();
        let goto_root = state
            .active_tab_data()
            .and_then(|td| td.location.fs_dir().map(std::path::Path::to_path_buf))
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        atlas_core::fs::index::build_index(goto_root.clone(), nucleo.injector());
        let watchers = state
            .all_pane_tile_ids()
            .into_iter()
            .filter_map(|id| {
                let Tile::Pane(pane) = state.pane_tree.tiles.get(id)? else {
                    return None;
                };
                let path = pane.location.fs_dir()?;
                atlas_core::fs::watcher::watch_path(path, tx.clone())
                    .ok()
                    .map(|watcher| (id, watcher))
            })
            .collect();

        Self {
            state,
            startup_started,
            startup_first_render_logged: false,
            tx,
            rx,
            rt,
            focus_handle,
            scroll_handles: HashMap::new(),
            miller_scroll_handles: HashMap::new(),
            watchers,
            nucleo,
            goto_index_root: Some((goto_root, false)),
            goto_prefill: None,
            fuzzy_scorer: atlas_core::search::fuzzy::FuzzyScorer::new(),
            palette_scroll: gpui::ScrollHandle::new(),
            ext_registry,
            tag_db_tx,
            #[cfg(feature = "provider-host")]
            cloud_tx,
            // Phase5: preview texture caches + overlay focus start empty.
            preview_textures: HashMap::new(),
            multi_textures: HashMap::new(),

            preview_view: HashMap::new(),
            preview_panel_anim_seq: 0,
            preview_panel_closing: None,
            last_viewport_size: None,
            window_resize_suppress_until: None,
            preview_drag: None,
            overlay_focus: None,
            batch_rename_field: BatchRenameField::Pattern,
            // Phase4: grid thumbnail cache starts empty.
            thumb_textures: HashMap::new(),
            ext_thumb_inflight: HashSet::new(),
            // Icon theme resolved above from the active profile (selection or
            // VSCode auto-discovery).
            icon_theme,
            icon_cache: HashMap::new(),
            icon_tx,
            icon_rx,
            // Phase A: dock built lazily on first render from the restored pane_tree.
            file_panel_cache: HashMap::new(),
            space_lens: HashMap::new(),
            pane_tabs_cache: HashMap::new(),
            pane_tabs_fp: HashMap::new(),
            pane_tabs_bounds: HashMap::new(),
            sidebar_panel: None,
            dock_host: None,
            focus_history: std::collections::VecDeque::new(),
            palette_host: None,
            palette_closing: None,
            layout_revision: 0,
            // Phase C: rubber-band selection state.
            rubber: None,
            row_rects: HashMap::new(),
            miller_row_rects: HashMap::new(),
            details_hit: HashMap::new(),
            details_hover: None,
            miller_column_bounds: HashMap::new(),
            dropzone_selected: HashSet::new(),
            dropzone_rects: Vec::new(),
            dropzone_bounds: None,
            dropzone_drag_paths: HashSet::new(),
            pending_pane_zones: Vec::new(),
            goto_input: None,
            palette_input: None,
            ext_palette_input: None,
            search_inputs: HashMap::new(),
            batch_pattern_input: None,
            batch_replacement_input: None,
            rename_profile_input: None,
            getting_started_profile_input: None,
            create_tag_input: None,
            renaming_profile_idx: None,
            settings_input: None,
            settings_section: 0,
            rename_edit: None,
            rename_visual_suppress: None,
            content_results: Vec::new(),
            content_gen: 0,
            content_cancel: None,
            theme_dirty: true,
            theme_preview_before: None,
            sidebar_width_dirty: true,
            sidebar_render_width: None,
            sidebar_width_animation: None,
            #[cfg(windows)]
            hwnd: hwnd_opt,
            #[cfg(windows)]
            dnd: dnd_opt,
            #[cfg(windows)]
            shell_menu,
            was_inactive: false,
            inactive_since: None,
            eco_cleared: false,
            ctx_menu: None,
            ctx_menu_input: None,
            ctx_menu_sub: None,
            ctx_menu_seq: 0,
            suppress_next_shell_ctx_menu: false,
            ctx_menu_scroll: gpui::ScrollHandle::new(),
            type_filter_scroll: gpui::ScrollHandle::new(),
            settings_scroll: gpui::ScrollHandle::new(),
            getting_started_theme_scroll: gpui::ScrollHandle::new(),
            ext_manager_scroll: gpui::ScrollHandle::new(),
            pane_zoom_sliders: HashMap::new(),
            pane_zoom_slider_subs: HashMap::new(),
            preview_editor: None,
            tag_picker_menu: None,
            tag_picker_input: None,
            tag_picker_sub: None,
            _rubber_escape_sub: Some(rubber_escape_sub),
            tag_picker_scroll: gpui::ScrollHandle::new(),
            thumb_wanted: std::sync::Arc::new(std::sync::Mutex::new(HashSet::new())),
            native_thumb_inflight: 0,
            pane_animations: HashMap::new(),
            pane_signals: HashMap::new(),
            dirty_panes: HashSet::new(),
            sidebar_scroll_region: None,
            atlas_launcher_profiles_anchor: gpui::Point::default(),
            pane_layout_bounds: HashMap::new(),
            pane_layout_animations: HashMap::new(),
            dock_root_bounds: None,
            dock_split_animations: HashMap::new(),
            pane_closing: HashMap::new(),
            tab_closing: HashMap::new(),
            pane_items_appear: HashMap::new(),
            ext_manager_input: None,
            #[cfg(feature = "provider-host")]
            cloud_add_inputs: None,
            #[cfg(feature = "provider-host")]
            rename_remote_input: None,
            pending_rename: None,
            pending_select: None,
            col_widths: details_prefs.widths,
            details_column_order: details_prefs.order,
            col_resize: None,
            col_dragging: None,
            col_drag_x: 0.0,
            details_reorder: None,
            type_filters: HashMap::new(),
            size_filters: HashMap::new(),
            type_filter_menu: None,
            type_filter_input: None,
            type_filter_sub: None,
            breadcrumb_menu: None,
            breadcrumb_menu_input: None,
            breadcrumb_menu_sub: None,
            size_filter_menu: None,
            drag_hover: crate::ui::drag_hover::DragHoverState::default(),
            pending_drag_hover_targets: Vec::new(),
            pane_view_bounds: HashMap::new(),
            dock_linear_bounds: HashMap::new(),
            dock_resize: None,
            sidebar_resize: None,
            inspector_resize: None,
            dirty_all_panes: false,
            command_shortcut_capture: None,
        }
    }
}

fn migrate_global_recent_folders(state: &mut AppState) {
    if state.settings.recent_folders.is_empty() {
        return;
    }
    use atlas_model::profiles::RecentFoldersFeature;
    let legacy = std::mem::take(&mut state.settings.recent_folders);
    for profile in &mut state.profiles.profiles {
        if profile.scoped::<RecentFoldersFeature>().is_empty() {
            profile.set_scoped::<RecentFoldersFeature>(&legacy);
        }
    }
}
