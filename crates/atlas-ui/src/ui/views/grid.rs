//! Grid (icons) view: `render_grid` + grid cell + thumbnail predicate helpers.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::root::{ctrl_scroll, pane_bounds_recorder, rect_recorder, DraggedFiles};
use crate::ui::views::virtual_grid::AnimatedVirtualGrid;
use gpui::prelude::*;
use gpui::{div, img, px, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme, ElementExt};
use std::ops::Range;

impl App {
    /// Icons (Grid) view: a virtualized grid of fixed-size cells. Column count is
    /// derived from the viewport width and the per-tab thumbnail size, so it stays
    /// virtualized (only visible rows built) for huge directories.
    pub(crate) fn render_grid(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.view.grid");
        let view = cx.entity();
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let (fg, accent, accent_fg, hover, muted, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                if figma_glass {
                    glass_header_fill()
                } else {
                    backdrop_tint(t.secondary, 0.18)
                },
            )
        };
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
        let thumb = self
            .tab_data_for(pane_id)
            .map_or(128.0, |t| t.thumbnail_size);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        const GRID_GAP: f32 = 8.0;
        const GRID_PAD: f32 = 0.0;
        // Use the width captured by on_prepaint from the previous frame. This
        // reflects the real pane size and adapts to preview panels and splitter
        // drags. Fall back to a viewport estimate on the very first frame before
        // any paint has run.
        let pane_w = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.last_pane_width)
            .filter(|&w| w > 1.0)
            .unwrap_or_else(|| (f32::from(window.viewport_size().width) - 217.0).max(0.0));
        let inner_avail = (pane_w - 2.0 * GRID_PAD).max(thumb);
        // n*(thumb+GAP) - GAP = inner_avail  →  n = ⌊(inner_avail + GAP) / (thumb + GAP)⌋
        let cols = (((inner_avail + GRID_GAP) / (thumb + GRID_GAP)).floor() as usize).max(1);
        let _perf = crate::perf::span_detail("grid.render", || {
            format!(
                "pane={} count={count} cols={cols} thumb={thumb:.1} pane_w={pane_w:.1}",
                pane_id.0
            )
        });

        // Hoist scroll handle so the outer relative wrapper can host the scrollbar
        // (vertical_scrollbar must NOT be inside overflow_y_scroll — it scrolls away).
        let scroll = self.scroll_handle_for(pane_id);
        let base_scroll = scroll.0.borrow().base_handle.clone();

        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, rust_i18n::t!("empty_folder").into_owned());
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .children(msg)
                .into_any_element()
        } else {
            let mut scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            let viewport_h = {
                let tracked = f32::from(base_scroll.bounds().size.height);
                if tracked > 1.0 {
                    tracked
                } else {
                    f32::from(window.viewport_size().height)
                }
            };
            let cell_h = thumb * 0.62 + 32.0;
            let pitch = cell_h + GRID_GAP;
            let rows = count.div_ceil(cols);
            let content_h = if rows == 0 {
                0.0
            } else {
                rows as f32 * pitch - GRID_GAP
            };
            let max_scroll = (content_h - viewport_h).max(0.0);
            if scroll_y > max_scroll {
                let offset = base_scroll.offset();
                base_scroll.set_offset(gpui::point(offset.x, gpui::px(-max_scroll)));
                scroll_y = max_scroll;
            }
            let start_row = (scroll_y / pitch).floor().max(0.0) as usize;
            let end_row = ((scroll_y + viewport_h) / pitch).ceil() as usize + 1;
            let start = (start_row * cols).min(count);
            let end = ((end_row * cols).min(count)).max(start);
            self.prepare_grid_visible(pane_id, start..end);

            let list_view = view.clone();
            let inner_scroll_view = view.clone();
            div()
                .id(SharedString::from(format!(
                    "anim-grid-scroll-{}",
                    pane_id.0
                )))
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&base_scroll)
                .on_scroll_wheel(move |ev, _window, cx| {
                    ctrl_scroll(ev, pane_id, &inner_scroll_view, cx);
                    cx.stop_propagation();
                })
                .child(AnimatedVirtualGrid::new(
                    SharedString::from(format!("anim-grid-{}", pane_id.0)),
                    count,
                    cols,
                    gpui::Size {
                        width: gpui::px(thumb),
                        height: gpui::px(cell_h),
                    },
                    gpui::Size {
                        width: gpui::px(GRID_GAP),
                        height: gpui::px(GRID_GAP),
                    },
                    scroll_y,
                    viewport_h,
                    !self.state.settings.disable_animations,
                    move |idx, window, cx| {
                        let app = list_view.read(cx);
                        let Some(ts) = app.state.tab_states.get(&pane_id) else {
                            return div().into_any_element();
                        };
                        let Some(&entry_idx) = ts.visible.get(idx) else {
                            return div().into_any_element();
                        };
                        let Some(entry) = ts.entries.get(entry_idx) else {
                            return div().into_any_element();
                        };
                        let file_icon = app.file_icon_for_entry(
                            &entry.path,
                            &entry.kind,
                            entry.extension.as_deref(),
                        );
                        let record_rects = true;
                        let flashing = matches!(entry.kind, FileKind::Dir)
                            && app.drag_hover.blink_id.as_ref()
                                == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                        let rename_input =
                            crate::ui::views::rename::active_input(app, pane_id, entry_idx);
                        let selected = ts.selection.selected.contains(&entry_idx)
                            && !crate::ui::views::rename::suppress_row_chrome(
                                app, pane_id, entry_idx,
                            );
                        let cell = render_cell(
                            entry,
                            entry_idx,
                            selected,
                            pane_id,
                            &list_view,
                            thumb,
                            thumb,
                            file_icon,
                            accent,
                            accent_fg,
                            hover,
                            flashing,
                            record_rects,
                            rename_input,
                            window,
                        );
                        cell
                    },
                ))
                .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
                        if this.rename_edit.is_some() {
                            this.commit_rename(cx);
                        }
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
                        this.drop_files_on_pane(pane_id, paths, false);
                        cx.notify();
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        this.drop_files_on_pane(pane_id, paths, true);
                        cx.notify();
                    });
                }
            })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.push_focus_history();
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |ev, _window, cx| ctrl_scroll(ev, pane_id, &scroll_view, cx))
            // Capture the real pane pixel width every frame so the next frame's
            // column calculation uses the actual container size (handles preview
            // panel open/close and splitter drags, not just window resize).
            .on_prepaint(move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                crate::perf::count("prepaint.grid_pane");
                let new_w = f32::from(bounds.size.width);
                prepaint_view.update(cx, |this, cx| {
                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                        if (ts.last_pane_width - new_w).abs() > 0.5 {
                            let old_w = ts.last_pane_width;
                            crate::perf::event(|| {
                                format!(
                                    "grid.width pane={} old={old_w:.1} new={new_w:.1}",
                                    pane_id.0
                                )
                            });
                            ts.last_pane_width = new_w;
                            cx.notify();
                        }
                    }
                    // FASE F: register this pane's bounds for OLE drop routing.
                    this.push_pane_zone(pane_id, bounds);
                });
            })
            .child(crumb_strip)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .when(figma_glass, |s| s.bg(glass_fill()))
                    .rounded_bl(px(12.0))
                    .rounded_br(px(12.0))
                    .on_prepaint(pane_bounds_recorder(pane_id, &bounds_view))
                    .child(inner)
                    .child(filter_pill)
                    .child(controls_pill)
                    .vertical_scrollbar(&base_scroll),
            )
            .into_any_element()
    }

    fn prepare_grid_visible(&mut self, pane_id: TabId, range: Range<usize>) {
        let windows_theme = crate::ui::icons::is_windows_theme(&self.state.profiles.active().theme);
        let (to_request, to_request_ext, to_request_shell, all_visible) = {
            let Some(ts) = self.state.tab_states.get(&pane_id) else {
                return;
            };
            let mut imgs = Vec::new();
            let mut exts: Vec<(
                std::path::PathBuf,
                String,
                std::collections::HashMap<String, std::path::PathBuf>,
            )> = Vec::new();
            // Windows: non-native-image files routed through IShellItemImageFactory.
            let mut shell_paths: Vec<(std::path::PathBuf, bool)> = Vec::new();
            let mut all_visible = Vec::new();
            for vi in range {
                let Some(&entry_idx) = ts.visible.get(vi) else {
                    break;
                };
                let Some(entry) = ts.entries.get(entry_idx) else {
                    continue;
                };
                let thumbable = is_thumbnailable(entry);
                let not_thumbable_file = !thumbable && entry.kind == FileKind::File;
                let windows_folder = windows_theme && entry.kind == FileKind::Dir;

                // On Windows all non-native-image files use the Shell thumbnail cache
                // (IShellItemImageFactory) — covers video, RAW, HEIC, PDF, etc. with
                // zero processes spawned and persistent cross-session caching.
                // On other platforms keep the existing extension thumbnailer (ffmpeg).
                #[cfg(target_os = "windows")]
                let use_shell = not_thumbable_file || windows_folder;
                #[cfg(not(target_os = "windows"))]
                let use_shell = false;

                #[cfg(not(target_os = "windows"))]
                let ext_cmd = if not_thumbable_file {
                    entry.extension.as_deref().and_then(|ext| {
                        self.ext_registry
                            .thumbnailer_for(ext)
                            .map(|cmd| (cmd.command.run.clone(), cmd.tools))
                    })
                } else {
                    None
                };
                #[cfg(target_os = "windows")]
                let ext_cmd: Option<(
                    String,
                    std::collections::HashMap<String, std::path::PathBuf>,
                )> = None;

                if thumbable || use_shell || ext_cmd.is_some() {
                    all_visible.push(entry.path.clone());
                }
                if self.thumb_textures.contains_key(&entry.path) {
                    continue;
                }
                if thumbable {
                    imgs.push(entry.path.clone());
                } else if use_shell {
                    shell_paths.push((entry.path.clone(), windows_folder));
                } else if let Some((run, tools)) = ext_cmd {
                    exts.push((entry.path.clone(), run, tools));
                }
            }
            (imgs, exts, shell_paths, all_visible)
        };

        {
            let mut wanted = self.thumb_wanted.lock().unwrap();
            wanted.clear();
            wanted.extend(all_visible);
        }
        for path in to_request {
            self.request_thumbnail(path);
        }
        for (path, run, tools) in to_request_ext {
            self.request_ext_thumbnail(path, run, tools);
        }
        #[cfg(target_os = "windows")]
        for (path, allow_icon) in to_request_shell {
            if allow_icon {
                self.request_shell_thumbnail_or_icon(path);
            } else {
                self.request_shell_thumbnail(path);
            }
        }
        // On non-Windows to_request_shell is always empty; drop it to avoid unused warning.
        #[cfg(not(target_os = "windows"))]
        drop(to_request_shell);
    }
}

/// Extensions worth decoding into a grid thumbnail.
fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// Whether this entry should get a downscaled thumbnail (an image file).
fn is_thumbnailable(entry: &FileEntry) -> bool {
    matches!(entry.kind, FileKind::File) && entry.extension.as_deref().is_some_and(is_image_ext)
}

/// One grid cell: thumbnail (or a scaled glyph fallback while it loads / for
/// non-images) + truncated name, fixed to the thumbnail size. The texture is a
/// cached, downscaled `RenderImage` (see `ui::thumbs`) — never a full-res decode.
#[allow(clippy::too_many_arguments)]
fn render_cell(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    thumb: f32,
    cell_w: f32,
    file_icon: FileIcon,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    flashing: bool,
    record_rects: bool,
    rename_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    window: &mut Window,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    let font_size = window.rem_size() * 0.75;
    v_flex()
        .id(SharedString::from(format!(
            "cell-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(cell_w))
        .p_1()
        .gap_1()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .when(selected && rename_input.is_none(), |s| {
            s.bg(accent).text_color(accent_fg)
        })
        .when(flashing && !selected, |s| s.bg(accent.opacity(0.38)))
        .when(!selected && rename_input.is_none(), |s| {
            s.hover(|h| h.bg(hover))
        })
        .child({
            let area = div()
                .h(px(thumb * 0.62))
                .flex()
                .items_center()
                .justify_center();
            match file_icon {
                FileIcon::PreviewTexture(tex) => area.child(
                    img(tex)
                        .max_h(px(thumb * 0.6))
                        .max_w(px(thumb))
                        .rounded_sm(),
                ),
                FileIcon::IconTexture(tex) => area.child(
                    img(tex)
                        .max_h(px(thumb * 0.56))
                        .max_w(px(thumb * 0.56))
                        .flex_none(),
                ),
                FileIcon::Builtin { kind, extension } => {
                    area.child(FileIcon::Builtin { kind, extension }.element(thumb * 0.42, accent))
                }
            }
        })
        .child(match &rename_input {
            Some(input) => div()
                .w(px(crate::ui::views::rename::input_width(
                    &name, cell_w, font_size, window,
                )))
                .h(px(crate::ui::views::rename::INLINE_INPUT_H))
                .text_xs()
                .child(crate::ui::views::rename::input_element(
                    input,
                    view,
                    crate::ui::views::rename::INLINE_INPUT_H,
                    font_size,
                ))
                .into_any_element(),
            None => div()
                .w_full()
                .text_center()
                .text_xs()
                .truncate()
                .child(name)
                .into_any_element(),
        })
        .on_click(move |ev, _window, cx| {
            let m = ev.modifiers();
            let shift = m.shift;
            let ctrl = m.control || m.platform;
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if this.rename_edit.is_some() {
                    this.commit_rename(cx);
                }
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
                    let paths = this.select_context_menu_paths(pane_id, entry_idx);
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .when(rename_input.is_none(), |s| {
            s.on_drag(
                crate::ui::drag::file_drag_value(vec![entry.path.clone()]),
                crate::ui::drag::file_drag_handler_for_item(
                    view.clone(),
                    pane_id,
                    entry_idx,
                    entry.path.clone(),
                ),
            )
        })
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            let dest = entry.path.clone();
            let hover_id = DragHoverId::Folder(pane_id, dest.clone());
            let dest_internal = dest.clone();
            let dest_external = dest.clone();
            let mv = view.clone();
            let cpv = view.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Folder,
                    DragHoverCommand::NavigatePathIn(pane_id, dest.clone()),
                    DragHoverDropCommand::TransferTo(dest.clone()),
                    view,
                ))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let paths = d.paths.clone();
                    mv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_internal.clone()),
                            paths,
                            true,
                        );
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    cpv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_external.clone()),
                            paths,
                            false,
                        );
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}
