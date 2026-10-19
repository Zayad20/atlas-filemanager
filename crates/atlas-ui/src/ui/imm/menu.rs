//! One IMM leaf for a menu's rows: arithmetic slots, clipped GPU paint and input.
//! Commands stay upstream. The canvas receives immutable ink and opaque row ids.
use crate::ui::icons::AtlasIcon;
use crate::ui::surface::{
    line_width, paint_fill, paint_rounded, paint_theme_icon, shape_text_truncated, Slot,
};
use gpui::ColorExt as _;
use gpui::{
    point, px, size, App, Bounds, Corners, CursorStyle, DispatchPhase, Element, ElementId, Font,
    FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, RenderImage, ShapedLine,
    SharedString, Style, TextAlign, Window,
};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum MenuIcon {
    Theme(AtlasIcon),
    Image(Arc<RenderImage>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuRowKind {
    Item,
    Separator,
    Loading,
}

pub(crate) struct MenuRow {
    pub kind: MenuRowKind,
    pub label: SharedString,
    pub icon: Option<MenuIcon>,
    pub shortcut: Option<SharedString>,
    pub shortcut_editable: bool,
    pub shortcut_recording: bool,
    pub depth: u8,
    pub disabled: bool,
    pub emphasized: bool,
    pub submenu: bool,
    pub expanded: bool,
}

impl MenuRow {
    pub fn height(&self, zoom: f32) -> f32 {
        if self.kind == MenuRowKind::Separator {
            9.0 * zoom
        } else {
            30.0 * zoom
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MenuColors {
    pub foreground: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
}

type RowCallback = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Keyboard navigation reveals a slot using the same content-space geometry.
pub(crate) fn visible_scroll_top(scroll_top: f32, viewport: f32, top: f32, bottom: f32) -> f32 {
    if top < scroll_top {
        top.max(0.0)
    } else if bottom > scroll_top + viewport {
        (bottom - viewport).max(0.0)
    } else {
        scroll_top
    }
}

pub(crate) struct MenuCanvas {
    id: ElementId,
    rows: Arc<Vec<MenuRow>>,
    selected: Option<usize>,
    colors: MenuColors,
    reveal_ms: f32,
    on_hover: RowCallback,
    on_activate: RowCallback,
    on_edit_shortcut: RowCallback,
}

#[derive(Clone, Copy)]
struct RowTarget {
    index: usize,
    slot: Slot,
    shortcut: Option<Slot>,
    disabled: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum MenuHit {
    Activate(usize),
    EditShortcut(usize),
}

fn hit_target(rows: &[RowTarget], x: f32, y: f32) -> Option<MenuHit> {
    let row = rows.iter().find(|row| row.slot.contains(x, y))?;
    if row.disabled {
        return None;
    }
    if row.shortcut.is_some_and(|slot| slot.contains(x, y)) {
        Some(MenuHit::EditShortcut(row.index))
    } else {
        Some(MenuHit::Activate(row.index))
    }
}

struct PreparedRow {
    index: usize,
    slot: Slot,
    label: ShapedLine,
    shortcut: Option<ShapedLine>,
    label_x: f32,
    shortcut_slot: Option<Slot>,
}

pub(crate) struct MenuPrepaint {
    hitbox: Hitbox,
    rows: Vec<PreparedRow>,
    zoom: f32,
}

impl MenuCanvas {
    pub fn new(
        rows: Vec<MenuRow>,
        selected: Option<usize>,
        colors: MenuColors,
        reveal_ms: f32,
        on_hover: RowCallback,
        on_activate: RowCallback,
        on_edit_shortcut: RowCallback,
    ) -> Self {
        Self {
            id: ElementId::Name("ctx-menu-imm-canvas".into()),
            rows: Arc::new(rows),
            selected,
            colors,
            reveal_ms,
            on_hover,
            on_activate,
            on_edit_shortcut,
        }
    }
}

impl IntoElement for MenuCanvas {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MenuCanvas {
    type RequestLayoutState = Style;
    type PrepaintState = MenuPrepaint;
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Style) {
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let mut style = Style::default();
        style.size.width =
            gpui::Length::Definite(gpui::DefiniteLength::Relative(gpui::relative(1.0)));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Absolute(
            gpui::AbsoluteLength::Pixels(px(
                8.0 * zoom + self.rows.iter().map(|r| r.height(zoom)).sum::<f32>()
            )),
        ));
        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> MenuPrepaint {
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let mask = window.content_mask().bounds.intersect(&bounds);
        let hitbox = window.insert_hitbox(mask, HitboxBehavior::Normal);
        let mut rows = Vec::new();
        let mut y = 4.0 * zoom;
        let width = f32::from(bounds.size.width);
        let base_font = window.text_style().font();
        for (index, row) in self.rows.iter().enumerate() {
            let slot = Slot::new(0.0, y, width, row.height(zoom));
            y += slot.h;
            if !slot.to_bounds(bounds.origin).intersect(&mask).is_empty() {
                let selected = self.selected == Some(index);
                let color = if selected {
                    self.colors.accent_foreground
                } else if row.disabled || row.kind == MenuRowKind::Loading {
                    self.colors.muted
                } else {
                    self.colors.foreground
                };
                let mut font: Font = base_font.clone();
                if row.emphasized {
                    font.weight = FontWeight::SEMIBOLD;
                }
                let shortcut = row.shortcut.as_ref().map(|s| {
                    shape_text_truncated(
                        s,
                        &base_font,
                        px(11.0 * zoom),
                        if selected {
                            self.colors.accent_foreground
                        } else {
                            self.colors.muted
                        },
                        width * 0.4,
                        window,
                    )
                });
                let shortcut_w = shortcut
                    .as_ref()
                    .map_or(0.0, |s| line_width(s) + 12.0 * zoom);
                let label_x = if row.kind == MenuRowKind::Loading {
                    8.0 * zoom
                } else {
                    (32.0 + f32::from(row.depth) * 14.0) * zoom
                };
                let right = (8.0 + if row.submenu { 20.0 } else { 0.0 }) * zoom + shortcut_w;
                let shortcut_slot = shortcut.as_ref().map(|_| {
                    Slot::new(
                        width - right,
                        slot.y + (slot.h - 20.0 * zoom) * 0.5,
                        shortcut_w,
                        20.0 * zoom,
                    )
                });
                let label = shape_text_truncated(
                    &row.label,
                    &font,
                    px(14.0 * zoom),
                    color,
                    (width - label_x - right - if shortcut.is_some() { 8.0 * zoom } else { 0.0 })
                        .max(0.0),
                    window,
                );
                rows.push(PreparedRow {
                    index,
                    slot,
                    label,
                    shortcut,
                    label_x,
                    shortcut_slot,
                });
            }
        }
        crate::perf::count_by("ctx_menu.imm.visible_rows", rows.len() as u64);
        MenuPrepaint { hitbox, rows, zoom }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepared: &mut MenuPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let zoom = prepared.zoom;
        let colors = self.colors;
        let selected = self.selected;
        style.paint(bounds, window, cx, |window, cx| {
            for ink in &prepared.rows {
                let row = &self.rows[ink.index];
                let progress = crate::ui::motion::menu_row_progress(self.reveal_ms, ink.index);
                let mut origin = bounds.origin;
                origin.y += px(3.0 * zoom * (1.0 - progress));
                let row_bounds = ink.slot.to_bounds(origin);
                window.with_element_opacity(Some(0.65 + 0.35 * progress), |window| {
                    window.paint_layer(row_bounds, |window| {
                        if row.kind == MenuRowKind::Separator {
                            paint_fill(
                                window,
                                Bounds {
                                    origin: point(
                                        row_bounds.left() + px(8.0 * zoom),
                                        row_bounds.top() + px(4.0 * zoom),
                                    ),
                                    size: size(px((ink.slot.w - 16.0 * zoom).max(0.0)), px(zoom)),
                                },
                                colors.border,
                            );
                            return;
                        }
                        let is_selected =
                            selected == Some(ink.index) && row.kind == MenuRowKind::Item;
                        if is_selected {
                            paint_rounded(window, row_bounds, 5.0 * zoom, colors.accent);
                        }
                        // ShapedLine centers ascent/descent inside the supplied line height.
                        // Its origin is the line box top, not the glyph top.
                        let text_y = row_bounds.top();
                        let _ = ink.label.paint(
                            point(row_bounds.left() + px(ink.label_x), text_y),
                            px(ink.slot.h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        let color = if row.disabled {
                            colors.muted
                        } else if is_selected {
                            colors.accent_foreground
                        } else {
                            colors.foreground
                        };
                        if let Some(icon) = &row.icon {
                            let slot = Slot::new(
                                (8.0 + f32::from(row.depth) * 14.0) * zoom,
                                ink.slot.y + (ink.slot.h - 16.0 * zoom) * 0.5,
                                16.0 * zoom,
                                16.0 * zoom,
                            );
                            match icon {
                                MenuIcon::Theme(icon) => paint_theme_icon(
                                    window,
                                    cx,
                                    origin,
                                    slot,
                                    *icon,
                                    false,
                                    16.0 * zoom,
                                    color,
                                ),
                                MenuIcon::Image(image) => {
                                    let b = slot.to_bounds(origin);
                                    let _ = window.paint_image(
                                        b,
                                        b,
                                        Corners::default(),
                                        image.clone(),
                                        0,
                                        false,
                                    );
                                }
                            }
                        }
                        if let (Some(shortcut), Some(slot)) = (&ink.shortcut, ink.shortcut_slot) {
                            let chip = slot.to_bounds(origin);
                            let hovered = row.shortcut_editable
                                && prepared.hitbox.is_hovered(window)
                                && chip.contains(&window.mouse_position());
                            let border = if is_selected {
                                colors.accent_foreground.opacity(0.4)
                            } else {
                                colors.border
                            };
                            window.paint_quad(gpui::quad(
                                chip,
                                Corners::all(px(4.0 * zoom)),
                                if row.shortcut_recording || hovered {
                                    color.opacity(0.12)
                                } else {
                                    gpui::transparent_black()
                                },
                                gpui::Edges::all(px(zoom)),
                                border,
                                Default::default(),
                            ));
                            let _ = shortcut.paint(
                                point(chip.left() + px(6.0 * zoom), chip.top()),
                                chip.size.height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if row.submenu {
                            paint_theme_icon(
                                window,
                                cx,
                                origin,
                                Slot::new(
                                    ink.slot.w - 20.0 * zoom,
                                    ink.slot.y + (ink.slot.h - 12.0 * zoom) * 0.5,
                                    12.0 * zoom,
                                    12.0 * zoom,
                                ),
                                if row.expanded {
                                    AtlasIcon::CaretDown
                                } else {
                                    AtlasIcon::CaretRight
                                },
                                false,
                                12.0 * zoom,
                                color,
                            );
                        }
                    });
                });
            }
            let slots: Vec<_> = prepared
                .rows
                .iter()
                .filter(|r| self.rows[r.index].kind == MenuRowKind::Item)
                .map(|r| {
                    let offset = 3.0
                        * zoom
                        * (1.0 - crate::ui::motion::menu_row_progress(self.reveal_ms, r.index));
                    let mut slot = r.slot;
                    slot.y += offset;
                    RowTarget {
                        index: r.index,
                        slot,
                        shortcut: r
                            .shortcut_slot
                            .filter(|_| self.rows[r.index].shortcut_editable)
                            .map(|mut slot| {
                                slot.y += offset;
                                slot
                            }),
                        disabled: self.rows[r.index].disabled,
                    }
                })
                .collect();
            let hitbox = prepared.hitbox.clone();
            if hitbox.is_hovered(window) {
                let position = window.mouse_position() - bounds.origin;
                if hit_target(&slots, f32::from(position.x), f32::from(position.y)).is_some() {
                    window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
                }
            }
            let hover = self.on_hover.clone();
            let move_slots = slots.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) || ev.dragging() {
                    return;
                }
                let local_x = f32::from(ev.position.x - bounds.left());
                let local_y = f32::from(ev.position.y - bounds.top());
                if let Some(row) = move_slots
                    .iter()
                    .find(|row| row.slot.contains(local_x, local_y))
                {
                    hover(row.index, window, cx);
                }
            });
            let hitbox = prepared.hitbox.clone();
            let activate = self.on_activate.clone();
            let edit_shortcut = self.on_edit_shortcut.clone();
            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                if ev.button == MouseButton::Left {
                    let x = f32::from(ev.position.x - bounds.left());
                    let y = f32::from(ev.position.y - bounds.top());
                    match hit_target(&slots, x, y) {
                        Some(MenuHit::EditShortcut(index)) => edit_shortcut(index, window, cx),
                        Some(MenuHit::Activate(index)) => activate(index, window, cx),
                        None => {}
                    }
                }
                // Consume even disabled rows/separators: they are still inside the menu.
                cx.stop_propagation();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_click_does_not_execute_row_and_native_rows_stay_actions() {
        let mut row = RowTarget {
            index: 7,
            slot: Slot::new(0.0, 34.0, 300.0, 30.0),
            shortcut: Some(Slot::new(244.0, 39.0, 48.0, 20.0)),
            disabled: false,
        };
        assert_eq!(
            hit_target(&[row], 250.0, 45.0),
            Some(MenuHit::EditShortcut(7))
        );
        assert_eq!(hit_target(&[row], 100.0, 45.0), Some(MenuHit::Activate(7)));
        assert_eq!(hit_target(&[row], 250.0, 65.0), None);
        row.disabled = true;
        assert_eq!(hit_target(&[row], 250.0, 45.0), None);
        row.disabled = false;
        row.shortcut = None;
        assert_eq!(hit_target(&[row], 250.0, 45.0), Some(MenuHit::Activate(7)));
    }
    #[test]
    fn keyboard_scroll_uses_slots_instead_of_layout_child_indices() {
        assert_eq!(visible_scroll_top(30.0, 90.0, 39.0, 69.0), 30.0);
        assert_eq!(visible_scroll_top(30.0, 90.0, 4.0, 34.0), 4.0);
        assert_eq!(visible_scroll_top(0.0, 90.0, 94.0, 124.0), 34.0);
    }
    #[test]
    fn rows_have_distinct_scaled_slots_including_separators() {
        let item = MenuRow {
            kind: MenuRowKind::Item,
            label: "item".into(),
            icon: None,
            shortcut: None,
            shortcut_editable: false,
            shortcut_recording: false,
            depth: 0,
            disabled: false,
            emphasized: false,
            submenu: false,
            expanded: false,
        };
        assert_eq!(item.height(1.5), 45.0);
        let separator = MenuRow {
            kind: MenuRowKind::Separator,
            ..item
        };
        assert_eq!(separator.height(1.5), 13.5);
    }
}
