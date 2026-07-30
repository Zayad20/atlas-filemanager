//! Retained, flat Grid surface.
//!
//! Grid used to build one GPUI subtree per visible cell.  This element keeps the
//! same interaction contract as Details, but projects every cell with the small
//! `(row, col) -> (x, y)` grid equation and paints the result directly.

use crate::app::{App, GridHitModel};
use crate::ui::drag_hover::{
    DragHoverCommand, DragHoverDropCommand, DragHoverId, DragHoverKind, DragHoverTarget,
};
use crate::ui::file_icons::FileIcon;
use crate::ui::icons::AtlasIcon;
use crate::ui::selection_checkbox;
use crate::ui::surface::paint_rounded;
use crate::ui::views::grid::{
    GRID_BUILTIN_ICON, GRID_ICON_AREA, GRID_LABEL_H, GRID_PREVIEW_MAX, GRID_TEXTURE_ICON_MAX,
};
use atlas_model::file_entry::FileKind;
use atlas_model::pane::TabId;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, Corners, DispatchPhase, Element,
    ElementId, Font, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Refineable,
    ShapedLine, SharedString, Style, StyleRefinement, Styled, TextAlign, WeakEntity, Window,
};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
struct GridSurfaceSlot {
    row: usize,
    shape_key: u64,
    selected: bool,
    selected_at: Option<Instant>,
    hovered: bool,
    flashing: bool,
    dimmed: bool,
    name: ShapedLine,
    icon: FileIcon,
}

#[derive(Clone, Default)]
struct GridSurfaceState {
    slots: Vec<Option<GridSurfaceSlot>>,
}

impl GridSurfaceState {
    fn ensure_capacity(&mut self, n: usize) {
        let n = n.max(1);
        if self.slots.len() < n {
            self.slots.resize_with(n, || None);
        }
    }

    fn index(&self, row: usize) -> usize {
        row % self.slots.len()
    }

    fn sync(
        &mut self,
        row: usize,
        key: u64,
        fill: impl FnOnce() -> GridSurfaceSlot,
    ) -> &GridSurfaceSlot {
        if self.slots.is_empty() {
            self.ensure_capacity(1);
        }
        let ix = self.index(row);
        let hit = matches!(&self.slots[ix], Some(slot) if slot.row == row && slot.shape_key == key);
        if hit {
            crate::perf::count("grid.surface.ring.hit");
        } else {
            crate::perf::count("grid.surface.ring.miss");
            self.slots[ix] = Some(fill());
        }
        self.slots[ix]
            .as_ref()
            .expect("grid surface slot just filled")
    }

    fn touch_flags(
        &mut self,
        row: usize,
        key: u64,
        selected: bool,
        hovered: bool,
        flashing: bool,
        dimmed: bool,
    ) {
        if self.slots.is_empty() {
            return;
        }
        let ix = self.index(row);
        if let Some(slot) = self.slots[ix].as_mut() {
            if slot.row == row && slot.shape_key == key {
                if slot.selected != selected {
                    slot.selected_at = selected.then(Instant::now);
                }
                slot.selected = selected;
                slot.hovered = hovered;
                slot.flashing = flashing;
                slot.dimmed = dimmed;
            }
        }
    }

    fn collect(&self, start: usize, end: usize) -> Vec<GridSurfaceSlot> {
        (start..end)
            .filter_map(|row| {
                let slot = self.slots.get(self.index(row))?.as_ref()?;
                (slot.row == row).then(|| slot.clone())
            })
            .collect()
    }
}

pub(crate) struct GridSurfaceElement {
    id: ElementId,
    pane_id: TabId,
    app: WeakEntity<App>,
    visible: Arc<Vec<usize>>,
    start_idx: usize,
    end_idx: usize,
    total_count: usize,
    cols: usize,
    cell_w: f32,
    cell_h: f32,
    pitch_x: f32,
    pitch_y: f32,
    thumb: f32,
    scroll_y: f32,
    viewport_h: f32,
    colors: GridSurfaceColors,
    font: Font,
    font_size: Pixels,
    style: StyleRefinement,
}

#[derive(Clone, Copy)]
pub(crate) struct GridSurfaceColors {
    pub(crate) fg: Hsla,
    pub(crate) accent: Hsla,
    pub(crate) accent_fg: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) muted: Hsla,
}

pub(crate) struct GridSurfacePrepaint {
    slots: Vec<GridSurfaceSlot>,
    show_checkboxes: bool,
    hitbox: Hitbox,
}

impl GridSurfaceElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: TabId,
        app: WeakEntity<App>,
        visible: Arc<Vec<usize>>,
        start_idx: usize,
        end_idx: usize,
        cols: usize,
        total_count: usize,
        cell_w: f32,
        cell_h: f32,
        pitch_x: f32,
        pitch_y: f32,
        thumb: f32,
        scroll_y: f32,
        viewport_h: f32,
        colors: GridSurfaceColors,
        font: Font,
        font_size: Pixels,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("grid-surface-{}", pane_id.0))),
            pane_id,
            app,
            visible,
            start_idx,
            end_idx,
            total_count,
            cols: cols.max(1),
            cell_w,
            cell_h,
            pitch_x,
            pitch_y,
            thumb,
            scroll_y,
            viewport_h,
            colors,
            font,
            font_size,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for GridSurfaceElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for GridSurfaceElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for GridSurfaceElement {
    type RequestLayoutState = Style;
    type PrepaintState = GridSurfacePrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> (LayoutId, Self::RequestLayoutState) {
        crate::perf::count("grid.surface.layout");
        let mut style = Style::default();
        style.refine(&self.style);
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        let _perf = crate::perf::span_detail("grid.surface.prepaint", || {
            format!(
                "pane={} visible={}..{} cols={}",
                self.pane_id.0, self.start_idx, self.end_idx, self.cols
            )
        });
        crate::perf::count("grid.surface.prepaint");
        crate::perf::count_by(
            "grid.surface.slots",
            self.end_idx.saturating_sub(self.start_idx) as u64,
        );

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let Some(app_entity) = self.app.upgrade() else {
            return GridSurfacePrepaint {
                slots: Vec::new(),
                show_checkboxes: false,
                hitbox,
            };
        };

        let mut state = GridSurfaceState::default();
        window.with_optional_element_state(
            id,
            |state_opt: Option<Option<GridSurfaceState>>, _window| {
                state = state_opt.flatten().unwrap_or_default();
                ((), Some(state.clone()))
            },
        );
        state.ensure_capacity((self.end_idx.saturating_sub(self.start_idx) + 8).max(16));

        let pane_id = self.pane_id;
        let visible = self.visible.clone();
        let start_idx = self.start_idx;
        let end_idx = self.end_idx;
        let cols = self.cols;
        let colors = self.colors;
        let font = self.font.clone();
        let font_size = self.font_size;
        let model = GridHitModel {
            bounds: Bounds {
                origin: point(bounds.origin.x, bounds.origin.y + px(self.scroll_y)),
                size: size(bounds.size.width, px(self.viewport_h)),
            },
            scroll_y: self.scroll_y,
            cell_w: self.cell_w,
            cell_h: self.cell_h,
            cols,
            pitch_y: self.pitch_y,
            pitch_x: self.pitch_x,
            visible: visible.clone(),
            start_idx,
            end_idx,
            total_count: self.total_count,
        };

        let (slots, show_checkboxes) = app_entity.update(cx, |app, _| {
            app.grid_hit.insert(pane_id, model.clone());

            if app.drag_hover_maybe_active() {
                let targets = app
                    .state
                    .tab_states
                    .get(&pane_id)
                    .map(|tab_state| {
                        (start_idx..end_idx)
                            .filter_map(|visible_pos| {
                                let entry_idx = *visible.get(visible_pos)?;
                                let entry = tab_state.entries.get(entry_idx)?;
                                matches!(entry.kind, FileKind::Dir).then(|| DragHoverTarget {
                                    id: DragHoverId::Folder(pane_id, entry.path.clone()),
                                    bounds: model.cell_bounds(visible_pos),
                                    kind: DragHoverKind::Folder,
                                    command: DragHoverCommand::NavigatePathIn(
                                        pane_id,
                                        entry.path.clone(),
                                    ),
                                    drop_command: Some(DragHoverDropCommand::TransferTo(
                                        entry.path.clone(),
                                    )),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for target in targets {
                    app.register_drag_hover_target(target);
                }
            }

            let Some(tab_state) = app.state.tab_states.get(&pane_id) else {
                return (Vec::new(), false);
            };
            let show_checkboxes = tab_state.selection.selected.len() > 1;
            let text_system = window.text_system();

            for row in start_idx..end_idx {
                let Some(&entry_idx) = visible.get(row) else {
                    continue;
                };
                let Some(entry) = tab_state.entries.get(entry_idx) else {
                    continue;
                };
                if crate::ui::views::rename::active_input(app, pane_id, entry_idx).is_some() {
                    continue;
                }
                let selected = tab_state.selection.selected.contains(&entry_idx)
                    && !crate::ui::views::rename::suppress_row_chrome(app, pane_id, entry_idx);
                let dimmed = crate::ui::views::rename::dim_other_entry(app, pane_id, entry_idx);
                let hovered = app.grid_hover == Some((pane_id, entry_idx));
                let flashing = matches!(entry.kind, FileKind::Dir)
                    && app.drag_hover.blink_id.as_ref()
                        == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                let name_color = if selected {
                    colors.accent_fg
                } else {
                    colors.fg
                };
                let name_color = if dimmed {
                    name_color.opacity(0.45)
                } else {
                    name_color
                };
                let key = grid_shape_key(
                    entry_idx,
                    &entry.name,
                    selected,
                    dimmed,
                    font_size,
                    name_color,
                );
                let name = entry.name.to_string();
                let icon =
                    app.file_icon_for_entry(&entry.path, &entry.kind, entry.extension.as_deref());
                let previous_selected = state
                    .slots
                    .get(state.index(row))
                    .and_then(Option::as_ref)
                    .filter(|slot| slot.row == row && slot.shape_key == key)
                    .is_some_and(|slot| slot.selected);
                let _ = state.sync(row, key, || GridSurfaceSlot {
                    row,
                    shape_key: key,
                    selected,
                    selected_at: (selected && !previous_selected).then(Instant::now),
                    hovered,
                    flashing,
                    dimmed,
                    name: text_system.shape_line(
                        name.into(),
                        font_size,
                        &[gpui::TextRun {
                            len: entry.name.len(),
                            font: font.clone(),
                            color: name_color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    ),
                    icon,
                });
                state.touch_flags(row, key, selected, hovered, flashing, dimmed);
            }

            (state.collect(start_idx, end_idx), show_checkboxes)
        });

        window.with_optional_element_state(id, |_, _window| ((), Some(state.clone())));
        crate::perf::count_by("grid.surface.rows", slots.len() as u64);
        GridSurfacePrepaint {
            slots,
            show_checkboxes,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        let _perf = crate::perf::span("grid.surface.paint");
        crate::perf::count("grid.surface.paint");
        let pane_id = self.pane_id;
        let app = self.app.clone();
        let colors = self.colors;
        let cols = self.cols;
        let cell_w = self.cell_w;
        let cell_h = self.cell_h;
        let pitch_x = self.pitch_x;
        let pitch_y = self.pitch_y;
        let thumb = self.thumb;

        style.paint(bounds, window, cx, |window, cx| {
            let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
            let icon_area_h = thumb * GRID_ICON_AREA;
            let label_h = GRID_LABEL_H * zoom;

            for slot in &prepaint.slots {
                let col = slot.row % cols;
                let row = slot.row / cols;
                let x = bounds.origin.x + px(col as f32 * pitch_x);
                let y = bounds.origin.y + px(row as f32 * pitch_y);
                let cell = Bounds {
                    origin: point(x, y),
                    size: size(px(cell_w), px(cell_h)),
                };

                let background = if slot.selected {
                    Some(colors.accent)
                } else if slot.flashing {
                    Some(colors.accent.opacity(0.38))
                } else if slot.hovered {
                    Some(colors.hover)
                } else {
                    None
                };
                if let Some(mut color) = background {
                    if slot.dimmed {
                        color = color.opacity(color.a * 0.45);
                    }
                    paint_rounded(window, cell, 6.0 * zoom, color);
                }

                let icon_center = point(
                    x + px(cell_w * 0.5),
                    y + px((icon_area_h.min(cell_h - label_h)) * 0.5),
                );
                if slot.selected && prepaint.show_checkboxes {
                    selection_checkbox::paint_animated(
                        window,
                        cx,
                        icon_center,
                        16.0 * zoom,
                        slot.selected_at,
                    );
                } else {
                    match &slot.icon {
                        FileIcon::PreviewTexture(tex) => {
                            let natural = tex.size(0);
                            let natural_w = i32::from(natural.width).max(1) as f32;
                            let natural_h = i32::from(natural.height).max(1) as f32;
                            let max_h = (thumb * GRID_PREVIEW_MAX).min(icon_area_h);
                            let scale = (thumb / natural_w).min(max_h / natural_h).min(1.0);
                            let w = natural_w * scale;
                            let h = natural_h * scale;
                            let icon_bounds = Bounds {
                                origin: point(
                                    icon_center.x - px(w * 0.5),
                                    icon_center.y - px(h * 0.5),
                                ),
                                size: size(px(w), px(h)),
                            };
                            let _ = window.paint_image(
                                icon_bounds,
                                Corners::default(),
                                tex.clone(),
                                0,
                                false,
                            );
                        }
                        FileIcon::IconTexture(tex) => {
                            let sz = (thumb * GRID_TEXTURE_ICON_MAX).min(icon_area_h);
                            let icon_bounds = Bounds {
                                origin: point(
                                    icon_center.x - px(sz * 0.5),
                                    icon_center.y - px(sz * 0.5),
                                ),
                                size: size(px(sz), px(sz)),
                            };
                            let _ = window.paint_image(
                                icon_bounds,
                                Corners::default(),
                                tex.clone(),
                                0,
                                false,
                            );
                        }
                        FileIcon::Builtin { kind, .. } => {
                            let icon = if matches!(kind, FileKind::Dir) {
                                AtlasIcon::Folder
                            } else {
                                AtlasIcon::File
                            };
                            let sz = thumb * GRID_BUILTIN_ICON;
                            let icon_bounds = Bounds {
                                origin: point(
                                    icon_center.x - px(sz * 0.5),
                                    icon_center.y - px(sz * 0.5),
                                ),
                                size: size(px(sz), px(sz)),
                            };
                            let _ = window.paint_svg(
                                icon_bounds,
                                SharedString::from(icon.path()),
                                None,
                                Default::default(),
                                if slot.selected {
                                    colors.accent_fg
                                } else {
                                    colors.muted
                                },
                                cx,
                            );
                        }
                    }
                }

                let name_bounds = Bounds {
                    origin: point(x + px(4.0 * zoom), y + px(icon_area_h)),
                    size: size(px((cell_w - 8.0 * zoom).max(0.0)), px(label_h)),
                };
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: name_bounds,
                    }),
                    |window| {
                        let _ = slot.name.paint(
                            point(name_bounds.origin.x, name_bounds.origin.y),
                            px(label_h),
                            TextAlign::Center,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            let Some(app_entity) = app.upgrade() else {
                return;
            };
            let move_view = app_entity.clone();
            let move_hitbox = prepaint.hitbox.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.dragging() {
                    return;
                }
                let pos = ev.position;
                move_view.update(cx, |this, cx| {
                    let hover_entry = move_hitbox
                        .is_hovered(window)
                        .then(|| {
                            this.grid_hit
                                .get(&pane_id)
                                .and_then(|model| model.hit_entry(pos))
                                .map(|idx| (pane_id, idx))
                        })
                        .flatten();
                    if this.grid_hover != hover_entry {
                        gpui_component::tooltip::Tooltip::dismiss(window, cx);
                        this.grid_hover = hover_entry;
                        this.notify_pane(pane_id, cx);
                    }
                });
            });

            let down_view = app_entity;
            let down_hitbox = prepaint.hitbox.clone();
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                if !down_hitbox.is_hovered(window)
                    || !down_view
                        .read(cx)
                        .grid_hit
                        .get(&pane_id)
                        .is_some_and(|model| model.bounds.contains(&pos))
                {
                    return;
                }
                down_view.update(cx, |this, cx| {
                    let hit = this
                        .grid_hit
                        .get(&pane_id)
                        .and_then(|model| model.hit_entry(pos));
                    match ev.button {
                        MouseButton::Left => this.details_left_mouse_down(
                            pane_id,
                            hit,
                            pos,
                            ev.modifiers.shift,
                            ev.modifiers.control || ev.modifiers.platform,
                            ev.click_count,
                            cx,
                        ),
                        MouseButton::Right => {
                            this.details_right_mouse_down(pane_id, hit, pos, window, cx)
                        }
                        _ => {}
                    }
                });
                if matches!(ev.button, MouseButton::Left | MouseButton::Right) {
                    cx.stop_propagation();
                }
            });
        });
    }
}

fn grid_shape_key(
    entry_idx: usize,
    name: &str,
    selected: bool,
    dimmed: bool,
    font_size: Pixels,
    color: Hsla,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entry_idx.hash(&mut hasher);
    name.hash(&mut hasher);
    selected.hash(&mut hasher);
    dimmed.hash(&mut hasher);
    f32::from(font_size).to_bits().hash(&mut hasher);
    color.h.to_bits().hash(&mut hasher);
    color.s.to_bits().hash(&mut hasher);
    color.l.to_bits().hash(&mut hasher);
    color.a.to_bits().hash(&mut hasher);
    hasher.finish()
}
