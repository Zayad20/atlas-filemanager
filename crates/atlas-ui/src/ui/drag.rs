//! Shared file drag-and-drop wiring used by every view's rows/cells.
//!
//! The `on_drag` source closure (drag the full selection, route through the
//! Windows OLE path) was copy-pasted into all four view renderers. It lives here
//! once; each view calls [`file_drag_handler`].

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{Entity, Window};

use crate::app::App;
#[cfg(windows)]
use crate::ui::root::DragNoop;
#[cfg(not(windows))]
use crate::ui::root::DraggedFiles;
use atlas_model::pane::TabId;

/// The view entity gpui renders as the drag ghost. On Windows the OLE path owns
/// the cursor, so we hand gpui an invisible [`DragNoop`]; elsewhere gpui paints
/// the [`DraggedFiles`] ghost itself.
#[cfg(windows)]
pub(crate) type DragView = DragNoop;
#[cfg(not(windows))]
pub(crate) type DragView = DraggedFiles;

/// The drag *payload* gpui tracks for its native DnD. It must be [`DragView`] so
/// that on Windows it's [`DragNoop`] — a type no `on_drop::<DraggedFiles>` handler
/// matches. Otherwise gpui's synthetic mouse-up (injected after `DoDragDrop`)
/// would fire a second, native drop and move the file even when OLE was cancelled.
#[cfg(windows)]
pub(crate) fn file_drag_value(_paths: Vec<PathBuf>) -> DragView {
    DragNoop
}
#[cfg(not(windows))]
pub(crate) fn file_drag_value(paths: Vec<PathBuf>) -> DragView {
    DraggedFiles { paths }
}

pub(crate) fn file_drag_handler(
    view: Entity<App>,
    anchor: PathBuf,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    build_file_drag_handler(view, None, anchor)
}

pub(crate) fn file_drag_handler_for_item(
    view: Entity<App>,
    pane: TabId,
    idx: usize,
    anchor: PathBuf,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    build_file_drag_handler(view, Some((pane, idx)), anchor)
}

pub(crate) fn file_drag_handler_for_details_content(
    view: Entity<App>,
    pane: TabId,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    move |_d, cursor_offset, _window, cx| {
        let item = {
            let app = view.read(&*cx);
            app.details_hit
                .get(&pane)
                .and_then(|model| model.hit_entry_at_content_offset(cursor_offset))
                .and_then(|idx| {
                    app.details_row_selected(pane, idx).then(|| {
                        app.state
                            .tab_states
                            .get(&pane)
                            .and_then(|ts| ts.entries.get(idx))
                            .map(|entry| (idx, entry.path.clone()))
                    })?
                })
        };

        let Some((idx, anchor)) = item else {
            tracing::debug!("dnd details content drag ignored outside selected row");
            return empty_drag_view(cx);
        };

        build_file_drag_start(&view, Some((pane, idx)), anchor, _window, cx)
    }
}

/// Drag source for the retained Grid surface.  The hit model is populated in
/// `GridSurfaceElement::prepaint`, so the closure does no widget traversal.
pub(crate) fn file_drag_handler_for_grid_content(
    view: Entity<App>,
    pane: TabId,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    file_drag_handler_for_surface_content(view, pane, false)
}

pub(crate) fn file_drag_handler_for_list_content(
    view: Entity<App>,
    pane: TabId,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    file_drag_handler_for_surface_content(view, pane, true)
}

fn file_drag_handler_for_surface_content(
    view: Entity<App>,
    pane: TabId,
    list: bool,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    move |_d, cursor_offset, _window, cx| {
        let item = {
            let app = view.read(&*cx);
            let hit_model = if list {
                app.list_hit.get(&pane)
            } else {
                app.grid_hit.get(&pane)
            };
            hit_model
                .and_then(|model| model.hit_entry_at_content_offset(cursor_offset))
                .and_then(|idx| {
                    app.details_row_selected(pane, idx).then(|| {
                        app.state
                            .tab_states
                            .get(&pane)
                            .and_then(|ts| ts.entries.get(idx))
                            .map(|entry| (idx, entry.path.clone()))
                    })?
                })
        };

        let Some((idx, anchor)) = item else {
            tracing::debug!(
                surface = if list { "list" } else { "grid" },
                "dnd surface content drag ignored outside selected cell"
            );
            return empty_drag_view(cx);
        };

        build_file_drag_start(&view, Some((pane, idx)), anchor, _window, cx)
    }
}

fn build_file_drag_handler(
    view: Entity<App>,
    item: Option<(TabId, usize)>,
    anchor: PathBuf,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    move |_d, _pos, _window, cx| build_file_drag_start(&view, item, anchor.clone(), _window, cx)
}

fn empty_drag_view(cx: &mut gpui::App) -> Entity<DragView> {
    #[cfg(windows)]
    {
        cx.new(|_| DragNoop)
    }
    #[cfg(not(windows))]
    {
        cx.new(|_| DraggedFiles { paths: Vec::new() })
    }
}

fn build_file_drag_start(
    view: &Entity<App>,
    item: Option<(TabId, usize)>,
    anchor: PathBuf,
    _window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<DragView> {
    if view.read(&*cx).rubber.is_some() {
        tracing::debug!(anchor = %anchor.display(), "dnd file drag blocked by active rubber");
        return empty_drag_view(cx);
    }
    // Recycle Bin rows carry mangled `$R` payload paths and remote rows carry
    // virtual paths — an OS drag would ship garbage. Block at the source.
    {
        use atlas_model::location::Location;
        let source_pane = item.map(|(pane, _)| pane);
        let blocked_key = {
            let app = view.read(&*cx);
            let pane = source_pane.unwrap_or(app.state.focused_tile);
            match app.tab_data_for(pane).map(|t| &t.location) {
                Some(Location::RecycleBin) => Some("dnd_recycle_drag_blocked"),
                #[cfg(feature = "provider-host")]
                Some(Location::Remote { .. }) => Some("dnd_remote_drag_blocked"),
                _ => None,
            }
        };
        if let Some(key) = blocked_key {
            view.update(cx, |app, cx| {
                let msg = if key == "dnd_recycle_drag_blocked" {
                    rust_i18n::t!("dnd_recycle_drag_blocked")
                } else {
                    rust_i18n::t!("dnd_remote_drag_blocked")
                };
                app.toast_error(msg.into_owned());
                cx.notify();
            });
            return empty_drag_view(cx);
        }
    }
    view.update(cx, |app, cx| {
        if let Some((pane, idx)) = item {
            app.ensure_drag_selection(pane, idx);
        }
        app.begin_drag_hover();
        cx.notify();
    });
    // Full multi-selection at drag-start time (fix: was single entry).
    let (paths, archive_items) = {
        let app = view.read(&*cx);
        let mut arc = app.state.selected_archive_items();
        // Fallback: single-entry archive drag when the selection set is empty.
        if let Some(ref mut items) = arc {
            if items.is_empty() {
                if let Some(archive) = app
                    .state
                    .active_tab_data()
                    .and_then(|t| t.location.as_archive().map(|(a, _)| a.to_path_buf()))
                {
                    items.push((archive, anchor.clone()));
                }
            }
        }
        // Archive tabs: entry paths are virtual (nothing on disk) — passing
        // them as real paths would raise one "path does not exist" error per
        // selected item. The extraction drag rides on `arc` alone.
        let p = if arc.is_some() {
            Vec::new()
        } else {
            let p = app.state.selected_paths();
            if p.is_empty() {
                vec![anchor.clone()]
            } else {
                p
            }
        };
        (p, arc)
    };
    // Windows: schedule DoDragDrop via PostMessage so it runs AFTER this closure
    // returns and gpui's App RefCell is no longer borrowed (prevents a
    // "RefCell already borrowed" panic from DoDragDrop's nested message loop).
    #[cfg(windows)]
    {
        tracing::debug!(paths = paths.len(), "dnd file drag source triggered");
        view.read(&*cx)
            .trigger_os_drag(paths.clone(), archive_items);
        // Invisible zero-size placeholder; OLE provides cursor feedback and a
        // synthetic WM_LBUTTONUP clears gpui's active_drag without firing any
        // on_drop handler (DragNoop has no matching handlers).
        cx.new(|_| DragNoop)
    }
    #[cfg(not(windows))]
    {
        let _ = archive_items;
        let urls: Vec<String> = paths
            .iter()
            .map(|p| format!("file://{}", p.display()))
            .collect();
        _window.start_urilist_drag(urls);
        cx.new(|_| DraggedFiles { paths })
    }
}
