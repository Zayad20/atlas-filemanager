//! Immediate-mode Element for Unified Fuzzy Palettes (`PaletteImmElement`).
//!
//! Architectural principles (immediate-mode):
//! 1. Upstream / Downstream flow: Canonical state stays in `App` & `FuzzyOverlayState`; downstream
//!    derives layout, effects, and paint cache every frame.
//! 2. Single-cycle O(1) arithmetic layout with `RectCut` (`crate::ui::imm::cut::RectCut`) eliminating
//!    multi-pass flexbox equation solving for modal, search bar, rows, and footer.
//! 3. Procedural immediate rendering: hardware-clipped scissor rendering with `paint_fill`,
//!    `paint_rounded`, `paint_theme_icon`, shaped text, and `scrollbar_slot` projection.
//! 4. Full interaction coverage:
//!    - Mouse hover highlighting per row with pointing hand cursor.
//!    - Direct click execution: dispatch `Action` and dismiss overlay.
//!    - Comprehensive keyboard model: Up/Down selection, Enter confirm, Tab drill/expand, Esc close.
//!    - Click on background scrim closes overlay.
//!    - Tooltips on action buttons and paths.

use crate::app::App;
use crate::input::action::Action;
use crate::ui::imm::cut::RectCut;
use crate::ui::imm::menu_panel::scrollbar_slot;
use crate::ui::imm::motion::RetainedSpring;
use crate::ui::file_icons::FileIconRef;
use crate::ui::icons::AtlasIcon;
use crate::ui::palette::render::{
    PaletteColors, PANEL_COMPACT_ITEM_HEIGHT, PANEL_FOOTER_HEIGHT, PANEL_HEADER_HEIGHT,
    PANEL_ITEM_HEIGHT, PANEL_ITEM_WITH_DESC_HEIGHT, PANEL_LIST_PADDING, PANEL_MAX_VIEWPORT_RATIO,
    PANEL_MIN_VISIBLE_ROWS, PANEL_SEARCH_HEIGHT, PANEL_TOP_VIEWPORT_RATIO, PANEL_WIDTH,
    SCROLLBAR_GUTTER, SECTION_INDENT,
};
use crate::ui::palette::{
    FooterHint, FooterSpec, ItemButton, PaletteItem, PaletteKind, PaletteSection, ShortcutHint,
};
use crate::ui::surface::{
    line_width, paint_fill, paint_rounded, paint_theme_icon, shape_text, shape_text_truncated, Slot,
};
use atlas_model::file_entry::FileKind;
use gpui::ColorExt as _;
use gpui::{
    point, px, size, AnyElement, App as GpuiApp, AvailableSpace, Bounds, BoxShadow, ContentMask,
    Corners, CursorStyle, DispatchPhase, Element, ElementId, Font, FontWeight, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Refineable,
    ScrollWheelEvent, SharedString, Style, StyleRefinement, Styled, StyledText, TextAlign,
    WeakEntity, Window,
};
use gpui_component::input::InputState;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One shell timeline per mounted PaletteHost. Search batches only retarget height.
struct PaletteMotion {
    height: RetainedSpring,
    from: f32,
    target: f32,
    changed_at: Instant,
    viewport: [f32; 2],
}

impl PaletteMotion {
    fn new(height: f32, viewport: [f32; 2], now: Instant) -> Self {
        Self {
            height: RetainedSpring::new(height, now),
            from: 0.0,
            target: 1.0,
            changed_at: now,
            viewport,
        }
    }

    fn openness(&self, now: Instant) -> (f32, bool) {
        let ms = if self.target > 0.0 {
            super::render::PALETTE_OPEN_MS
        } else {
            super::render::PALETTE_CLOSE_MS
        };
        let t = now.saturating_duration_since(self.changed_at).as_secs_f32()
            / Duration::from_millis(ms).as_secs_f32();
        (
            self.from + (self.target - self.from) * crate::ui::motion::ease_out_quart(t),
            t < 1.0 && self.from != self.target,
        )
    }

    fn sample(
        &mut self,
        height: f32,
        viewport: [f32; 2],
        closing: bool,
        instant: bool,
        now: Instant,
    ) -> (f32, f32, bool) {
        let (current, _) = self.openness(now);
        let goal = if closing { 0.0 } else { 1.0 };
        if self.target != goal {
            self.from = current;
            self.target = goal;
            self.changed_at = now;
        }
        let (openness, opening) = if instant {
            self.from = goal;
            self.target = goal;
            self.changed_at = now;
            (goal, false)
        } else {
            self.openness(now)
        };
        let (height, resizing) =
            self.height
                .sample(height, instant || viewport != self.viewport, now);
        self.viewport = viewport;
        (height, openness.clamp(0.0, 1.0), opening || resizing)
    }
}

// ── Layout Geometry Structs ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModalGeometry {
    pub modal_slot: Slot,
    pub search_slot: Slot,
    pub list_viewport_slot: Slot,
    pub footer_slot: Option<Slot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SearchBarGeometry {
    pub search_icon_slot: Slot,
    pub prefix_slot: Option<Slot>,
    pub input_slot: Slot,
    pub clear_btn_slot: Option<Slot>,
    pub counter_slot: Option<Slot>,
    pub buttons_slot: Option<Slot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeaderGeometry {
    pub chevron_slot: Slot,
    pub icon_slot: Option<Slot>,
    pub title_slot: Slot,
    pub count_slot: Option<Slot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowGeometry {
    pub icon_slot: Option<Slot>,
    pub title_slot: Slot,
    pub desc_slot: Option<Slot>,
    pub buttons_slot: Option<Slot>,
    pub shortcuts_slot: Option<Slot>,
}

/// Computes modal container bounds and its 3 primary vertical regions using `RectCut`.
pub(crate) fn compute_modal_geometry(
    viewport_w: f32,
    viewport_h: f32,
    content_h: f32,
    zoom: f32,
    has_footer: bool,
) -> ModalGeometry {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    let modal_w = (PANEL_WIDTH * zoom).min((viewport_w - 24.0 * zoom).max(10.0));
    let modal_x = (viewport_w - modal_w) * 0.5;
    let modal_y = viewport_h * PANEL_TOP_VIEWPORT_RATIO;

    let chrome_h =
        (PANEL_SEARCH_HEIGHT + if has_footer { PANEL_FOOTER_HEIGHT } else { 0.0 }) * zoom;
    let max_modal_h = (viewport_h * PANEL_MAX_VIEWPORT_RATIO).max(chrome_h);
    let min_modal_h = chrome_h + PANEL_ITEM_HEIGHT * zoom * PANEL_MIN_VISIBLE_ROWS;
    let modal_h = (chrome_h + content_h).clamp(min_modal_h.min(max_modal_h), max_modal_h);

    let mut cut = RectCut::new(modal_x, modal_y, modal_w, modal_h);
    let search_slot = cut.cut_top(PANEL_SEARCH_HEIGHT * zoom);
    let footer_slot = if has_footer {
        Some(cut.cut_bottom(PANEL_FOOTER_HEIGHT * zoom))
    } else {
        None
    };
    let list_viewport_slot = cut.remaining();

    ModalGeometry {
        modal_slot: Slot::new(modal_x, modal_y, modal_w, modal_h),
        search_slot,
        list_viewport_slot,
        footer_slot,
    }
}

/// Slices the search bar into magnifier, prefix, input, clear button, match counter, and buttons.
pub(crate) fn compute_search_bar_layout(
    search_slot: Slot,
    zoom: f32,
    has_query: bool,
    counter_w: f32,
    prefix_w: f32,
    buttons_w: f32,
) -> SearchBarGeometry {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    let mut cut = RectCut::new(search_slot.x, search_slot.y, search_slot.w, search_slot.h);
    cut.pad(12.0 * zoom, 0.0);

    let search_icon_slot = cut.cut_left(16.0 * zoom);
    let _ = cut.cut_left(8.0 * zoom);

    let prefix_slot = if prefix_w > 0.0 {
        let s = cut.cut_left(prefix_w);
        let _ = cut.cut_left(6.0 * zoom);
        Some(s)
    } else {
        None
    };

    let buttons_slot = if buttons_w > 0.0 {
        let s = cut.cut_right(buttons_w);
        let _ = cut.cut_right(6.0 * zoom);
        Some(s)
    } else {
        None
    };

    let clear_btn_slot = if has_query {
        let s = cut.cut_right(18.0 * zoom);
        let _ = cut.cut_right(6.0 * zoom);
        Some(s)
    } else {
        None
    };

    let counter_slot = if counter_w > 0.0 {
        let s = cut.cut_right(counter_w);
        let _ = cut.cut_right(8.0 * zoom);
        Some(s)
    } else {
        None
    };

    let input_slot = cut.remaining();

    SearchBarGeometry {
        search_icon_slot,
        prefix_slot,
        input_slot,
        clear_btn_slot,
        counter_slot,
        buttons_slot,
    }
}

/// Slices a section header into chevron, optional icon, title, and item count badge.
pub(crate) fn compute_header_layout(
    header_slot: Slot,
    zoom: f32,
    has_icon: bool,
    count_w: f32,
) -> HeaderGeometry {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    let mut cut = RectCut::new(header_slot.x, header_slot.y, header_slot.w, header_slot.h);
    cut.pad(8.0 * zoom, 0.0);

    let chevron_slot = cut.cut_left(14.0 * zoom);
    let _ = cut.cut_left(4.0 * zoom);

    let icon_slot = if has_icon {
        let s = cut.cut_left(14.0 * zoom);
        let _ = cut.cut_left(4.0 * zoom);
        Some(s)
    } else {
        None
    };

    let count_slot = if count_w > 0.0 {
        let s = cut.cut_right(count_w);
        let _ = cut.cut_right(6.0 * zoom);
        Some(s)
    } else {
        None
    };

    let title_slot = cut.remaining();

    HeaderGeometry {
        chevron_slot,
        icon_slot,
        title_slot,
        count_slot,
    }
}

/// Slices an item row into icon, title, stacked/inline description, action buttons, and shortcut badges.
pub(crate) fn compute_row_layout(
    row_slot: Slot,
    zoom: f32,
    indent: bool,
    has_icon: bool,
    shortcuts_w: f32,
    buttons_w: f32,
    is_stacked_desc: bool,
) -> RowGeometry {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    let mut cut = RectCut::new(row_slot.x, row_slot.y, row_slot.w, row_slot.h);
    let pad_left = if indent {
        SECTION_INDENT * zoom
    } else {
        8.0 * zoom
    };
    let _ = cut.cut_left(pad_left);
    let _ = cut.cut_right(8.0 * zoom);

    let icon_slot = if has_icon {
        let s = cut.cut_left(16.0 * zoom);
        let _ = cut.cut_left(8.0 * zoom);
        Some(s)
    } else {
        None
    };

    let shortcuts_slot = if shortcuts_w > 0.0 {
        let s = cut.cut_right(shortcuts_w);
        let _ = cut.cut_right(6.0 * zoom);
        Some(s)
    } else {
        None
    };

    let buttons_slot = if buttons_w > 0.0 {
        let s = cut.cut_right(buttons_w);
        let _ = cut.cut_right(4.0 * zoom);
        Some(s)
    } else {
        None
    };

    if is_stacked_desc {
        let mut text_cut = RectCut::new(cut.x, cut.y, cut.w, cut.h);
        text_cut.pad(0.0, 4.0 * zoom);
        let title_slot = text_cut.cut_top(20.0 * zoom);
        let desc_slot = Some(text_cut.remaining());
        RowGeometry {
            icon_slot,
            title_slot,
            desc_slot,
            buttons_slot,
            shortcuts_slot,
        }
    } else {
        RowGeometry {
            icon_slot,
            title_slot: cut.remaining(),
            desc_slot: None,
            buttons_slot,
            shortcuts_slot,
        }
    }
}

// ── Flattened Key Rows and Navigation Model ──────────────────────────────────

#[derive(Clone, Debug)]
pub(crate) enum PaletteKeyRow {
    Header {
        section_id: &'static str,
    },
    Item {
        section_idx: usize,
        item_idx: usize,
        action: Action,
        expand: Option<SharedString>,
    },
}

/// Computes the new selection index for keyboard arrow keys, wrapping/clamping safely.
pub(crate) fn compute_keyboard_selection(
    current_idx: usize,
    key: &str,
    row_count: usize,
    key_rows: &[PaletteKeyRow],
) -> usize {
    if row_count == 0 {
        return 0;
    }
    match key {
        "up" => current_idx.saturating_sub(1),
        "down" => (current_idx + 1).min(row_count.saturating_sub(1)),
        "home" => {
            if matches!(key_rows.first(), Some(PaletteKeyRow::Header { .. })) {
                key_rows
                    .iter()
                    .position(|r| matches!(r, PaletteKeyRow::Item { .. }))
                    .unwrap_or(0)
            } else {
                0
            }
        }
        "end" => row_count.saturating_sub(1),
        _ => current_idx.min(row_count.saturating_sub(1)),
    }
}

/// Snaps initial selection past a leading section header directly onto the best match item.
pub(crate) fn snap_past_leading_header(key_rows: &[PaletteKeyRow], selected_idx: usize) -> usize {
    if selected_idx == 0 && matches!(key_rows.first(), Some(PaletteKeyRow::Header { .. })) {
        key_rows
            .iter()
            .position(|r| matches!(r, PaletteKeyRow::Item { .. }))
            .unwrap_or(0)
    } else {
        selected_idx.min(key_rows.len().saturating_sub(1))
    }
}

// ── Interactive Target Models ────────────────────────────────────────────────

#[derive(Clone)]
enum PaletteHitTarget {
    Scrim,
    ClearButton,
    Header {
        section_id: &'static str,
    },
    Item {
        visible_idx: usize,
        action: Action,
    },
    ItemButton {
        action: Action,
        closes_palette: bool,
        tooltip: SharedString,
    },
    ScrollbarThumb,
}

struct PreparedRow {
    visible_idx: usize,
    slot: Slot,
    title_slot: Slot,
    desc_slot: Option<Slot>,
    icon: Option<AtlasIcon>,
    file_icon_tex: Option<Arc<gpui::RenderImage>>,
    title_line: gpui::ShapedLine,
    desc_line: Option<gpui::ShapedLine>,
    shortcuts: Vec<(Slot, gpui::ShapedLine)>,
    buttons: Vec<(Slot, AtlasIcon, SharedString, Action, bool, bool)>, // (slot, icon, tip, action, closes, active)
    is_selected: bool,
    is_header: bool,
    header_section_id: Option<&'static str>,
    header_collapsed: bool,
    header_count_text: Option<String>,
}

pub(crate) struct PaletteImmPrepaint {
    selection_trail: Option<Slot>,
    hitbox: Hitbox,
    modal_slot: Slot,
    search_slot: Slot,
    input_slot: Slot,
    clear_btn_slot: Option<Slot>,
    counter_slot: Option<Slot>,
    counter_line: Option<gpui::ShapedLine>,
    list_viewport_slot: Slot,
    footer_slot: Option<Slot>,
    thumb_slot: Option<Slot>,
    rows: Vec<PreparedRow>,
    all_hits: Vec<(Slot, PaletteHitTarget)>,
    content_h: f32,
    zoom: f32,
    openness: f32,
}

// ── PaletteImmElement ─────────────────────────────────────────────────────────

pub(crate) struct PaletteImmElement {
    id: ElementId,
    app: WeakEntity<App>,
    kind: PaletteKind,
    spec_id: &'static str,
    query: String,
    input_entity: Option<gpui::Entity<InputState>>,
    input_element: Option<AnyElement>,
    search_prefix: Option<AnyElement>,
    search_buttons: Vec<AnyElement>,
    sections: Vec<PaletteSection>,
    collapsed_sections: HashSet<String>,
    empty_text: SharedString,
    footer: FooterSpec,
    colors: PaletteColors,
    selected_idx: usize,
    expanded: bool,
    scroll_top: f32,
    zoom: f32,
    windows_theme: bool,
    closing: bool,
    element_style: StyleRefinement,
}

impl PaletteImmElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app: WeakEntity<App>,
        kind: PaletteKind,
        spec_id: &'static str,
        query: String,
        input_entity: Option<gpui::Entity<InputState>>,
        input_element: Option<AnyElement>,
        search_prefix: Option<AnyElement>,
        search_buttons: Vec<AnyElement>,
        sections: Vec<PaletteSection>,
        collapsed_sections: HashSet<String>,
        empty_text: SharedString,
        footer: FooterSpec,
        colors: PaletteColors,
        selected_idx: usize,
        expanded: bool,
        scroll_top: f32,
        zoom: f32,
        windows_theme: bool,
        closing: bool,
    ) -> Self {
        Self {
            id: ElementId::Name(format!("palette-imm-{}", spec_id).into()),
            app,
            kind,
            spec_id,
            query,
            input_entity,
            input_element,
            search_prefix,
            search_buttons,
            sections,
            collapsed_sections,
            empty_text,
            footer,
            colors,
            selected_idx,
            expanded,
            scroll_top,
            zoom: if zoom.is_finite() && zoom > 0.0 {
                zoom
            } else {
                1.0
            },
            windows_theme,
            closing,
            element_style: StyleRefinement::default(),
        }
    }

    /// Build the flattened list of visible rows (headers included, collapsed items excluded).
    pub(crate) fn flatten_visible_rows(&self) -> Vec<PaletteKeyRow> {
        let mut key_rows = Vec::new();
        for (sec_idx, section) in self.sections.iter().enumerate() {
            let has_header = section.title.is_some();
            if has_header {
                key_rows.push(PaletteKeyRow::Header {
                    section_id: section.id,
                });
            }
            if has_header && self.collapsed_sections.contains(section.id) {
                continue;
            }
            for (item_idx, item) in section.items.iter().enumerate() {
                key_rows.push(PaletteKeyRow::Item {
                    section_idx: sec_idx,
                    item_idx,
                    action: item.action.clone(),
                    expand: item.expand.clone(),
                });
            }
        }
        key_rows
    }
}

impl IntoElement for PaletteImmElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for PaletteImmElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.element_style
    }
}

impl Element for PaletteImmElement {
    type RequestLayoutState = Style;
    type PrepaintState = PaletteImmPrepaint;

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
        cx: &mut GpuiApp,
    ) -> (LayoutId, Self::RequestLayoutState) {
        crate::perf::count("palette_imm.request_layout");
        let mut style = Style::default();
        style.refine(&self.element_style);
        // Scrim occupies the entire viewport
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Style,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        crate::perf::count("palette_imm.prepaint");
        let zoom = self.zoom;
        let viewport_w = f32::from(bounds.size.width);
        let viewport_h = f32::from(bounds.size.height);

        let key_rows = self.flatten_visible_rows();
        let row_count = key_rows.len();
        let selected_idx = snap_past_leading_header(&key_rows, self.selected_idx);

        // Precompute total content height
        let mut content_h = PANEL_LIST_PADDING * zoom;
        let mut visible_i = 0usize;
        for section in &self.sections {
            let has_header = section.title.is_some();
            if has_header {
                content_h += PANEL_HEADER_HEIGHT * zoom;
                visible_i += 1;
                if self.collapsed_sections.contains(section.id) {
                    continue;
                }
            }
            for item in &section.items {
                let shows_desc = !item.inline_description
                    && item.description.is_some()
                    && (self.expanded || visible_i == selected_idx);
                content_h += if self.kind == PaletteKind::Goto && item.inline_description {
                    PANEL_COMPACT_ITEM_HEIGHT * zoom
                } else if shows_desc {
                    PANEL_ITEM_WITH_DESC_HEIGHT * zoom
                } else {
                    PANEL_ITEM_HEIGHT * zoom
                };
                visible_i += 1;
            }
        }
        if key_rows.is_empty() {
            content_h += PANEL_ITEM_HEIGHT * zoom * PANEL_MIN_VISIBLE_ROWS;
        }

        let has_footer = !matches!(self.footer, FooterSpec::None);
        let mut modal_geom =
            compute_modal_geometry(viewport_w, viewport_h, content_h, zoom, has_footer);
        let motion = window.use_keyed_state(
            SharedString::from(format!("palette-shell-motion-{}", self.spec_id)),
            cx,
            |_, _| {
                PaletteMotion::new(
                    modal_geom.modal_slot.h,
                    [viewport_w, viewport_h],
                    Instant::now(),
                )
            },
        );
        let instant = cx.reduce_motion();
        let (height, openness, active) = motion.update(cx, |state, _| {
            state.sample(
                modal_geom.modal_slot.h,
                [viewport_w, viewport_h],
                self.closing,
                instant,
                Instant::now(),
            )
        });
        let chrome_h = modal_geom.search_slot.h + modal_geom.footer_slot.map_or(0.0, |slot| slot.h);
        modal_geom.modal_slot.h = height.clamp(
            chrome_h,
            (viewport_h * PANEL_MAX_VIEWPORT_RATIO).max(chrome_h),
        );
        // The search field stays pinned as results grow beneath it. Only opening
        // contributes a tiny displacement, and every hit/input uses these slots.
        let offset = if instant {
            0.0
        } else {
            6.0 * zoom * (1.0 - openness)
        };
        modal_geom.modal_slot.y += offset;
        modal_geom.search_slot.y += offset;
        modal_geom.list_viewport_slot.y += offset;
        modal_geom.list_viewport_slot.h = (modal_geom.modal_slot.h - chrome_h).max(0.0);
        if let Some(footer) = &mut modal_geom.footer_slot {
            footer.y = modal_geom.modal_slot.y + modal_geom.modal_slot.h - footer.h;
        }
        self.scroll_top = self
            .scroll_top
            .min((content_h - modal_geom.list_viewport_slot.h).max(0.0));
        if active {
            window.request_animation_frame();
        }

        let match_count = key_rows
            .iter()
            .filter(|r| matches!(r, PaletteKeyRow::Item { .. }))
            .count();
        let counter_str = if !self.query.is_empty() {
            Some(format!("{match_count}"))
        } else {
            None
        };

        let base_font = window.text_style().font();
        let counter_line = counter_str
            .as_ref()
            .map(|s| shape_text(s, &base_font, px(11.0 * zoom), self.colors.muted, window));
        let counter_w = counter_line
            .as_ref()
            .map_or(0.0, |l| line_width(l) + 8.0 * zoom);

        let prefix_w = if self.search_prefix.is_some() {
            70.0 * zoom
        } else {
            0.0
        };
        let buttons_w = if !self.search_buttons.is_empty() {
            60.0 * zoom
        } else {
            0.0
        };

        let search_geom = compute_search_bar_layout(
            modal_geom.search_slot,
            zoom,
            !self.query.is_empty(),
            counter_w,
            prefix_w,
            buttons_w,
        );

        // Prepaint search input child element if wired
        if let Some(input_el) = &mut self.input_element {
            input_el.prepaint_as_root(
                point(
                    bounds.origin.x + px(search_geom.input_slot.x),
                    bounds.origin.y + px(search_geom.input_slot.y),
                ),
                size(
                    AvailableSpace::Definite(px(search_geom.input_slot.w)),
                    AvailableSpace::Definite(px(search_geom.input_slot.h)),
                ),
                window,
                cx,
            );
        }

        // Scrollbar slot projection
        let thumb_slot = scrollbar_slot(
            modal_geom.list_viewport_slot,
            content_h,
            self.scroll_top,
            zoom,
        );
        let gutter = if thumb_slot.is_some() {
            SCROLLBAR_GUTTER * zoom
        } else {
            0.0
        };

        // Hit registration and prepared rows
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::BlockMouse);
        let mut all_hits = Vec::new();
        // Scrim click closes overlay
        all_hits.push((
            Slot::new(0.0, 0.0, viewport_w, viewport_h),
            PaletteHitTarget::Scrim,
        ));

        if let Some(clear_slot) = search_geom.clear_btn_slot {
            all_hits.push((clear_slot, PaletteHitTarget::ClearButton));
        }

        if let Some(thumb) = thumb_slot {
            all_hits.push((thumb, PaletteHitTarget::ScrollbarThumb));
        }

        let mut prepared_rows = Vec::new();
        let list_slot = modal_geom.list_viewport_slot;
        let mut current_y = list_slot.y + 4.0 * zoom;
        let row_w = (list_slot.w - gutter).max(10.0);

        let mut gi = 0usize;
        for section in &self.sections {
            let has_header = section.title.is_some();
            let is_collapsed = has_header && self.collapsed_sections.contains(section.id);

            if let Some(title) = &section.title {
                let visible_idx = gi;
                gi += 1;
                let header_h = PANEL_HEADER_HEIGHT * zoom;
                let header_slot = Slot::new(list_slot.x, current_y, row_w, header_h);
                current_y += header_h + 2.0 * zoom;

                let is_selected = visible_idx == selected_idx;
                let text_color = if is_selected {
                    self.colors.fg
                } else {
                    self.colors.muted
                };

                let count_str = format!("{}", section.items.len());
                let count_line = shape_text(
                    &count_str,
                    &base_font,
                    px(10.0 * zoom),
                    self.colors.muted,
                    window,
                );
                let count_w = line_width(&count_line) + 8.0 * zoom;

                let h_geom =
                    compute_header_layout(header_slot, zoom, section.icon.is_some(), count_w);
                let title_line = shape_text_truncated(
                    title,
                    &base_font,
                    px(12.0 * zoom),
                    text_color,
                    h_geom.title_slot.w,
                    window,
                );

                all_hits.push((
                    header_slot,
                    PaletteHitTarget::Header {
                        section_id: section.id,
                    },
                ));

                prepared_rows.push(PreparedRow {
                    visible_idx,
                    slot: header_slot,
                    title_slot: h_geom.title_slot,
                    desc_slot: None,
                    icon: section.icon,
                    file_icon_tex: None,
                    title_line,
                    desc_line: None,
                    shortcuts: Vec::new(),
                    buttons: Vec::new(),
                    is_selected,
                    is_header: true,
                    header_section_id: Some(section.id),
                    header_collapsed: is_collapsed,
                    header_count_text: Some(count_str),
                });
            }

            if is_collapsed {
                continue;
            }

            for item in &section.items {
                let visible_idx = gi;
                gi += 1;

                let is_selected = visible_idx == selected_idx;
                let shows_desc = !item.inline_description
                    && item.description.is_some()
                    && (self.expanded || is_selected);
                let row_h = if self.kind == PaletteKind::Goto && item.inline_description {
                    PANEL_COMPACT_ITEM_HEIGHT * zoom
                } else if shows_desc {
                    PANEL_ITEM_WITH_DESC_HEIGHT * zoom
                } else {
                    PANEL_ITEM_HEIGHT * zoom
                };

                let row_slot = Slot::new(list_slot.x, current_y, row_w, row_h);
                current_y += row_h + 2.0 * zoom;

                // Prepare shortcut badge lines
                let mut shortcuts = Vec::new();
                let mut shortcuts_total_w = 0.0;
                for sc in &item.shortcuts {
                    let badge_text = match sc {
                        ShortcutHint::Keys(k) => k.to_string(),
                        ShortcutHint::Label(l) => l.to_string(),
                        ShortcutHint::ForAction(_) => "".to_string(),
                        ShortcutHint::CommandShortcut { key, .. } => key
                            .as_ref()
                            .map(|k| k.to_string())
                            .unwrap_or_else(|| "".to_string()),
                    };
                    if !badge_text.is_empty() {
                        let sc_color = if is_selected {
                            self.colors.accent_fg
                        } else {
                            self.colors.muted
                        };
                        let line =
                            shape_text(&badge_text, &base_font, px(11.0 * zoom), sc_color, window);
                        let w = line_width(&line) + 10.0 * zoom;
                        shortcuts_total_w += w + 4.0 * zoom;
                        shortcuts.push((line, w));
                    }
                }

                let buttons_total_w = item.buttons.len() as f32 * 22.0 * zoom;
                let r_geom = compute_row_layout(
                    row_slot,
                    zoom,
                    has_header,
                    item.icon.is_some() || item.file_icon.is_some(),
                    shortcuts_total_w,
                    buttons_total_w,
                    shows_desc,
                );

                // Title and query match highlighting
                let title_color = if is_selected {
                    self.colors.accent_fg
                } else {
                    self.colors.fg
                };
                let match_color = if is_selected {
                    self.colors.accent_fg
                } else {
                    self.colors.accent
                };

                let title_line = if let Some(ranges) = &item.highlight {
                    if !ranges.is_empty() {
                        let mut style = window.text_style();
                        style.color = title_color;
                        let highlights: Vec<(Range<usize>, HighlightStyle)> = ranges
                            .iter()
                            .map(|r| {
                                (
                                    r.clone(),
                                    HighlightStyle {
                                        color: Some(match_color),
                                        font_weight: Some(FontWeight::SEMIBOLD),
                                        ..HighlightStyle::default()
                                    },
                                )
                            })
                            .collect();
                        let st = StyledText::new(item.title.clone())
                            .with_default_highlights(&style, highlights);
                        let run = gpui::TextRun {
                            len: item.title.len(),
                            font: base_font.clone(),
                            color: title_color,
                            underline: None,
                            strikethrough: None,
                            background_color: None,
                            letter_spacing: None,
                        };
                        window.text_system().shape_line(
                            item.title.clone(),
                            px(13.0 * zoom),
                            &[run],
                            None,
                        )
                    } else {
                        shape_text_truncated(
                            &item.title,
                            &base_font,
                            px(13.0 * zoom),
                            title_color,
                            r_geom.title_slot.w,
                            window,
                        )
                    }
                } else {
                    shape_text_truncated(
                        &item.title,
                        &base_font,
                        px(13.0 * zoom),
                        title_color,
                        r_geom.title_slot.w,
                        window,
                    )
                };

                let desc_line = if shows_desc {
                    item.description.as_ref().map(|desc| {
                        shape_text_truncated(
                            desc,
                            &base_font,
                            px(11.0 * zoom),
                            if is_selected {
                                self.colors.accent_fg.opacity(0.85)
                            } else {
                                self.colors.muted
                            },
                            r_geom.desc_slot.map_or(r_geom.title_slot.w, |s| s.w),
                            window,
                        )
                    })
                } else {
                    None
                };

                // Position shortcut badges from right to left
                let mut positioned_shortcuts = Vec::new();
                if let Some(sc_slot) = r_geom.shortcuts_slot {
                    let mut cur_x = sc_slot.x + sc_slot.w;
                    for (line, w) in shortcuts {
                        cur_x -= w;
                        let badge_slot = Slot::new(
                            cur_x,
                            row_slot.y + (row_slot.h - 18.0 * zoom) * 0.5,
                            w,
                            18.0 * zoom,
                        );
                        positioned_shortcuts.push((badge_slot, line));
                        cur_x -= 4.0 * zoom;
                    }
                }

                // Position action buttons
                let mut positioned_buttons = Vec::new();
                if let Some(btn_slot) = r_geom.buttons_slot {
                    let mut cur_x = btn_slot.x;
                    for btn in &item.buttons {
                        let b_slot = Slot::new(
                            cur_x,
                            row_slot.y + (row_slot.h - 18.0 * zoom) * 0.5,
                            18.0 * zoom,
                            18.0 * zoom,
                        );
                        positioned_buttons.push((
                            b_slot,
                            btn.icon,
                            btn.tooltip.clone(),
                            btn.action.clone(),
                            btn.closes_palette,
                            btn.active,
                        ));
                        all_hits.push((
                            b_slot,
                            PaletteHitTarget::ItemButton {
                                action: btn.action.clone(),
                                closes_palette: btn.closes_palette,
                                tooltip: btn.tooltip.clone(),
                            },
                        ));
                        cur_x += 20.0 * zoom;
                    }
                }

                // File icon thumbnail texture lookup
                let file_icon_tex =
                    if let (Some(app), Some(file_ref)) = (self.app.upgrade(), &item.file_icon) {
                        app.read(cx)
                            .file_icon(file_ref)
                            .texture_for_size(16.0 * zoom)
                    } else {
                        None
                    };

                all_hits.push((
                    row_slot,
                    PaletteHitTarget::Item {
                        visible_idx,
                        action: item.action.clone(),
                    },
                ));

                prepared_rows.push(PreparedRow {
                    visible_idx,
                    slot: row_slot,
                    title_slot: r_geom.title_slot,
                    desc_slot: r_geom.desc_slot,
                    icon: item.icon,
                    file_icon_tex,
                    title_line,
                    desc_line,
                    shortcuts: positioned_shortcuts,
                    buttons: positioned_buttons,
                    is_selected,
                    is_header: false,
                    header_section_id: None,
                    header_collapsed: false,
                    header_count_text: None,
                });
            }
        }

        let target = prepared_rows
            .iter()
            .find(|row| row.is_selected)
            .map(|row| row.slot);
        let indicator = window.use_keyed_state(
            SharedString::from(format!("palette-selection-motion-{}", self.spec_id)),
            cx,
            |_, _| {
                (
                    self.query.clone(),
                    self.scroll_top,
                    target.map(|slot| RetainedSpring::new(slot.y, Instant::now())),
                )
            },
        );
        let mouse = window.mouse_position();
        let pointer_selected = target.is_some_and(|mut slot| {
            slot.y -= self.scroll_top;
            slot.to_bounds(bounds.origin).contains(&mouse)
        });
        let (selection_trail, moving) = indicator.update(cx, |state, _| {
            let reset =
                state.0 != self.query || state.1 != self.scroll_top || instant || pointer_selected;
            state.0 = self.query.clone();
            state.1 = self.scroll_top;
            let Some(mut slot) = target else {
                state.2 = None;
                return (None, false);
            };
            let spring = state
                .2
                .get_or_insert_with(|| RetainedSpring::new(slot.y, Instant::now()));
            let (y, active) = spring.sample(slot.y, reset, Instant::now());
            slot.y = y - self.scroll_top;
            (active.then_some(slot), active)
        });
        if moving {
            window.request_animation_frame();
        }

        PaletteImmPrepaint {
            selection_trail,
            hitbox,
            modal_slot: modal_geom.modal_slot,
            search_slot: modal_geom.search_slot,
            input_slot: search_geom.input_slot,
            clear_btn_slot: search_geom.clear_btn_slot,
            counter_slot: search_geom.counter_slot,
            counter_line,
            list_viewport_slot: modal_geom.list_viewport_slot,
            footer_slot: modal_geom.footer_slot,
            thumb_slot,
            rows: prepared_rows,
            all_hits,
            content_h,
            zoom,
            openness,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Style,
        ink: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        crate::perf::count("palette_imm.paint");
        let zoom = ink.zoom;
        let origin = bounds.origin;
        let colors = self.colors;
        let mouse_pos = window.mouse_position();
        let is_hovered = ink.hitbox.is_hovered(window);

        // The full-window hitbox catches backdrop clicks without recoloring the
        // workspace, matching overlay_scrim for the retained fuzzy palettes.
        window.with_element_opacity(Some(ink.openness), |window| {
            // 2. Paint modal container
            let modal_bounds = ink.modal_slot.to_bounds(origin);
            let corners = Corners::all(px(10.0 * zoom));

            // Drop shadow
            window.paint_drop_shadows(
                modal_bounds,
                corners,
                &[
                    BoxShadow::new(px(0.0), px(12.0 * zoom), gpui::black().opacity(0.36))
                        .blur_radius(px(32.0 * zoom)),
                ],
            );

            // Background quad & border
            window.paint_quad(gpui::quad(
                modal_bounds,
                corners,
                colors.border.opacity(0.08),
                gpui::Edges::all(px(zoom)),
                colors.border,
                Default::default(),
            ));
            paint_fill(window, modal_bounds, colors.muted_bg);

            // 3. Paint Search Bar
            let search_bounds = ink.search_slot.to_bounds(origin);
            paint_fill(
                window,
                Bounds {
                    origin: point(modal_bounds.left(), search_bounds.bottom() - px(zoom)),
                    size: size(modal_bounds.size.width, px(zoom)),
                },
                colors.border,
            );

            // Search magnifier icon
            paint_theme_icon(
                window,
                cx,
                origin,
                Slot::new(
                    ink.search_slot.x + 12.0 * zoom,
                    ink.search_slot.y + (ink.search_slot.h - 16.0 * zoom) * 0.5,
                    16.0 * zoom,
                    16.0 * zoom,
                ),
                AtlasIcon::Search,
                self.windows_theme,
                16.0 * zoom,
                colors.muted,
            );

            // Clear button (X) when query is active
            if let Some(clear_slot) = ink.clear_btn_slot {
                let clear_bounds = clear_slot.to_bounds(origin);
                let is_clear_hover = is_hovered && clear_bounds.contains(&mouse_pos);
                let x_color = if is_clear_hover {
                    colors.fg
                } else {
                    colors.muted
                };
                if is_clear_hover {
                    paint_rounded(window, clear_bounds, 4.0 * zoom, colors.border);
                }
                paint_theme_icon(
                    window,
                    cx,
                    origin,
                    clear_slot,
                    AtlasIcon::X,
                    self.windows_theme,
                    12.0 * zoom,
                    x_color,
                );
            }

            // Query match counter
            if let (Some(counter_slot), Some(line)) = (ink.counter_slot, &ink.counter_line) {
                let _ = line.paint(
                    point(
                        origin.x + px(counter_slot.x),
                        origin.y + px(counter_slot.y + (counter_slot.h - 12.0 * zoom) * 0.5),
                    ),
                    px(counter_slot.h),
                    TextAlign::Right,
                    None,
                    window,
                    cx,
                );
            }

            // Search input element child paint
            if let Some(input_el) = &mut self.input_element {
                input_el.paint(window, cx);
            }

            // 4. Paint List Results within Viewport Scissor Mask
            let viewport_bounds = ink.list_viewport_slot.to_bounds(origin);
            let mut hovered_btn_tip: Option<(Slot, SharedString)> = None;

            window.with_content_mask(
                Some(ContentMask {
                    bounds: viewport_bounds,
                    fade_out: Default::default(),
                }),
                |window| {
                    let scroll_y = px(self.scroll_top);
                    // A moving outline carries keyboard continuity while the
                    // selected fill/text change immediately and keep contrast.
                    if let Some(slot) = ink.selection_trail {
                        window.paint_quad(
                            gpui::outline(
                                slot.to_bounds(origin),
                                colors.accent.opacity(0.65),
                                Default::default(),
                            )
                            .corner_radii(Corners::all(px(5.0 * zoom))),
                        );
                    }
                    for row in &ink.rows {
                        let mut row_slot = row.slot;
                        row_slot.y -= self.scroll_top;
                        let row_bounds = row_slot.to_bounds(origin);

                        // Scissor cull rows outside viewport bounds
                        if row_bounds.bottom() < viewport_bounds.top()
                            || row_bounds.top() > viewport_bounds.bottom()
                        {
                            continue;
                        }

                        let row_hovered = is_hovered && row_bounds.contains(&mouse_pos);

                        if row.is_header {
                            // Section header row
                            if row.is_selected {
                                paint_rounded(window, row_bounds, 4.0 * zoom, colors.border);
                            } else if row_hovered {
                                paint_rounded(
                                    window,
                                    row_bounds,
                                    4.0 * zoom,
                                    colors.border.opacity(0.6),
                                );
                            }

                            // Chevron
                            let chevron_icon = if row.header_collapsed {
                                AtlasIcon::CaretRight
                            } else {
                                AtlasIcon::CaretDown
                            };
                            paint_theme_icon(
                                window,
                                cx,
                                point(origin.x, origin.y - scroll_y),
                                Slot::new(
                                    row.slot.x + 8.0 * zoom,
                                    row.slot.y + (row.slot.h - 12.0 * zoom) * 0.5,
                                    12.0 * zoom,
                                    12.0 * zoom,
                                ),
                                chevron_icon,
                                self.windows_theme,
                                12.0 * zoom,
                                if row.is_selected {
                                    colors.fg
                                } else {
                                    colors.muted
                                },
                            );

                            // Header Title
                            let _ = row.title_line.paint(
                                point(
                                    origin.x + px(row.title_slot.x),
                                    origin.y + px(row.title_slot.y) - scroll_y,
                                ),
                                px(row.title_slot.h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );

                            // Count badge
                            if let Some(count_str) = &row.header_count_text {
                                let badge_font = window.text_style().font();
                                let badge_line = shape_text(
                                    count_str,
                                    &badge_font,
                                    px(10.0 * zoom),
                                    colors.muted,
                                    window,
                                );
                                let _ = badge_line.paint(
                                    point(
                                        origin.x + px(row.slot.x + row.slot.w - 24.0 * zoom),
                                        origin.y + px(row.slot.y + 4.0 * zoom) - scroll_y,
                                    ),
                                    px(row.slot.h),
                                    TextAlign::Right,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        } else {
                            // Item Row
                            if row.is_selected {
                                paint_rounded(window, row_bounds, 5.0 * zoom, colors.accent);
                            } else if row_hovered {
                                paint_rounded(
                                    window,
                                    row_bounds,
                                    5.0 * zoom,
                                    colors.border.opacity(0.5),
                                );
                            }

                            // Icon (AtlasIcon or FileIcon texture)
                            if let Some(tex) = &row.file_icon_tex {
                                let icon_bounds = Bounds {
                                    origin: point(
                                        origin.x + px(row.slot.x + 8.0 * zoom),
                                        origin.y
                                            + px(row.slot.y + (row.slot.h - 16.0 * zoom) * 0.5)
                                            - scroll_y,
                                    ),
                                    size: size(px(16.0 * zoom), px(16.0 * zoom)),
                                };
                                let _ = window.paint_image(
                                    icon_bounds,
                                    icon_bounds,
                                    Corners::default(),
                                    tex.clone(),
                                    0,
                                    false,
                                );
                            } else if let Some(ic) = row.icon {
                                paint_theme_icon(
                                    window,
                                    cx,
                                    point(origin.x, origin.y - scroll_y),
                                    Slot::new(
                                        row.slot.x + 8.0 * zoom,
                                        row.slot.y + (row.slot.h - 16.0 * zoom) * 0.5,
                                        16.0 * zoom,
                                        16.0 * zoom,
                                    ),
                                    ic,
                                    self.windows_theme,
                                    16.0 * zoom,
                                    if row.is_selected {
                                        colors.accent_fg
                                    } else {
                                        colors.muted
                                    },
                                );
                            }

                            // Title
                            let _ = row.title_line.paint(
                                point(
                                    origin.x + px(row.title_slot.x),
                                    origin.y + px(row.title_slot.y) - scroll_y,
                                ),
                                px(row.title_slot.h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );

                            // Description
                            if let (Some(desc_slot), Some(desc_line)) =
                                (row.desc_slot, &row.desc_line)
                            {
                                let _ = desc_line.paint(
                                    point(
                                        origin.x + px(desc_slot.x),
                                        origin.y + px(desc_slot.y) - scroll_y,
                                    ),
                                    px(desc_slot.h),
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }

                            // Action Buttons
                            for (btn_slot, btn_icon, tip, _action, _closes, active) in &row.buttons
                            {
                                let mut b_slot = *btn_slot;
                                b_slot.y -= self.scroll_top;
                                let b_bounds = b_slot.to_bounds(origin);
                                let btn_hovered = is_hovered && b_bounds.contains(&mouse_pos);
                                if btn_hovered {
                                    hovered_btn_tip = Some((b_slot, tip.clone()));
                                    paint_rounded(window, b_bounds, 3.0 * zoom, colors.border);
                                }
                                let btn_color = if row.is_selected {
                                    colors.accent_fg
                                } else if *active {
                                    colors.accent
                                } else {
                                    colors.muted
                                };
                                paint_theme_icon(
                                    window,
                                    cx,
                                    origin,
                                    b_slot,
                                    *btn_icon,
                                    self.windows_theme,
                                    14.0 * zoom,
                                    btn_color,
                                );
                            }

                            // Shortcut badges (Kbd styled)
                            for (badge_slot, badge_line) in &row.shortcuts {
                                let mut sc_slot = *badge_slot;
                                sc_slot.y -= self.scroll_top;
                                let sc_bounds = sc_slot.to_bounds(origin);

                                let (bg, border, text_color) = if row.is_selected {
                                    (
                                        gpui::transparent_black(),
                                        colors.accent_fg.opacity(0.4),
                                        colors.accent_fg,
                                    )
                                } else {
                                    (colors.muted_bg, colors.border, colors.muted)
                                };

                                window.paint_quad(gpui::quad(
                                    sc_bounds,
                                    Corners::all(px(4.0 * zoom)),
                                    bg,
                                    gpui::Edges::all(px(zoom)),
                                    border,
                                    Default::default(),
                                ));

                                let _ = badge_line.paint(
                                    point(
                                        sc_bounds.left() + px(5.0 * zoom),
                                        sc_bounds.top() + px(1.0 * zoom),
                                    ),
                                    sc_bounds.size.height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
                },
            );

            // 5. Paint Scrollbar Thumb
            if let Some(thumb) = ink.thumb_slot {
                let thumb_bounds = thumb.to_bounds(origin);
                paint_rounded(window, thumb_bounds, 3.0 * zoom, colors.muted.opacity(0.55));
            }

            // 6. Paint Footer Hint Bar
            if let Some(footer_slot) = ink.footer_slot {
                let footer_bounds = footer_slot.to_bounds(origin);
                paint_fill(
                    window,
                    Bounds {
                        origin: footer_bounds.origin,
                        size: size(footer_bounds.size.width, px(zoom)),
                    },
                    colors.border,
                );

                let footer_font = window.text_style().font();
                let count_text = rust_i18n::t!(
                    "palette_results_count",
                    count = ink.rows.iter().filter(|r| !r.is_header).count()
                );
                let count_line = shape_text(
                    count_text.as_ref(),
                    &footer_font,
                    px(11.0 * zoom),
                    colors.muted,
                    window,
                );
                let _ = count_line.paint(
                    point(
                        footer_bounds.left() + px(12.0 * zoom),
                        footer_bounds.top() + px(8.0 * zoom),
                    ),
                    footer_bounds.size.height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );

                // Contextual Key Hints on the right: ↑ ↓, Enter, Tab, Esc
                let hints: [(&str, std::borrow::Cow<'static, str>); 4] = [
                    ("↑ ↓", rust_i18n::t!("palette_footer_navigate")),
                    ("Enter", rust_i18n::t!("palette_footer_use")),
                    ("Tab", rust_i18n::t!("palette_footer_expand")),
                    ("Esc", rust_i18n::t!("palette_footer_close")),
                ];
                let mut right_x = footer_bounds.right() - px(12.0 * zoom);
                for (key, label) in hints.iter().rev() {
                    let lbl_line = shape_text(
                        label.as_ref(),
                        &footer_font,
                        px(11.0 * zoom),
                        colors.muted,
                        window,
                    );
                    let lbl_w = line_width(&lbl_line);
                    right_x -= px(lbl_w);
                    let _ = lbl_line.paint(
                        point(right_x, footer_bounds.top() + px(8.0 * zoom)),
                        footer_bounds.size.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    right_x -= px(6.0 * zoom);

                    let kbd_line =
                        shape_text(*key, &footer_font, px(10.0 * zoom), colors.muted, window);
                    let kbd_w = line_width(&kbd_line) + 8.0 * zoom;
                    right_x -= px(kbd_w);

                    let kbd_bounds = Bounds {
                        origin: point(right_x, footer_bounds.top() + px(6.0 * zoom)),
                        size: size(px(kbd_w), px(18.0 * zoom)),
                    };
                    window.paint_quad(gpui::quad(
                        kbd_bounds,
                        Corners::all(px(3.0 * zoom)),
                        colors.muted_bg,
                        gpui::Edges::all(px(zoom)),
                        colors.border,
                        Default::default(),
                    ));
                    let _ = kbd_line.paint(
                        point(
                            kbd_bounds.left() + px(4.0 * zoom),
                            kbd_bounds.top() + px(2.0 * zoom),
                        ),
                        kbd_bounds.size.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    right_x -= px(12.0 * zoom);
                }
            }

            // 7. Paint Action Button Tooltip Overlay
            if let Some((b_slot, tip)) = hovered_btn_tip {
                let tip_font = window.text_style().font();
                let tip_line = shape_text(&tip, &tip_font, px(11.0 * zoom), colors.fg, window);
                let tip_w = line_width(&tip_line) + 12.0 * zoom;
                let tip_h = 22.0 * zoom;
                let tip_bounds = Bounds {
                    origin: point(
                        origin.x + px(b_slot.x + (b_slot.w - tip_w) * 0.5),
                        origin.y + px(b_slot.y - tip_h - 4.0 * zoom),
                    ),
                    size: size(px(tip_w), px(tip_h)),
                };
                window.paint_drop_shadows(
                    tip_bounds,
                    Corners::all(px(4.0 * zoom)),
                    &[
                        BoxShadow::new(px(0.0), px(4.0 * zoom), gpui::black().opacity(0.3))
                            .blur_radius(px(8.0 * zoom)),
                    ],
                );
                window.paint_quad(gpui::quad(
                    tip_bounds,
                    Corners::all(px(4.0 * zoom)),
                    colors.border.opacity(0.95),
                    gpui::Edges::all(px(zoom)),
                    colors.border,
                    Default::default(),
                ));
                let _ = tip_line.paint(
                    point(
                        tip_bounds.left() + px(6.0 * zoom),
                        tip_bounds.top() + px(4.0 * zoom),
                    ),
                    tip_bounds.size.height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // 8. Cursor styling: PointingHand on clickable rows, buttons, headers
        });

        if self.closing {
            return;
        }

        if is_hovered {
            let lx = f32::from(mouse_pos.x - origin.x);
            let ly = f32::from(mouse_pos.y - origin.y);
            let is_interactive = ink.all_hits.iter().any(|(s, target)| {
                s.contains(lx, ly) && !matches!(target, PaletteHitTarget::Scrim)
            });
            if is_interactive {
                window.set_cursor_style(CursorStyle::PointingHand, &ink.hitbox);
            }
        }

        // 9. Interactive Mouse Events
        let app_handle = self.app.clone();
        let all_hits = Arc::new(ink.all_hits.clone());
        let kind = self.kind;
        let input_ent = self.input_entity.clone();
        let hitbox = ink.hitbox.clone();
        let list_viewport = ink.list_viewport_slot;
        let content_h = ink.content_h;
        let max_scroll = (content_h - list_viewport.h).max(0.0);

        // Scroll Wheel
        window.on_mouse_event::<ScrollWheelEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            if let Some(app) = app_handle.upgrade() {
                app.update(cx, |this, cx| {
                    let state = kind.state(this);
                    state.scroll_to_selected = false;
                    let cur_offset = this.palette_scroll.offset();
                    let delta_y = f32::from(ev.delta.pixel_delta(px(30.0 * zoom)).y);
                    let new_y = (-f32::from(cur_offset.y) - delta_y).clamp(0.0, max_scroll);
                    this.palette_scroll
                        .set_offset(gpui::point(cur_offset.x, gpui::px(-new_y)));
                    cx.notify();
                });
            }
            cx.stop_propagation();
        });

        // Mouse Down (clicks on rows, buttons, clear, headers, scrim)
        let app_handle = self.app.clone();
        let hitbox = ink.hitbox.clone();
        let input_ent = self.input_entity.clone();
        let hits = all_hits.clone();
        window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || !hitbox.is_hovered(window)
                || ev.button != MouseButton::Left
            {
                return;
            }
            let lx = f32::from(ev.position.x - origin.x);
            let ly = f32::from(ev.position.y - origin.y);

            // Find topmost target
            let mut matched_target = None;
            for (slot, target) in hits.iter().rev() {
                if slot.contains(lx, ly) {
                    matched_target = Some(target.clone());
                    break;
                }
            }

            if let Some(target) = matched_target {
                match target {
                    PaletteHitTarget::Scrim => {
                        if let Some(app) = app_handle.upgrade() {
                            app.update(cx, |this, cx| {
                                this.process_actions(vec![kind.close_action()]);
                                cx.notify();
                            });
                        }
                    }
                    PaletteHitTarget::ClearButton => {
                        if let (Some(app), Some(inp)) = (app_handle.upgrade(), &input_ent) {
                            inp.update(cx, |state, cx| state.set_value("", window, cx));
                            app.update(cx, |this, cx| {
                                let state = kind.state(this);
                                state.query.clear();
                                state.selected_idx = 0;
                                cx.notify();
                            });
                        }
                    }
                    PaletteHitTarget::Header { section_id } => {
                        if let Some(app) = app_handle.upgrade() {
                            app.update(cx, |this, cx| {
                                this.toggle_palette_section(kind, section_id);
                                cx.notify();
                            });
                        }
                    }
                    PaletteHitTarget::Item {
                        visible_idx: _,
                        action,
                    } => {
                        if let Some(app) = app_handle.upgrade() {
                            app.update(cx, |this, cx| {
                                this.process_actions(vec![action, kind.close_action()]);
                                this.flush_dirty_panes(cx);
                            });
                        }
                    }
                    PaletteHitTarget::ItemButton {
                        action,
                        closes_palette,
                        tooltip: _,
                    } => {
                        if let Some(app) = app_handle.upgrade() {
                            app.update(cx, |this, cx| {
                                let mut actions = vec![action];
                                if closes_palette {
                                    actions.push(kind.close_action());
                                }
                                this.process_actions(actions);
                                this.flush_dirty_panes(cx);
                            });
                        }
                    }
                    PaletteHitTarget::ScrollbarThumb => {
                        // Handled by scroll dragging
                    }
                }
                cx.stop_propagation();
            }
        });

        // Mouse Move for row hover highlighting
        let app_handle = self.app.clone();
        let hitbox = ink.hitbox.clone();
        let hits = all_hits.clone();
        window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) || ev.dragging() {
                return;
            }
            let lx = f32::from(ev.position.x - origin.x);
            let ly = f32::from(ev.position.y - origin.y);

            for (slot, target) in hits.iter().rev() {
                if slot.contains(lx, ly) {
                    if let PaletteHitTarget::Item { visible_idx, .. } = target {
                        let idx = *visible_idx;
                        if let Some(app) = app_handle.upgrade() {
                            app.update(cx, |this, cx| {
                                let state = kind.state(this);
                                if state.selected_idx != idx {
                                    state.selected_idx = idx;
                                    cx.notify();
                                }
                            });
                        }
                        break;
                    }
                }
            }
        });

        // 10. Full Keyboard Navigation
        let app_handle = self.app.clone();
        let key_rows = self.flatten_visible_rows();
        let close_action = self.kind.close_action();
        let input_for_keys = self.input_entity.clone();
        window.on_key_event::<KeyDownEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let key = ev.keystroke.key.as_str();
            let Some(app) = app_handle.upgrade() else {
                return;
            };

            let row_count = key_rows.len();
            match key {
                "up" | "down" | "home" | "end" => {
                    app.update(cx, |this, cx| {
                        let state = kind.state(this);
                        let next = compute_keyboard_selection(
                            state.selected_idx,
                            key,
                            row_count,
                            &key_rows,
                        );
                        state.selected_idx = next;
                        state.scroll_to_selected = true;
                        this.palette_scroll.scroll_to_item(next);
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
                "enter" => {
                    app.update(cx, |this, cx| {
                        let state = kind.state(this);
                        let idx = state.selected_idx.min(row_count.saturating_sub(1));
                        match key_rows.get(idx) {
                            Some(PaletteKeyRow::Item { action, .. }) => {
                                let act = action.clone();
                                this.process_actions(vec![act, close_action.clone()]);
                                this.flush_dirty_panes(cx);
                            }
                            Some(PaletteKeyRow::Header { section_id }) => {
                                this.toggle_palette_section(kind, section_id);
                                cx.notify();
                            }
                            None => {}
                        }
                    });
                    cx.stop_propagation();
                }
                "tab" => {
                    app.update(cx, |this, cx| {
                        let state = kind.state(this);
                        let idx = state.selected_idx.min(row_count.saturating_sub(1));
                        match key_rows.get(idx) {
                            Some(PaletteKeyRow::Header { section_id }) => {
                                this.toggle_palette_section(kind, section_id);
                                cx.notify();
                            }
                            Some(PaletteKeyRow::Item {
                                expand: Some(text), ..
                            }) => {
                                let expand_text = text.clone();
                                if let Some(inp) = &input_for_keys {
                                    inp.update(cx, |s, cx| s.set_value(expand_text, window, cx));
                                }
                                cx.notify();
                            }
                            _ => {
                                state.expanded = !state.expanded;
                                cx.notify();
                            }
                        }
                    });
                    cx.stop_propagation();
                }
                "escape" => {
                    let has_query = matches!(kind, PaletteKind::Goto)
                        && input_for_keys
                            .as_ref()
                            .is_some_and(|inp| !inp.read(cx).value().trim().is_empty());
                    if has_query {
                        if let Some(inp) = &input_for_keys {
                            inp.update(cx, |state, cx| state.set_value("", window, cx));
                        }
                    } else {
                        app.update(cx, |this, cx| {
                            this.process_actions(vec![close_action.clone()]);
                            cx.notify();
                        });
                    }
                    cx.stop_propagation();
                }
                _ => {}
            }
        });
    }
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_motion_query_updates_do_not_restart_opening() {
        let now = Instant::now();
        let viewport = [1200.0, 800.0];
        let mut motion = PaletteMotion::new(200.0, viewport, now);
        let time = now + Duration::from_millis(50);
        let (_, open_before, _) = motion.sample(200.0, viewport, false, false, time);
        let (height, open_after, active) = motion.sample(400.0, viewport, false, false, time);
        assert_eq!(height, 200.0);
        assert_eq!(open_before, open_after);
        assert!(active);
        // Continuous frames let the resize settle; a single two-second gap
        // deliberately freezes the spring on resume under the post-idle policy.
        for frame in 1..=120 {
            motion.sample(400.0, viewport, false, false, time + Duration::from_millis(frame * 16));
        }
        let (height, open, active) =
            motion.sample(400.0, viewport, false, false, now + Duration::from_secs(2));
        assert_eq!((height, open, active), (400.0, 1.0, false));
    }

    #[test]
    fn palette_motion_closing_mid_open_is_continuous() {
        let now = Instant::now();
        let viewport = [1200.0, 800.0];
        let mut motion = PaletteMotion::new(200.0, viewport, now);
        let time = now + Duration::from_millis(40);
        let (_, before, _) = motion.sample(200.0, viewport, false, false, time);
        let (_, after, active) = motion.sample(200.0, viewport, true, false, time);
        assert_eq!(before, after);
        assert!(active);
        let (_, closing, _) = motion.sample(
            200.0,
            viewport,
            true,
            false,
            time + Duration::from_millis(30),
        );
        assert!(closing < before);
        assert_eq!(
            motion.sample(200.0, viewport, true, false, now + Duration::from_secs(2)),
            (200.0, 0.0, false)
        );
    }

    #[test]
    fn palette_motion_disabled_stays_settled_when_motion_is_restored() {
        let now = Instant::now();
        let viewport = [1200.0, 800.0];
        let mut motion = PaletteMotion::new(200.0, viewport, now);
        assert_eq!(
            motion.sample(300.0, viewport, false, true, now),
            (300.0, 1.0, false)
        );
        assert_eq!(
            motion.sample(300.0, viewport, false, false, now),
            (300.0, 1.0, false)
        );
        assert_eq!(
            motion.sample(300.0, viewport, true, true, now),
            (300.0, 0.0, false)
        );
        assert_eq!(
            motion.sample(300.0, viewport, true, false, now),
            (300.0, 0.0, false)
        );
    }

    #[test]
    fn test_modal_geometry() {
        for zoom in [0.75, 1.0, 1.5, 2.0] {
            let viewport_w = 1920.0;
            let viewport_h = 1080.0;
            let content_h = 400.0 * zoom;

            let geom = compute_modal_geometry(viewport_w, viewport_h, content_h, zoom, true);

            // Modal container width is 620px * zoom, clamped to viewport width
            let expected_w = (PANEL_WIDTH * zoom).min(viewport_w - 24.0 * zoom);
            assert!((geom.modal_slot.w - expected_w).abs() < 0.001);

            // Modal is centered horizontally
            let expected_x = (viewport_w - geom.modal_slot.w) * 0.5;
            assert!((geom.modal_slot.x - expected_x).abs() < 0.001);

            // Top offset is 20% viewport height
            let expected_y = viewport_h * PANEL_TOP_VIEWPORT_RATIO;
            assert!((geom.modal_slot.y - expected_y).abs() < 0.001);

            // Height is bounded by 60% viewport height
            assert!(geom.modal_slot.h <= viewport_h * PANEL_MAX_VIEWPORT_RATIO + 0.001);

            // Vertical partition satisfies: search_slot.h + list_viewport_slot.h + footer_slot.h == modal_slot.h
            let footer_h = geom.footer_slot.map_or(0.0, |f| f.h);
            let sum_h = geom.search_slot.h + geom.list_viewport_slot.h + footer_h;
            assert!((sum_h - geom.modal_slot.h).abs() < 0.001);
        }
    }

    #[test]
    fn test_search_bar_layout() {
        let zoom = 1.0;
        let search_slot = Slot::new(100.0, 50.0, 620.0, 45.0);
        let geom = compute_search_bar_layout(search_slot, zoom, true, 30.0, 0.0, 0.0);

        // Magnifier on left
        assert!(geom.search_icon_slot.x >= search_slot.x + 12.0);
        assert_eq!(geom.search_icon_slot.w, 16.0);

        // Clear button (X) exists when has_query is true
        assert!(geom.clear_btn_slot.is_some());
        let clear_slot = geom.clear_btn_slot.unwrap();
        assert_eq!(clear_slot.w, 18.0);

        // Counter slot exists
        assert!(geom.counter_slot.is_some());

        // Input slot fills the remaining space between magnifier and counter/clear
        assert!(geom.input_slot.x > geom.search_icon_slot.x + geom.search_icon_slot.w);
        assert!(geom.input_slot.x + geom.input_slot.w <= geom.counter_slot.unwrap().x);
    }

    #[test]
    fn test_row_slicing() {
        let zoom = 1.0;
        // Standard row: 34px
        let row_slot = Slot::new(0.0, 0.0, 600.0, 34.0);
        let geom = compute_row_layout(row_slot, zoom, false, true, 60.0, 40.0, false);

        assert!(geom.icon_slot.is_some());
        assert_eq!(geom.icon_slot.unwrap().w, 16.0);
        assert!(geom.shortcuts_slot.is_some());
        assert_eq!(geom.shortcuts_slot.unwrap().w, 60.0);
        assert!(geom.buttons_slot.is_some());
        assert_eq!(geom.buttons_slot.unwrap().w, 40.0);
        assert!(geom.desc_slot.is_none());

        // Stacked description row: 52px
        let desc_row = Slot::new(0.0, 0.0, 600.0, 52.0);
        let desc_geom = compute_row_layout(desc_row, zoom, true, true, 0.0, 0.0, true);
        assert!(desc_geom.desc_slot.is_some());
        assert!(desc_geom.title_slot.h < desc_row.h);
    }

    #[test]
    fn test_keyboard_selection_projection() {
        let key_rows = vec![
            PaletteKeyRow::Header { section_id: "sec1" },
            PaletteKeyRow::Item {
                section_idx: 0,
                item_idx: 0,
                action: Action::ClosePalette,
                expand: None,
            },
            PaletteKeyRow::Item {
                section_idx: 0,
                item_idx: 1,
                action: Action::ClosePalette,
                expand: None,
            },
        ];

        // Snapping past leading header to first item
        let snapped = snap_past_leading_header(&key_rows, 0);
        assert_eq!(snapped, 1);

        // Down arrow advances
        assert_eq!(
            compute_keyboard_selection(1, "down", key_rows.len(), &key_rows),
            2
        );
        // Down clamps at end
        assert_eq!(
            compute_keyboard_selection(2, "down", key_rows.len(), &key_rows),
            2
        );

        // Up arrow walks backward to header
        assert_eq!(
            compute_keyboard_selection(1, "up", key_rows.len(), &key_rows),
            0
        );
        // Up clamps at 0
        assert_eq!(
            compute_keyboard_selection(0, "up", key_rows.len(), &key_rows),
            0
        );
    }
}
