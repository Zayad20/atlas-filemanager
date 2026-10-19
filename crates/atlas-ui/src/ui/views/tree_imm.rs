//! Immediate-mode Hierarchical Tree View Element for Atlas (`TreeImmElement`).
//!
//! Architectural principles (immediate-mode, O(1) rect-slicing):
//! 1. Stateless procedural generation: implements `gpui::Element`, `IntoElement`, `Styled`.
//! 2. Single-cycle O(1) layout: uses `RectCut` (from `crate::ui::imm::cut::RectCut`) without Taffy/CSS flexbox for row internals.
//! 3. UTF-8 binary search text elision: uses `shape_text_truncated` from `crate::ui::surface::shape_text_truncated`
//!    for filename and metadata, ensuring multi-byte code points are never split.
//! 4. Per-cell GPU hardware clipping: wraps text/cells with `window.paint_layer(bounds, ...)` to ensure zero text overflow.
//! 5. Drop targets & drag rehydration:
//!    - For every directory row visible, registers `DragHoverTarget` with `DragHoverId::Folder(pane_id, entry.path.clone())`.
//!    - On prepaint, purges stale targets for this pane with `app.pending_drag_hover_targets.retain(...)`.
//!    - Visual hover/blink feedback when `app.drag_hover.blink_id == Some(DragHoverId::Folder(pane_id, path))`.
//! 6. Interactive hitboxes & hover effects:
//!    - Expand/collapse chevron hitbox (`AtlasIcon::CaretDown` / `AtlasIcon::CaretRight`) toggles `Action::ToggleTreeExpand(path)`.
//!    - Indentation guides: vertical 1px guide lines at `x + 8.0` for each depth level `0..depth`.
//!    - Row hover background tint, selection highlight with accent color, keyboard cursor border.
//!    - Mouse click (single click selects, shift-click range, ctrl-click toggle, double click activates file/folder).
//!    - Right click opens context menu (`open_ctx_menu`).
//!    - Automatic hover tooltip on entries (`entry_hover_tooltip`).
//! 7. Responsive 3-phase degradation:
//!    - Metadata columns (Size 70px, Date 130px) elide or drop right-to-left if row width < 300px, leaving priority to the filename.

use crate::app::App;
use crate::input::action::Action;
use crate::ui::imm::cut::RectCut;
use crate::ui::imm::motion::{presentation_epoch, DisclosureMotion, PresenceMotion, RowMotion};
use crate::ui::drag_hover::{
    DragHoverCommand, DragHoverDropCommand, DragHoverId, DragHoverKind, DragHoverTarget,
};
use crate::ui::glass::backdrop_tint;
use crate::ui::icons::{is_windows_theme, AtlasIcon};
use crate::ui::root::ctrl_scroll;
use crate::ui::selection_checkbox;
use crate::ui::surface::{
    line_width, paint_fill, paint_rounded, paint_theme_icon, shape_text_truncated, text_origin_y,
    Slot,
};
use atlas_model::file_entry::{FileEntry, FileKind};
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    div, point, px, size, transparent_black, AnyElement, App as GpuiApp, Bounds, Context, Corners,
    CursorStyle, DispatchPhase, Edges, Element, ElementId, Font, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Refineable, ShapedLine, SharedString, Style, StyleRefinement, Styled,
    TextAlign, WeakEntity, Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{v_flex, ActiveTheme};
use std::path::PathBuf;
use std::sync::Arc;

pub const TREE_ROW_H: f32 = 24.0;
pub const INDENT_STEP: f32 = 18.0;
pub const GUIDE_OFFSET_X: f32 = 8.0;
pub const CHEVRON_W: f32 = 18.0;
pub const CHEVRON_ICON_SZ: f32 = 12.0;
pub const ICON_W: f32 = 18.0;
pub const ICON_SZ: f32 = 14.0;
pub const SIZE_COL_W: f32 = 70.0;
pub const DATE_COL_W: f32 = 130.0;
pub const ROW_PAD_X: f32 = 8.0;
pub const MIN_FILENAME_W: f32 = 50.0;
pub const PHASE1_MIN_WIDTH: f32 = 300.0;
pub const PHASE2_MIN_WIDTH: f32 = 200.0;

/// 3-Phase Responsive Degradation for metadata columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeDegradationPhase {
    /// Full width (>= 300px): both Date (130px) and Size (70px) columns preserved.
    Full,
    /// Intermediate (row < 300px): Date (130px) dropped, Size (70px) retained.
    NoDate,
    /// Minimal (row < 200px or narrow remaining space): Date and Size both dropped, 100% priority to Filename.
    FilenameOnly,
}

/// Arithmetic slot layout for a single hierarchical tree row.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRowLayout {
    pub row_slot: Slot,
    pub indent_guide_xs: Vec<f32>,
    pub chevron_slot: Option<Slot>,
    pub icon_slot: Slot,
    pub name_slot: Slot,
    pub size_slot: Option<Slot>,
    pub date_slot: Option<Slot>,
    pub phase: TreeDegradationPhase,
}

/// Single-cycle O(1) arithmetic layout calculation using `RectCut`.
pub fn compute_tree_row_layout(
    row_x: f32,
    row_y: f32,
    row_w: f32,
    row_h: f32,
    depth: usize,
    is_dir: bool,
    zoom: f32,
) -> TreeRowLayout {
    let mut slicer = RectCut::new(row_x, row_y, row_w, row_h);
    let pad_x = ROW_PAD_X * zoom;
    let _ = slicer.cut_left(pad_x);
    let _ = slicer.cut_right(pad_x);

    let indent_step = INDENT_STEP * zoom;
    let mut indent_guide_xs = Vec::with_capacity(depth);
    for _ in 0..depth {
        let slot = slicer.cut_left(indent_step);
        indent_guide_xs.push(slot.x + GUIDE_OFFSET_X * zoom);
    }

    let chevron_w = CHEVRON_W * zoom;
    let ch_slot = slicer.cut_left(chevron_w);
    let chevron_slot = if is_dir { Some(ch_slot) } else { None };

    let icon_w = ICON_W * zoom;
    let icon_slot = slicer.cut_left(icon_w);

    let size_w = SIZE_COL_W * zoom;
    let date_w = DATE_COL_W * zoom;
    let min_name_w = MIN_FILENAME_W * zoom;

    let (phase, size_slot, date_slot) =
        if row_w >= PHASE1_MIN_WIDTH * zoom && slicer.w >= min_name_w + size_w + date_w {
            let date = slicer.cut_right(date_w);
            let size = slicer.cut_right(size_w);
            (TreeDegradationPhase::Full, Some(size), Some(date))
        } else if row_w >= PHASE2_MIN_WIDTH * zoom && slicer.w >= min_name_w + size_w {
            let size = slicer.cut_right(size_w);
            (TreeDegradationPhase::NoDate, Some(size), None)
        } else {
            (TreeDegradationPhase::FilenameOnly, None, None)
        };

    let name_slot = slicer.remaining();

    TreeRowLayout {
        row_slot: Slot::new(row_x, row_y, row_w, row_h),
        indent_guide_xs,
        chevron_slot,
        icon_slot,
        name_slot,
        size_slot,
        date_slot,
        phase,
    }
}

/// Palette tokens for tree view rendering.
#[derive(Clone, Copy, Debug)]
pub struct TreeImmColors {
    pub fg: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub hover: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub bg: Hsla,
}

impl TreeImmColors {
    pub fn from_theme(cx: &gpui::App, is_glass: bool) -> Self {
        let t = cx.theme();
        Self {
            fg: t.foreground,
            accent: t.accent,
            accent_fg: t.accent_foreground,
            hover: t.list_active,
            muted: t.muted_foreground,
            border: t.border,
            bg: if is_glass {
                t.list_head
            } else {
                backdrop_tint(t.secondary, 0.18)
            },
        }
    }
}

/// Prepared row data produced in `prepaint` and consumed in `paint`.
#[derive(Clone)]
struct TreeRowPrepaint {
    idx: usize,
    layout: TreeRowLayout,
    entry: FileEntry,
    #[allow(dead_code)]
    depth: usize,
    is_expanded: bool,
    selected: bool,
    cursor: bool,
    hovered: bool,
    is_blinking: bool,
    icon_tex: Option<Arc<gpui::RenderImage>>,
    name_line: ShapedLine,
    size_line: Option<ShapedLine>,
    date_line: Option<ShapedLine>,
    check_slot: Option<Slot>,
    selected_at: Option<std::time::Instant>,
    openness: f32,
    opacity: f32,
    interactive: bool,
    /// Paint-only vertical offset (disclosure accordion); hit slots stay put.
    dy: f32,
    /// Local y above which this row is clipped: children slide out from under
    /// (and back into) their parent row instead of overlapping it.
    clip_top: Option<f32>,
}

/// Finder-style disclosure. Rows entering or leaving with a visible ancestor
/// travel between that ancestor and their slot by the same spring progress as
/// the neighbours being pushed (identical springs), so the block opens and closes
/// like an accordion without gaps or overlap.
fn apply_disclosure_accordion(rows: &mut [TreeRowPrepaint], row_h: f32) {
    let mut presented: std::collections::HashMap<PathBuf, f32> =
        std::collections::HashMap::with_capacity(rows.len());
    // Live rows are top-down, so parents resolve before their children.
    for row in rows.iter_mut().filter(|row| row.interactive) {
        let slot_y = row.layout.row_slot.y;
        let path = row.entry.path();
        if row.opacity < 1.0 {
            if let Some(parent_y) = path.parent().and_then(|p| presented.get(p)).copied() {
                row.dy = (parent_y + (slot_y - parent_y) * row.opacity) - slot_y;
                row.clip_top = Some(parent_y + row_h);
            }
        }
        presented.insert(path, slot_y + row.dy);
    }
    // Leaving rows collapse into their nearest surviving ancestor.
    for row in rows.iter_mut().filter(|row| !row.interactive) {
        let slot_y = row.layout.row_slot.y;
        let path = row.entry.path();
        if let Some(parent_y) = path.ancestors().skip(1).find_map(|p| presented.get(p)).copied() {
            row.dy = (parent_y + (slot_y - parent_y) * row.opacity) - slot_y;
            row.clip_top = Some(parent_y + row_h);
        }
    }
}

/// Prepaint state containing hit testing targets and shaped text glyphs.
pub struct TreeImmPrepaint {
    hitbox: Hitbox,
    rows: Vec<TreeRowPrepaint>,
    chevron_hits: Vec<(Slot, PathBuf)>,
    row_hits: Vec<(Slot, usize, PathBuf, bool)>,
}

/// Stateless procedural immediate-mode element for hierarchical tree rendering.
pub struct TreeImmElement {
    id: ElementId,
    pane_id: TabId,
    app: WeakEntity<App>,
    rows: Arc<[(FileEntry, usize, bool, bool)]>,
    row_h: f32,
    #[allow(dead_code)]
    scroll_y: f32,
    #[allow(dead_code)]
    viewport_h: f32,
    start_idx: usize,
    end_idx: usize,
    zoom: f32,
    colors: TreeImmColors,
    font: Font,
    font_size: Pixels,
    windows_theme: bool,
    style: StyleRefinement,
}

impl TreeImmElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pane_id: TabId,
        app: WeakEntity<App>,
        rows: Arc<[(FileEntry, usize, bool, bool)]>,
        row_h: f32,
        scroll_y: f32,
        viewport_h: f32,
        start_idx: usize,
        end_idx: usize,
        zoom: f32,
        colors: TreeImmColors,
        font: Font,
        font_size: Pixels,
        windows_theme: bool,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("tree-imm-{}", pane_id.0))),
            pane_id,
            app,
            rows,
            row_h,
            scroll_y,
            viewport_h,
            start_idx,
            end_idx,
            zoom,
            colors,
            font,
            font_size,
            windows_theme,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for TreeImmElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for TreeImmElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for TreeImmElement {
    type RequestLayoutState = Style;
    type PrepaintState = TreeImmPrepaint;

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
        crate::perf::count("tree_imm.request_layout");
        let mut style = Style::default();
        style.refine(&self.style);
        style.size.width =
            gpui::Length::Definite(gpui::DefiniteLength::Relative(gpui::relative(1.0)));
        style.size.height =
            gpui::Length::Definite(gpui::DefiniteLength::Relative(gpui::relative(1.0)));

        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        crate::perf::count("tree_imm.prepaint");

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let origin = bounds.origin;
        let total_w = f32::from(bounds.size.width);
        let zoom = self.zoom;
        let pane_id = self.pane_id;
        let row_h = self.row_h;

        // Slit/sub-pixel culling: if pane is collapsing/narrower than 12px, bypass text shaping & processing
        if bounds.size.width < px(12.0) || bounds.size.height < px(12.0) {
            return TreeImmPrepaint {
                hitbox,
                rows: Vec::new(),
                chevron_hits: Vec::new(),
                row_hits: Vec::new(),
            };
        }

        let Some(app_entity) = self.app.upgrade() else {
            return TreeImmPrepaint {
                hitbox,
                rows: Vec::new(),
                chevron_hits: Vec::new(),
                row_hits: Vec::new(),
            };
        };

        let start_idx = self.start_idx.min(self.rows.len());
        let end_idx = self.end_idx.min(self.rows.len()).max(start_idx);
        let visible_count = end_idx.saturating_sub(start_idx);

        let mut prepaint_rows = Vec::with_capacity(visible_count);
        let mut chevron_hits = Vec::new();
        let mut row_hits = Vec::with_capacity(visible_count);

        let font = self.font.clone();
        let font_size = self.font_size;
        let colors = self.colors;

        let has_gpui_drag = cx.has_active_drag();
        let now = std::time::Instant::now();
        let (epoch, instant) = {
            let app = app_entity.read(cx);
            (
                app.state.tab_states.get(&pane_id).map_or(0, |ts| {
                    super::details_element::row_content_epoch(ts, app.type_filters.get(&pane_id))
                }),
                cx.reduce_motion()
                    || app.rubber.is_some()
                    || app.rename_edit.is_some()
                    || app.is_drag_active(has_gpui_drag),
            )
        };
        let targets: Vec<_> = (start_idx..end_idx)
            .map(|i| (self.rows[i].0.path(), i, i as f32 * row_h))
            .collect();
        let motion = window.use_keyed_state(
            SharedString::from(format!("tree-row-motion-{}", pane_id.0)),
            cx,
            |_, _| RowMotion::<PathBuf>::new(epoch, [self.scroll_y, row_h, self.viewport_h]),
        );
        let journey_rows = (self.viewport_h / row_h.max(1.0)).ceil() + 2.0;
        let (positions, moving) = motion.update(cx, |state, _| {
            state.long_journey_rows = journey_rows;
            state.sample(
                &targets,
                epoch,
                [self.scroll_y, row_h, self.viewport_h],
                instant,
                now,
            )
        });
        let positions: std::collections::HashMap<_, _> =
            positions.into_iter().map(|(i, y, _)| (i, y)).collect();
        app_entity.update(cx, |this, _| {
            let is_drag_active = this.is_drag_active(has_gpui_drag);
            if is_drag_active {
                // Cleanly purge stale folder targets for this pane on each prepaint
                this.pending_drag_hover_targets
                    .retain(|t| !matches!(t.id, DragHoverId::Folder(tab, _) if tab == pane_id));
            } else if !this.pending_drag_hover_targets.is_empty() {
                this.pending_drag_hover_targets
                    .retain(|t| !matches!(t.id, DragHoverId::Folder(tab, _) if tab == pane_id));
            }

            let (selected_set, keyboard_cursor) = this
                .state
                .tab_states
                .get(&pane_id)
                .map(|ts| (ts.selection.selected.clone(), ts.selection.keyboard_cursor))
                .unwrap_or_default();

            let blink_folder_path = this.drag_hover.blink_id.as_ref().and_then(|id| match id {
                DragHoverId::Folder(p, path) if *p == pane_id => Some(path.clone()),
                _ => None,
            });

            let current_hover_row = this
                .details_hover
                .and_then(|(p, i)| (p == pane_id).then_some(i));

            for i in start_idx..end_idx {
                let (entry, depth, is_expanded, _) = &self.rows[i];
                let is_dir = entry.kind() == FileKind::Dir;
                let row_y = positions.get(&i).copied().unwrap_or(i as f32 * row_h);
                // No checkbox gutter: the checkbox replaces the icon (same rule as
                // Details/List), so a multi-selection never shifts names.
                let gutter = 0.0;
                let multi_select = selected_set.len() > 1;

                let mut layout = compute_tree_row_layout(
                    gutter,
                    row_y,
                    (total_w - gutter).max(0.0),
                    row_h,
                    *depth,
                    is_dir,
                    zoom,
                );
                layout.row_slot = Slot::new(0.0, row_y, total_w, row_h);
                let layout_icon_slot = layout.icon_slot;

                let row_bounds = layout.row_slot.to_bounds(origin);

                // Register drag hover target for every visible directory row only if drag is active
                if is_dir && is_drag_active {
                    let r_bounds = row_bounds.intersect(&window.content_mask().bounds);
                    this.register_drag_hover_target(DragHoverTarget {
                        id: DragHoverId::Folder(pane_id, entry.path()),
                        bounds: r_bounds,
                        kind: DragHoverKind::Folder,
                        command: DragHoverCommand::NavigatePathIn(pane_id, entry.path()),
                        drop_command: Some(DragHoverDropCommand::TransferTo(entry.path())),
                    });
                }

                // Register hit targets
                if let Some(ch_slot) = layout.chevron_slot {
                    chevron_hits.push((ch_slot, entry.path()));
                }
                row_hits.push((layout.row_slot, i, entry.path(), is_dir));

                let selected = selected_set.contains(&i);
                let cursor = keyboard_cursor == Some(i);
                let hovered = current_hover_row == Some(i);
                let is_blinking = is_dir && blink_folder_path.as_ref() == Some(&entry.path());

                let icon_tex = this
                    .file_icon_for_entry(&entry.path(), &entry.kind(), entry.extension().as_deref())
                    .texture();

                let name_color = if selected {
                    colors.accent_fg
                } else {
                    colors.fg
                };
                let meta_color = if selected {
                    colors.accent_fg.opacity(0.85)
                } else {
                    colors.muted
                };

                // UTF-8 binary search text truncation ensuring multi-byte code points are never split
                let name_line = shape_text_truncated(
                    &entry.name,
                    &font,
                    font_size,
                    name_color,
                    layout.name_slot.w,
                    window,
                );

                let size_line = layout.size_slot.map(|slot| {
                    shape_text_truncated(
                        &crate::ui::views::details::entry_display_size(&entry),
                        &font,
                        font_size,
                        meta_color,
                        slot.w,
                        window,
                    )
                });

                let date_line = layout.date_slot.map(|slot| {
                    shape_text_truncated(
                        &crate::ui::views::details::entry_display_date(&entry),
                        &font,
                        font_size,
                        meta_color,
                        slot.w,
                        window,
                    )
                });

                prepaint_rows.push(TreeRowPrepaint {
                    idx: i,
                    layout,
                    entry: entry.clone(),
                    depth: *depth,
                    is_expanded: *is_expanded,
                    selected,
                    cursor,
                    hovered,
                    is_blinking,
                    icon_tex,
                    name_line,
                    size_line,
                    date_line,
                    check_slot: multi_select.then_some(layout_icon_slot),
                    selected_at: None,
                    openness: if *is_expanded { 1.0 } else { 0.0 },
                    opacity: 1.0,
                    interactive: true,
                    dy: 0.0,
                    clip_top: None,
                });
            }
        });

        let feedback = window.use_keyed_state(
            SharedString::from(format!("tree-selection-feedback-{}", pane_id.0)),
            cx,
            |_, _| selection_checkbox::SelectionFeedback::<PathBuf>::default(),
        );
        let selection: Vec<_> = prepaint_rows
            .iter()
            .map(|row| {
                (
                    row.entry.path(),
                    row.selected && row.check_slot.is_some(),
                )
            })
            .collect();
        let selected_at = feedback.update(cx, |state, _| state.sample(&selection, now));
        let disclosure = window.use_keyed_state(
            SharedString::from(format!("tree-disclosure-{}", pane_id.0)),
            cx,
            |_, _| DisclosureMotion::<PathBuf>::new(),
        );
        let expanded: Vec<_> = prepaint_rows
            .iter()
            .map(|row| (row.entry.path(), row.is_expanded))
            .collect();
        let (openness, rotating) =
            disclosure.update(cx, |state, _| state.sample(&expanded, instant, now));
        for ((row, at), open) in prepaint_rows.iter_mut().zip(selected_at).zip(openness) {
            row.selected_at = at;
            row.openness = open;
        }
        let presence = window.use_keyed_state(
            SharedString::from(format!("tree-row-presence-{}", pane_id.0)),
            cx,
            |_, _| PresenceMotion::<PathBuf, TreeRowPrepaint>::new(),
        );
        let rows = prepaint_rows
            .into_iter()
            .map(|row| (row.entry.path(), row))
            .collect();
        let epoch = presentation_epoch(epoch, &[self.scroll_y, self.viewport_h, row_h, total_w]);
        let (samples, appearing) =
            presence.update(cx, |state, _| state.sample(rows, epoch, instant, now));
        let mut prepaint_rows: Vec<TreeRowPrepaint> = samples
            .into_iter()
            .map(|sample| {
                let mut row = sample.value;
                row.opacity = sample.opacity;
                row.interactive = sample.interactive;
                row.dy = 0.0;
                row.clip_top = None;
                row
            })
            .collect();
        if appearing && !instant {
            apply_disclosure_accordion(&mut prepaint_rows, row_h);
        }
        if moving || rotating || appearing {
            window.request_animation_frame();
        }

        TreeImmPrepaint {
            hitbox,
            rows: prepaint_rows,
            chevron_hits,
            row_hits,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        crate::perf::count("tree_imm.paint");

        if bounds.size.width < px(12.0) || bounds.size.height < px(12.0) || prepaint.rows.is_empty()
        {
            return;
        }

        let origin = bounds.origin;
        let zoom = self.zoom;
        let colors = self.colors;
        let windows_theme = self.windows_theme;
        let mouse_pos = window.mouse_position();
        let is_over = prepaint.hitbox.is_hovered(window);

        let active_hover_idx = if is_over {
            let lx = f32::from(mouse_pos.x - origin.x);
            let ly = f32::from(mouse_pos.y - origin.y);
            prepaint
                .row_hits
                .iter()
                .rev()
                .find(|(slot, _, _, _)| slot.contains(lx, ly))
                .map(|(_, idx, _, _)| *idx)
        } else {
            None
        };

        if is_over {
            window.set_cursor_style(CursorStyle::PointingHand, &prepaint.hitbox);
        }

        // Paint background and rows within hardware clipped viewport
        for row in &prepaint.rows {
            let clip = row.clip_top.map(|top| gpui::ContentMask {
                bounds: Bounds::from_corners(
                    point(bounds.left(), origin.y + px(top)),
                    bounds.bottom_right(),
                ),
                fade_out: Default::default(),
            });
            let origin = point(origin.x, origin.y + px(row.dy));
            window.with_content_mask(clip, |window| {
            window.with_element_opacity(Some(row.opacity * crate::ui::views::rename::cut_path_opacity(&self.app, &row.entry.path(), cx)), |window| {
                let row_slot = row.layout.row_slot;
                let row_bounds = row_slot.to_bounds(origin);

                // 1. Row background fill (Selection > Hover > Normal)
                if row.selected {
                    paint_rounded(window, row_bounds, 3.0 * zoom, colors.accent);
                } else if row.interactive && (active_hover_idx == Some(row.idx) || row.hovered) {
                    paint_rounded(window, row_bounds, 3.0 * zoom, colors.hover);
                }

                // Visual blink feedback during drag hover
                if row.interactive {
                    crate::ui::operation_feedback::paint_file_feedback(&self.app, &row.entry.path(), row_bounds, colors.accent, 3.0 * zoom, window, cx);
                }
                if row.is_blinking {
                    paint_rounded(window, row_bounds, 3.0 * zoom, colors.accent.opacity(0.40));
                }

                // Keyboard cursor indicator border
                if row.cursor && !row.selected && row.interactive {
                    let border_col = colors
                        .accent
                        .opacity(if row.selected { 0.95 } else { 0.65 });
                    window.paint_quad(gpui::quad(
                        row_bounds,
                        Corners::all(px(3.0 * zoom)),
                        transparent_black(),
                        Edges::all(px(1.0)),
                        border_col,
                        Default::default(),
                    ));
                }

                // 2. Indentation guide lines: 1px vertical line at x + 8.0 for each depth level
                let guide_color = if row.selected {
                    colors.accent_fg.opacity(0.35)
                } else {
                    colors.border.opacity(0.40)
                };
                for &guide_x in &row.layout.indent_guide_xs {
                    let line_bounds = Bounds {
                        origin: point(origin.x + px(guide_x), row_bounds.origin.y),
                        size: size(px(1.0), row_bounds.size.height),
                    };
                    paint_fill(window, line_bounds, guide_color);
                }

                // 3. Directory chevron icon
                if let Some(ch_slot) = row.layout.chevron_slot {
                    let chevron_color = if row.selected {
                        colors.accent_fg
                    } else {
                        colors.muted
                    };
                    let sz = CHEVRON_ICON_SZ * zoom;
                    let icon_bounds = Bounds {
                        origin: point(
                            origin.x + px(ch_slot.x + (ch_slot.w - sz) * 0.5),
                            origin.y + px(ch_slot.y + (ch_slot.h - sz) * 0.5),
                        ),
                        size: size(px(sz), px(sz)),
                    };
                    let center = icon_bounds.center();
                    let scale = window.scale_factor();
                    let transform = gpui::TransformationMatrix::unit()
                        .translate(center.scale(scale))
                        .rotate(gpui::radians(row.openness * std::f32::consts::FRAC_PI_2))
                        .translate(center.scale(-scale));
                    let _ = window.paint_svg(
                        icon_bounds,
                        AtlasIcon::CaretRight.path().into(),
                        None,
                        transform,
                        chevron_color,
                        cx,
                    );
                }

                // 4. File / folder icon
                let icon_slot = row.layout.icon_slot;
                let icon_sz = ICON_SZ * zoom;
                let icon_b = icon_slot.to_bounds(origin);
                let icon_bounds = Bounds {
                    origin: point(
                        icon_b.origin.x + px((f32::from(icon_b.size.width) - icon_sz) * 0.5),
                        icon_b.origin.y + px((f32::from(icon_b.size.height) - icon_sz) * 0.5),
                    ),
                    size: size(px(icon_sz), px(icon_sz)),
                };

                if row.selected && row.check_slot.is_some() {
                    selection_checkbox::paint_animated(
                        window,
                        cx,
                        icon_bounds.center(),
                        selection_checkbox::ROW_SIZE * zoom,
                        row.selected_at,
                    );
                } else if let Some(tex) = &row.icon_tex {
                    let _ = window.paint_image(
                        icon_bounds,
                        icon_bounds,
                        Corners::all(px(0.0)),
                        tex.clone(),
                        0,
                        false,
                    );
                } else {
                    let icon_kind = if row.entry.kind() == FileKind::Dir {
                        AtlasIcon::Folder
                    } else {
                        AtlasIcon::File
                    };
                    let icon_color = if row.selected {
                        colors.accent_fg
                    } else {
                        colors.accent
                    };
                    paint_theme_icon(
                        window,
                        cx,
                        origin,
                        icon_slot,
                        icon_kind,
                        windows_theme,
                        icon_sz,
                        icon_color,
                    );
                }

                // 5. Filename
                let name_bounds = row.layout.name_slot.to_bounds(origin);
                if name_bounds.size.width > px(0.0) {
                    window.paint_layer(name_bounds, |window| {
                        let text_x = name_bounds.origin.x;
                        let text_y = origin.y + px(text_origin_y(row_slot.y, zoom));
                        let _ = row.name_line.paint(
                            point(text_x, text_y),
                            px(row_slot.h),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    });
                }

                // 6. Aligned metadata: Size
                if let (Some(size_slot), Some(size_line)) = (row.layout.size_slot, &row.size_line) {
                    let size_bounds = size_slot.to_bounds(origin);
                    if size_bounds.size.width > px(0.0) {
                        window.paint_layer(size_bounds, |window| {
                            let text_w = line_width(size_line);
                            let text_x = size_bounds.origin.x + size_bounds.size.width - px(text_w);
                            let text_y = origin.y + px(text_origin_y(row_slot.y, zoom));
                            let _ = size_line.paint(
                                point(text_x, text_y),
                                px(row_slot.h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        });
                    }
                }

                // 7. Aligned metadata: Date
                if let (Some(date_slot), Some(date_line)) = (row.layout.date_slot, &row.date_line) {
                    let date_bounds = date_slot.to_bounds(origin);
                    if date_bounds.size.width > px(0.0) {
                        window.paint_layer(date_bounds, |window| {
                            let text_w = line_width(date_line);
                            let text_x = date_bounds.origin.x + date_bounds.size.width - px(text_w);
                            let text_y = origin.y + px(text_origin_y(row_slot.y, zoom));
                            let _ = date_line.paint(
                                point(text_x, text_y),
                                px(row_slot.h),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        });
                    }
                }
            });
            });
        }

        // Interactive mouse handlers
        if let Some(app_entity) = self.app.upgrade() {
            let click_view = app_entity.clone();
            let move_view = app_entity;
            let pane_id = self.pane_id;
            let chevron_hits = prepaint.chevron_hits.clone();
            let row_hits = prepaint.row_hits.clone();
            let move_row_hits = prepaint.row_hits.clone();
            // Visible checkboxes (they replace the icon on selected rows).
            let check_hits: Vec<(Slot, usize)> = prepaint
                .rows
                .iter()
                .filter(|row| row.interactive && row.selected)
                .filter_map(|row| row.check_slot.map(|slot| (slot, row.idx)))
                .collect();
            let click_hitbox = prepaint.hitbox.clone();
            let move_hitbox = prepaint.hitbox.clone();

            window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !click_hitbox.is_hovered(window) {
                    return;
                }
                let lx = f32::from(ev.position.x - origin.x);
                let ly = f32::from(ev.position.y - origin.y);

                if ev.button == MouseButton::Left {
                    // Check chevron click first
                    for (ch_slot, ch_path) in chevron_hits.iter().rev() {
                        if ch_slot.contains(lx, ly)
                            && row_hits
                                .iter()
                                .rev()
                                .find(|(slot, _, _, _)| slot.contains(lx, ly))
                                .is_some_and(|(_, _, path, _)| path == ch_path)
                        {
                            let path = ch_path.clone();
                            click_view.update(cx, |this, cx| {
                                if this.suppress_navigation_double_click(pane_id, ev.position, ev.click_count) { return; }
                                this.activate_tab(pane_id);
                                this.process_actions(vec![Action::ToggleTreeExpand(path)]);
                                this.notify_pane(pane_id, cx);
                                cx.notify();
                            });
                            return;
                        }
                    }

                    // Check row click
                    for (row_slot, idx, path, is_dir) in row_hits.iter().rev() {
                        if row_slot.contains(lx, ly) {
                            let idx = *idx;
                            let path = path.clone();
                            let is_dir = *is_dir;
                            let m = ev.modifiers;
                            // A click on a visible checkbox toggles just that row.
                            let on_check = check_hits
                                .iter()
                                .any(|(slot, row)| *row == idx && slot.contains(lx, ly));
                            let (shift, ctrl) =
                                (m.shift && !on_check, m.control || m.platform || on_check);
                            let activate = ev.click_count >= 2 && !on_check;

                            click_view.update(cx, |this, cx| {
                                if this.rename_edit.is_some() {
                                    this.commit_rename(cx);
                                }
                                if this.suppress_navigation_double_click(pane_id, ev.position, ev.click_count) { return; }
                                if this.state.settings.single_click_opens_folder(is_dir, ev.click_count, shift || ctrl) {
                                    this.queue_tree_folder_click(pane_id, idx, path, ev.position);
                                } else if activate {
                                    if is_dir {
                                        this.process_actions(vec![Action::ToggleTreeExpand(path)]);
                                    } else {
                                        this.spawn_open_files(vec![path]);
                                    }
                                } else {
                                    this.activate_tab(pane_id);
                                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                                        // Slow second click on the sole selected row renames,
                                        // like every other view.
                                        let sole = ts.selection.selected.len() == 1
                                            && ts.selection.selected.contains(&idx);
                                        let slow = ts.selection.slow_rename_click(
                                            idx,
                                            std::time::Instant::now(),
                                            !shift && !ctrl,
                                        );
                                        if slow && sole {
                                            this.pending_rename = Some((pane_id, path.clone(), false));
                                        }
                                        ts.selection.keyboard_cursor = Some(idx);
                                        ts.selection.last_clicked_index = Some(idx);
                                        if shift {
                                            let anchor =
                                                ts.selection.selection_anchor.unwrap_or(idx);
                                            let (lo, hi) = (anchor.min(idx), anchor.max(idx));
                                            ts.selection.selected.clear();
                                            ts.selection.selected.extend(lo..=hi);
                                        } else if ctrl {
                                            if ts.selection.selected.contains(&idx) {
                                                ts.selection.selected.remove(&idx);
                                            } else {
                                                ts.selection.selected.insert(idx);
                                            }
                                        } else {
                                            ts.selection.selection_anchor = Some(idx);
                                            ts.selection.selected.clear();
                                            ts.selection.selected.insert(idx);
                                        }
                                    }
                                }
                                this.notify_pane(pane_id, cx);
                                cx.notify();
                            });
                            return;
                        }
                    }
                } else if ev.button == MouseButton::Right {
                    let pos = ev.position;
                    for (row_slot, idx, path, _) in row_hits.iter().rev() {
                        if row_slot.contains(lx, ly) {
                            let idx = *idx;
                            let path = path.clone();
                            click_view.update(cx, |this, cx| {
                                this.activate_tab(pane_id);
                                if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                                    if !ts.selection.selected.contains(&idx) {
                                        ts.selection.keyboard_cursor = Some(idx);
                                        ts.selection.selection_anchor = Some(idx);
                                        ts.selection.last_clicked_index = Some(idx);
                                        ts.selection.selected.clear();
                                        ts.selection.selected.insert(idx);
                                    }
                                }
                                this.open_ctx_menu(vec![path], pos, window, cx);
                                this.notify_pane(pane_id, cx);
                                cx.notify();
                            });
                            return;
                        }
                    }
                }
            });

            // Mouse move: dismiss previous tooltip and track hovered row
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !move_hitbox.is_hovered(window) {
                    return;
                }
                let lx = f32::from(ev.position.x - origin.x);
                let ly = f32::from(ev.position.y - origin.y);
                let mut hovered_entry = None;
                for (row_slot, idx, _, _) in move_row_hits.iter().rev() {
                    if row_slot.contains(lx, ly) {
                        hovered_entry = Some(*idx);
                        break;
                    }
                }
                move_view.update(cx, |this, cx| {
                    let hover_pair = hovered_entry.map(|idx| (pane_id, idx));
                    if this.details_hover != hover_pair {
                        gpui_component::tooltip::Tooltip::dismiss(window, cx);
                        this.details_hover = hover_pair;
                        this.notify_pane(pane_id, cx);
                        cx.notify();
                    }
                });
            });
        }
    }
}

/// Transparent drag source over the tree rows. The press already selected the
/// row (element mouse-down), so a drag carries the whole tree selection.
fn tree_drag_layer(view: &gpui::Entity<App>, pane_id: TabId, row_h: f32, total_height: f32) -> AnyElement {
    let drag_view = view.clone();
    div()
        .id(SharedString::from(format!("tree-imm-drag-layer-{}", pane_id.0)))
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(px(total_height))
        .on_drag_alt(move |cursor_offset, cursor_style, window, cx| {
            let item = {
                let app = drag_view.read(&*cx);
                let row = (f32::from(cursor_offset.y) / row_h.max(1.0)).floor() as usize;
                app.state.tab_states.get(&pane_id).and_then(|ts| {
                    if !ts.selection.selected.contains(&row) {
                        return None;
                    }
                    ts.visible_tree_rows_snapshot().get(row).map(|r| (row, r.0.path()))
                })
            };
            let Some((row, anchor)) = item else {
                return crate::ui::drag::empty_any_drag(cx);
            };
            crate::ui::drag::build_file_any_drag(
                &drag_view,
                Some((pane_id, row)),
                anchor,
                cursor_style,
                window,
                cx,
            )
        })
        .into_any_element()
}

/// Public render entrypoint for Atlas: renders the hierarchical Tree view via immediate-mode architecture.
pub fn render_tree_view_imm(
    app: &mut App,
    pane_id: TabId,
    window: &mut Window,
    cx: &mut Context<App>,
) -> AnyElement {
    app.render_tree_view_imm(pane_id, window, cx)
}

impl App {
    /// Public render entrypoint on `App` for immediate-mode hierarchical tree view.
    pub fn render_tree_view_imm(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.view.tree_imm");
        let view = cx.entity();
        let active_theme = self.state.profiles.active().theme.as_str();
        let is_glass = active_theme.ends_with(" Glass");
        let windows_theme = is_windows_theme(active_theme);
        let colors = TreeImmColors::from_theme(cx, is_glass);

        let crumb_strip = self.render_crumb_strip(
            pane_id,
            &view,
            colors.fg,
            colors.muted,
            colors.border,
            colors.bg,
            cx,
        );

        let rows: Arc<[(FileEntry, usize, bool, bool)]> = {
            let Some(ts) = self.state.tab_states.get_mut(&pane_id) else {
                return div().into_any_element();
            };
            ts.visible_tree_rows()
                .into_iter()
                .map(|(e, d, exp, has)| (e.clone(), d, exp, has))
                .collect::<Vec<_>>()
                .into()
        };

        // Fetch children asynchronously if an expanded directory has not loaded them yet.
        for (entry, _, is_expanded, _) in rows.iter() {
            if entry.kind() == FileKind::Dir && *is_expanded {
                let has_children = self
                    .state
                    .tab_states
                    .get(&pane_id)
                    .is_some_and(|ts| ts.tree.children.contains_key(&entry.path()));
                if !has_children {
                    self.spawn_list_tree(pane_id, entry.path());
                }
            }
        }

        let body: AnyElement = if rows.is_empty() {
            let empty_msg =
                self.empty_pane_message(pane_id, rust_i18n::t!("empty_folder").into_owned());
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .children(empty_msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let base_scroll = scroll.0.borrow().base_handle.clone();
            let mut scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);

            let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
            let row_h = TREE_ROW_H * zoom_scale;
            base_scroll.set_line_scroll_height(gpui::px(row_h));

            let viewport_h = {
                let tracked = f32::from(base_scroll.bounds().size.height);
                if tracked > row_h {
                    tracked
                } else {
                    f32::from(window.viewport_size().height)
                }
            };

            let count = rows.len();
            let total_height = count as f32 * row_h;
            let max_scroll = self.file_scroll_max(total_height, viewport_h);
            if crate::ui::imm::scroll::apply_deferred(&scroll, true, viewport_h, max_scroll, |row| {
                (row < count).then(|| (row as f32 * row_h, row_h))
            }) {
                scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            }
            if scroll_y > max_scroll {
                let offset = base_scroll.offset();
                base_scroll.set_offset(gpui::point(offset.x, gpui::px(-max_scroll)));
                scroll_y = max_scroll;
            }

            let overscan_rows = ((viewport_h * 0.2) / row_h).ceil() as usize;
            let start_idx =
                ((scroll_y / row_h).floor().max(0.0) as usize).saturating_sub(overscan_rows);
            let end_idx =
                (((scroll_y + viewport_h) / row_h).ceil() as usize + overscan_rows).min(count);

            // Pre-request icons for visible entries
            for i in start_idx..end_idx {
                let (entry, _, _, _) = &rows[i];
                let file_ref = crate::ui::file_icons::FileIconRef {
                    path: entry.path(),
                    kind: entry.kind().clone(),
                    extension: entry
                        .extension()
                        .as_ref()
                        .map(|s| SharedString::from(s.to_string())),
                };
                self.maybe_request_file_icon(&file_ref);
            }

            let font = window.text_style().font();
            let font_size = window.text_style().font_size.to_pixels(window.rem_size());

            let tree_el = TreeImmElement::new(
                pane_id,
                view.downgrade(),
                rows.clone(),
                row_h,
                scroll_y,
                viewport_h,
                start_idx,
                end_idx,
                zoom_scale,
                colors,
                font,
                font_size,
                windows_theme,
            )
            .absolute()
            .inset_0()
            .into_any_element();

            let scroll_view = view.clone();

            let tree_tooltip = self
                .details_hover
                .and_then(|(hover_pane, entry_idx)| (hover_pane == pane_id).then_some(entry_idx))
                .and_then(|entry_idx| rows.get(entry_idx))
                .map(|(entry, _, _, _)| {
                    (
                        entry.name.to_string(),
                        entry.path(),
                        matches!(entry.kind(), FileKind::Dir),
                        entry.size(),
                        entry.modified,
                    )
                });

            let tooltip_target = self
                .details_hover
                .filter(|(hover_pane, _)| *hover_pane == pane_id)
                .map(|(_, entry_idx)| entry_idx);
            // Stable id: the per-row tooltip lives in a sibling overlay so hover
            // changes never reset the tree's keyed state (checkbox pop, FLIP,
            // presence, disclosure chevrons).
            let tooltip_overlay = tooltip_target.zip(tree_tooltip).map(|(target, data)| {
                crate::ui::views::hover_tooltip_overlay("tree", pane_id, target, data)
            });

            // Inline rename editor over the row's name slot (same geometry as paint).
            let rename_target = rows
                .iter()
                .enumerate()
                .skip(start_idx)
                .take(end_idx.saturating_sub(start_idx))
                .find_map(|(i, (entry, depth, _, _))| {
                    let input =
                        crate::ui::views::rename::active_input_for_path(self, pane_id, &entry.path())?;
                    Some((i, entry.name.to_string(), *depth, entry.kind() == FileKind::Dir, input))
                });
            let rename_overlay = rename_target.map(|(i, name, depth, is_dir, input)| {
                let total_w = {
                    let tracked = f32::from(base_scroll.bounds().size.width);
                    if tracked > 1.0 { tracked } else { f32::from(window.viewport_size().width) }
                };
                let gutter = 0.0;
                let layout = compute_tree_row_layout(
                    gutter,
                    i as f32 * row_h,
                    (total_w - gutter).max(0.0),
                    row_h,
                    depth,
                    is_dir,
                    zoom_scale,
                );
                let slot = layout.name_slot;
                let input_w =
                    crate::ui::views::rename::input_width(&name, slot.w, font_size, window);
                div()
                    .absolute()
                    .left(px(slot.x))
                    .top(px(slot.y))
                    .w(px(input_w))
                    .h(px(slot.h))
                    .child(crate::ui::views::rename::input_element(
                        &input,
                        &view,
                        px(slot.h),
                        font_size,
                        window,
                        cx,
                    ))
                    .into_any_element()
            });

            let content_scroller = div()
                .id(SharedString::from(format!("tree-imm-viewport-{}", pane_id.0)))
                .size_full()
                .relative()
                .overflow_hidden()
                .child(
                    div()
                        .id(SharedString::from(format!("tree-imm-scroll-{}", pane_id.0)))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&base_scroll)
                        .on_scroll_wheel(move |ev, window, cx| {
                            if ev.modifiers.control || ev.modifiers.platform {
                                ctrl_scroll(ev, pane_id, &scroll_view, window, cx);
                            }
                        })
                        .child(
                            div()
                                .w_full()
                                .h(px(total_height))
                                .relative()
                                .child(tree_el)
                                .child(tree_drag_layer(&view, pane_id, row_h, total_height))
                                // Above the drag layer so the editor owns its clicks.
                                .children(rename_overlay),
                        )
                        .vertical_scrollbar(&base_scroll),
                )
                .children(tooltip_overlay);

            v_flex()
                .size_full()
                .flex_1()
                .min_h(px(0.))
                .child(content_scroller)
                .into_any_element()
        };

        let pane_view = view;
        crate::ui::views::pane_background_interactions(
            v_flex()
                .id(SharedString::from(format!("pane-{}", pane_id.0)))
                .size_full()
                .overflow_hidden(),
            &pane_view,
            pane_id,
        )
        .child(crumb_strip)
        .child(body)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_row_layout_full() {
        let layout = compute_tree_row_layout(0.0, 0.0, 400.0, 24.0, 0, true, 1.0);
        assert_eq!(layout.phase, TreeDegradationPhase::Full);
        assert!(layout.chevron_slot.is_some());
        assert!(layout.size_slot.is_some());
        assert!(layout.date_slot.is_some());

        let ch = layout.chevron_slot.unwrap();
        assert_eq!(ch.x, ROW_PAD_X);
        assert_eq!(ch.w, CHEVRON_W);

        let icon = layout.icon_slot;
        assert_eq!(icon.x, ROW_PAD_X + CHEVRON_W);
        assert_eq!(icon.w, ICON_W);

        let date = layout.date_slot.unwrap();
        assert_eq!(date.w, DATE_COL_W);
        assert_eq!(date.x, 400.0 - ROW_PAD_X - DATE_COL_W);

        let size = layout.size_slot.unwrap();
        assert_eq!(size.w, SIZE_COL_W);
        assert_eq!(size.x, 400.0 - ROW_PAD_X - DATE_COL_W - SIZE_COL_W);

        assert_eq!(layout.name_slot.x, ROW_PAD_X + CHEVRON_W + ICON_W);
        assert_eq!(
            layout.name_slot.w,
            400.0 - (ROW_PAD_X * 2.0) - CHEVRON_W - ICON_W - SIZE_COL_W - DATE_COL_W
        );
    }

    #[test]
    fn test_tree_row_layout_indent_guides() {
        let layout = compute_tree_row_layout(0.0, 0.0, 400.0, 24.0, 3, false, 1.0);
        assert_eq!(layout.indent_guide_xs.len(), 3);
        assert_eq!(layout.indent_guide_xs[0], ROW_PAD_X + GUIDE_OFFSET_X);
        assert_eq!(
            layout.indent_guide_xs[1],
            ROW_PAD_X + INDENT_STEP + GUIDE_OFFSET_X
        );
        assert_eq!(
            layout.indent_guide_xs[2],
            ROW_PAD_X + INDENT_STEP * 2.0 + GUIDE_OFFSET_X
        );
        assert!(layout.chevron_slot.is_none());
    }

    #[test]
    fn test_tree_row_layout_degradation_nodate() {
        // Row width 260px (< 300px, >= 200px)
        let layout = compute_tree_row_layout(0.0, 0.0, 260.0, 24.0, 1, true, 1.0);
        assert_eq!(layout.phase, TreeDegradationPhase::NoDate);
        assert!(layout.chevron_slot.is_some());
        assert!(layout.date_slot.is_none());
        assert!(layout.size_slot.is_some());

        let size = layout.size_slot.unwrap();
        assert_eq!(size.w, SIZE_COL_W);
        assert_eq!(size.x, 260.0 - ROW_PAD_X - SIZE_COL_W);

        // Filename takes remaining space
        assert_eq!(
            layout.name_slot.w,
            260.0 - (ROW_PAD_X * 2.0) - INDENT_STEP - CHEVRON_W - ICON_W - SIZE_COL_W
        );
    }

    #[test]
    fn test_tree_row_layout_degradation_filename_only() {
        // Row width 180px (< 200px)
        let layout = compute_tree_row_layout(0.0, 0.0, 180.0, 24.0, 0, true, 1.0);
        assert_eq!(layout.phase, TreeDegradationPhase::FilenameOnly);
        assert!(layout.chevron_slot.is_some());
        assert!(layout.date_slot.is_none());
        assert!(layout.size_slot.is_none());

        // 100% of remaining space to filename
        assert_eq!(
            layout.name_slot.w,
            180.0 - (ROW_PAD_X * 2.0) - CHEVRON_W - ICON_W
        );
    }

    #[test]
    fn test_tree_row_deep_indent_preserves_filename() {
        // 320px width, but depth 8 takes 8 * 18 = 144px indent.
        // Even though row_w >= 300px, date drops because remaining space cannot fit full metadata.
        let layout = compute_tree_row_layout(0.0, 0.0, 320.0, 24.0, 8, true, 1.0);
        assert_ne!(layout.phase, TreeDegradationPhase::Full);
        assert!(layout.name_slot.w >= MIN_FILENAME_W);
    }
}
