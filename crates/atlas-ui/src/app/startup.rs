//! `App::new`: construct the app, restore session/profiles, kick off loads.
#![allow(clippy::too_many_lines)] // reason: cohesive startup/drain kept whole

use crate::state::layout_tree::Tile;
use std::collections::{HashMap, HashSet};

use gpui::{Context, Window};
use smol::Timer;

use super::*;
use crate::fs::list::list_dir;
use crate::search::nucleo_pool::NucleoPool;
use crate::state::app_state::{AppState, BgResult};
use crate::state::tab::LoadState;
use crate::util::config::{load_profiles, load_session};

impl App {
    pub fn new(
        rt: tokio::runtime::Handle,
        settings: crate::state::settings::Settings,
        startup_started: std::time::Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_new_started = std::time::Instant::now();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
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
        state.profiles = profiles;
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = session_started.elapsed().as_millis(),
            "startup: session"
        );
        // Profiles saved before the cloud feature lack the Network section, which
        // would hide configured remotes — backfill it.
        #[cfg(feature = "cloud")]
        state.profiles.ensure_cloud_section();

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
        let tag_db_tx = crate::fs::tag_db::spawn(
            &rt,
            crate::util::config::data_dir().join("tags.db"),
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
            #[cfg(feature = "cloud")]
            if matches!(location, crate::state::location::Location::Remote { .. }) {
                continue;
            }
            // A restored virtual tag tab re-queries its paths instead of listing a dir.
            if let crate::state::location::Location::Tag(tag) = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = tag_db_tx.send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
                continue;
            }
            // A restored archive tab lists its inner directory on startup.
            if let crate::state::location::Location::Archive { archive, inner } = location {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let tx_clone = tx.clone();
                let archive2 = archive.clone();
                let inner2 = inner.clone();
                rt.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let provider = crate::fs::archive::open(&archive)
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
                    Ok(entries) => {
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
        let _ =
            crate::ext::install::ensure_bundled("file-tools", crate::app::ext::FILE_TOOLS_MANIFEST);
        let _ = crate::ext::install::ensure_bundled(
            "media-thumbs",
            crate::app::ext::MEDIA_THUMBS_MANIFEST,
        );
        let _ = crate::ext::install::ensure_bundled(
            "document-previews",
            crate::app::ext::DOCUMENT_PREVIEWS_MANIFEST,
        );
        let runtime_tools_src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/runtime-tools");
        let _ = crate::ext::install::ensure_bundled_with_assets(
            "runtime-tools",
            crate::app::ext::RUNTIME_TOOLS_MANIFEST,
            &runtime_tools_src,
        );
        state.ext.enabled = crate::util::config::load_ext_enabled();
        let ext_started = std::time::Instant::now();
        let ext_registry = crate::ext::Registry::load(&state.ext.enabled);
        tracing::info!(
            total_ms = startup_started.elapsed().as_millis(),
            phase_ms = ext_started.elapsed().as_millis(),
            "startup: extensions"
        );

        // Spawn the cloud worker (rclone sidecar is started lazily on first use)
        // and restore previously-configured remotes for the sidebar.
        #[cfg(feature = "cloud")]
        let cloud_tx = crate::cloud::spawn(&rt, tx.clone());
        #[cfg(feature = "cloud")]
        {
            state.cloud.remotes = crate::util::config::load_cloud_remotes();
            // Auto-mount remotes the user flagged. Not added to `navigate_on_mount`,
            // so they mount silently without hijacking the startup tab.
            for r in &state.cloud.remotes {
                if r.auto_mount && r.access == crate::cloud::AccessKind::Mount {
                    let _ = cloud_tx.send(crate::cloud::CloudCmd::Mount { id: r.id.clone() });
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

        Self {
            state,
            startup_started,
            startup_first_render_logged: false,
            tx,
            rx,
            rt,
            focus_handle,
            scroll_handles: HashMap::new(),
            watchers: HashMap::new(),
            nucleo: NucleoPool::new(),
            nucleo_results: Vec::new(),
            goto_index_root: None,
            goto_prefill: None,
            fuzzy_scorer: crate::search::fuzzy::FuzzyScorer::new(),
            palette_scroll: gpui::ScrollHandle::new(),
            ext_registry,
            pending_preview: None,
            tag_db_tx,
            #[cfg(feature = "cloud")]
            cloud_tx,
            // FASE5: preview texture caches + overlay focus start empty.
            preview_textures: HashMap::new(),
            multi_textures: HashMap::new(),

            preview_view: HashMap::new(),
            preview_drag: None,
            overlay_focus: None,
            batch_rename_field: BatchRenameField::Pattern,
            // FASE4: grid thumbnail cache starts empty.
            thumb_textures: HashMap::new(),
            ext_thumb_inflight: HashSet::new(),
            // Icon theme resolved above from the active profile (selection or
            // VSCode auto-discovery).
            icon_theme,
            icon_cache: HashMap::new(),
            icon_tx,
            icon_rx,
            // FASE A: dock built lazily on first render from the restored pane_tree.
            dock: None,
            file_panel_cache: HashMap::new(),
            sidebar_panel: None,
            focus_history: std::collections::VecDeque::new(),
            palette_host: None,
            palette_closing: None,
            layout_revision: 0,
            dock_revision: 0,
            // FASE C: rubber-band selection state.
            rubber: None,
            row_rects: HashMap::new(),
            pending_pane_zones: Vec::new(),
            goto_input: None,
            palette_input: None,
            ext_palette_input: None,
            rename_input: None,
            search_inputs: HashMap::new(),
            batch_pattern_input: None,
            batch_replacement_input: None,
            rename_profile_input: None,
            create_tag_input: None,
            renaming_profile_idx: None,
            rename_edit: None,
            content_results: Vec::new(),
            content_gen: 0,
            content_cancel: None,
            theme_dirty: true,
            sidebar_width_dirty: true,
            #[cfg(windows)]
            hwnd: hwnd_opt,
            #[cfg(windows)]
            dnd: dnd_opt,
            #[cfg(windows)]
            shell_menu,
            ctx_menu: None,
            ctx_menu_input: None,
            ctx_menu_sub: None,
            ctx_menu_seq: 0,
            ctx_menu_scroll: gpui::ScrollHandle::new(),
            type_filter_scroll: gpui::ScrollHandle::new(),
            settings_scroll: gpui::ScrollHandle::new(),
            ext_manager_scroll: gpui::ScrollHandle::new(),
            preview_editor: None,
            tag_picker_menu: None,
            tag_picker_input: None,
            tag_picker_sub: None,
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
            pane_layout_frame_pending: false,
            dock_split_animations: HashMap::new(),
            pending_dock_ops: Vec::new(),
            pane_closing: HashMap::new(),
            tab_closing: HashMap::new(),
            reconcile_pending: false,
            pane_items_appear: HashMap::new(),
            ext_manager_input: None,
            #[cfg(feature = "cloud")]
            cloud_add_inputs: None,
            #[cfg(feature = "cloud")]
            rename_remote_input: None,
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
