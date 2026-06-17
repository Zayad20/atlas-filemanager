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
use crate::ui::root::DraggedFiles;

/// The view entity gpui renders as the drag ghost. On Windows the OLE path owns
/// the cursor, so we hand gpui an invisible [`DragNoop`]; elsewhere gpui paints
/// the [`DraggedFiles`] ghost itself.
#[cfg(windows)]
pub(crate) type DragView = DragNoop;
#[cfg(not(windows))]
pub(crate) type DragView = DraggedFiles;

/// Build the shared `on_drag` move-closure for a file row/cell. Drags the whole
/// current selection (falling back to just `anchor` when nothing is selected),
/// routing through the Windows OLE `DoDragDrop` path when present.
///
/// Usage: `.on_drag(DraggedFiles { paths: vec![path.clone()] }, file_drag_handler(view.clone(), path.clone()))`.
pub(crate) fn file_drag_handler(
    view: Entity<App>,
    anchor: PathBuf,
) -> impl Fn(&DraggedFiles, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    move |_d, _pos, _window, cx| {
        view.update(cx, |app, cx| {
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
            let p = app.state.selected_paths();
            let p = if p.is_empty() {
                vec![anchor.clone()]
            } else {
                p
            };
            (p, arc)
        };
        // Windows: schedule DoDragDrop via PostMessage so it runs AFTER this closure
        // returns and gpui's App RefCell is no longer borrowed (prevents a
        // "RefCell already borrowed" panic from DoDragDrop's nested message loop).
        #[cfg(windows)]
        {
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
            cx.new(|_| DraggedFiles { paths })
        }
    }
}
