//! List (compact) view: `render_list_compact` + compact row cell.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;
use crate::state::tab::LoadState;
use crate::ui::glass::GLASS_RADIUS;
use crate::ui::icons::file_type_icon;
use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, AnyElement, Context,
    Entity, Hsla, MouseButton,
    RenderImage, SharedString, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::sync::Arc;
use crate::ui::root::{ctrl_scroll, rect_recorder, zone_recorder, DragNoop, DraggedFiles};

impl App {
    /// Compact List view: small `icon + name` cells flowing row-major into as many
    /// columns as fit the width — the classic Windows "List" layout. Virtualized by
    /// row (`uniform_list`) like the grid, so huge dirs stay smooth.
    pub(crate) fn render_list_compact(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let focused = self.state.focused_tile == pane_id;
        let (fg, accent, accent_fg, hover, muted, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                t.secondary,
            )
        };
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        #[allow(clippy::items_after_statements)] // local constant kept next to its use
        const CELL_W: f32 = 220.0;
        let cell_stride = CELL_W + 6.0;
        let avail = (f32::from(window.viewport_size().width) - 217.0).max(cell_stride);
        let cols = ((avail / cell_stride).floor() as usize).max(1);
        let row_count = count.div_ceil(cols);

        let inner = if count == 0 {
            let msg = match self.state.tab_states.get(&pane_id).map(|t| &t.load_state) {
                Some(LoadState::Loading) => "Cargando…",
                Some(LoadState::Error(_)) => "No se pudo leer la carpeta",
                _ => "Carpeta vacía",
            };
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let list_view = view.clone();
            uniform_list(
                SharedString::from(format!("listc-{}", pane_id.0)),
                row_count,
                move |range, _window, cx| {
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
                    let record_rects = app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
                    range
                        .map(|row| {
                            let mut cells: Vec<AnyElement> = Vec::with_capacity(cols);
                            for c in 0..cols {
                                let vi = row * cols + c;
                                let Some(&entry_idx) = ts.visible.get(vi) else {
                                    break;
                                };
                                let Some(entry) = ts.entries.get(entry_idx) else {
                                    continue;
                                };
                                let selected = ts.selection.selected.contains(&entry_idx);
                                let icon_tex =
                                    app.icon_tex_for_entry(&entry.kind, entry.extension.as_deref());
                                cells.push(render_cell_compact(
                                    entry,
                                    entry_idx,
                                    selected,
                                    pane_id,
                                    &list_view,
                                    accent,
                                    accent_fg,
                                    hover,
                                    record_rects,
                                    icon_tex,
                                ));
                            }
                            h_flex().w_full().gap(px(6.)).px_2().children(cells)
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let zone_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            // The pane content is transparent: the hosting TabPanel (fork) is the
            // rounded glass card, with the tab selector inside it. Clip the list to the
            // card radius; show an accent ring only on the focused pane.
            .overflow_hidden()
            .when(focused, |s| {
                s.rounded(px(GLASS_RADIUS)).border_2().border_color(accent)
            })
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
                        this.begin_rubber(pane_id, pos, additive);
                        cx.notify();
                    });
                }
            })
            .on_drop::<DraggedFiles>({
                let v = rubber_view.clone();
                move |d, _window, cx| {
                    let paths = d.paths.clone();
                    v.update(cx, |this, cx| {
                        if let Some(dest) = this.tab_data_for(pane_id).and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
                            this.process_actions(vec![Action::Move { paths, dest }]);
                            cx.notify();
                        }
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        if let Some(dest) = this.tab_data_for(pane_id).and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
                            this.process_actions(vec![Action::CopyTo { paths, dest }]);
                            cx.notify();
                        }
                    });
                }
            })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |ev, _window, cx| ctrl_scroll(ev, pane_id, &scroll_view, cx))
            // FASE F: register this pane's window-space rect each frame for OLE drop routing.
            .when(cfg!(windows), |el| {
                el.on_prepaint(zone_recorder(pane_id, &zone_view))
            })
            .child(crumb_strip)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .child(inner)
                    .child(filter_pill)
                    .child(controls_pill),
            )
            .into_any_element()
    }

}

/// One compact List cell: 16px icon + truncated name in a fixed 220px row. Click
/// routing mirrors the Details rows (shift = range, ctrl = toggle, dbl = open).
#[allow(clippy::too_many_arguments)]
fn render_cell_compact(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    record_rects: bool,
    icon_tex: Option<Arc<RenderImage>>,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!(
            "lc-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(220.))
        .px_1()
        .py(px(2.))
        .gap_1()
        .items_center()
        .rounded_sm()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(
            div().w(px(16.)).flex_none().child(match icon_tex {
                Some(tex) => img(tex)
                    .w(px(14.))
                    .h(px(14.))
                    .flex_none()
                    .into_any_element(),
                None => file_type_icon(&entry.kind, entry.extension.as_deref(), 14.0),
            }),
        )
        .child(div().flex_1().min_w(px(0.)).truncate().child(name))
        .on_click(move |ev, _window, cx| {
            let m = ev.modifiers();
            let shift = m.shift;
            let ctrl = m.control || m.platform;
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if activate {
                    this.process_actions(vec![Action::Activate {
                        tile: pane_id,
                        idx: entry_idx,
                    }]);
                } else {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift,
                        ctrl,
                    }]);
                }
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, {
            let rv = view.clone();
            move |ev, window, cx| {
                let pos = ev.position;
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
                    // Right-click opens the Atlas–style menu on the (now
                    // selected) item; native shell items stream in afterwards.
                    let paths = this.selected_paths();
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .when(!selected, |s| {
            let rv = view.clone();
            s.on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                let (pos, additive) = (ev.position, ev.modifiers.control || ev.modifiers.platform);
                rv.update(cx, |this, cx| {
                    this.begin_rubber(pane_id, pos, additive);
                    cx.notify();
                });
                cx.stop_propagation();
            })
        })
        .when(selected, |s| {
            s.on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                cx.stop_propagation();
            })
            .on_drag(
                DraggedFiles {
                    paths: vec![entry.path.clone()],
                },
                {
                    let drag_view = view.clone();
                    let anchor = entry.path.clone();
                    move |_d, _pos, _window, cx| {
                        // Full multi-selection at drag-start time (fix: was single entry).
                        let (paths, archive_items) = {
                            let app = drag_view.read(&*cx);
                            let mut arc = app.state.selected_archive_items();
                            // Fallback: single-entry drag when nothing is in the selection set.
                            if let Some(ref mut items) = arc {
                                if items.is_empty() {
                                    if let Some(archive) =
                                        app.state.active_tab_data().and_then(|t| t.location.as_archive().map(|(a, _)| a.to_path_buf()))
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
                        // Windows: schedule DoDragDrop via PostMessage so it runs
                        // AFTER this closure returns and gpui's App RefCell is
                        // no longer borrowed (prevents "RefCell already borrowed"
                        // panic from DoDragDrop's nested Win32 message loop).
                        #[cfg(windows)]
                        {
                            drag_view
                                .read(&*cx)
                                .trigger_os_drag(paths.clone(), archive_items);
                            // Return an invisible zero-size placeholder. OLE provides
                            // cursor feedback; a synthetic WM_LBUTTONUP injected after
                            // DoDragDrop clears gpui's active_drag without firing any
                            // on_drop handler (DragNoop has no matching handlers).
                            return cx.new(|_| DragNoop);
                        }
                        // Non-Windows: OLE path absent; gpui renders the ghost itself.
                        #[cfg(not(windows))]
                        {
                            let _ = archive_items;
                        }
                        #[cfg(not(windows))]
                        cx.new(|_| DraggedFiles { paths })
                    }
                },
            )
        })
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            let dest_move = entry.path.clone();
            let dest_copy = entry.path.clone();
            let mv = view.clone();
            let cpv = view.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let (paths, dest) = (d.paths.clone(), dest_move.clone());
                    mv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::Move { paths, dest }]);
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let (paths, dest) = (paths.paths().to_vec(), dest_copy.clone());
                    cpv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::CopyTo { paths, dest }]);
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}

