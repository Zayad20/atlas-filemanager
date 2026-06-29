use gpui::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// An experimental Virtual Grid for GPUI that supports FLIP animations.
/// An experimental Virtual Grid for GPUI that supports FLIP animations.
///
/// # THE GOAL (El Objetivo)
/// We want a buttery smooth user experience where resizing the window or changing
/// the zoom scale of the grid causes the items to smoothly slide into their new
/// positions, even if they wrap to the next line.
///
/// # THE PROBLEM (El Problema con `uniform_list`)
/// Currently, `Grid` and `Details` views use `gpui::uniform_list`. This list chunks
/// items into `Row` components. When the width shrinks, an item drops to a different
/// row index. Because GPUI's layout engine hashes the parent tree to generate the
/// `GlobalElementId`, changing rows completely destroys the item's `GlobalElementId`.
/// As a result, GPUI's built-in `with_animation` state is lost, the old bounds are
/// discarded, and the item "snaps" (se ve "choppy") instead of smoothly interpolating.
///
/// # THE SOLUTION (La Solución: AnimatedVirtualGrid)
/// This custom `AnimatedVirtualGrid` element implements a FLIP (First, Last, Invert, Play)
/// animation system at the lowest GPUI layout level.
///
/// 1. **Flat Hierarchy**: We discard rows. The virtual grid takes the viewport dimensions,
///    calculates the columns, and renders a flat list of visible items directly.
///    This guarantees `GlobalElementId` never resets simply because an item wrapped lines.
/// 2. **Manual Layout**: We calculate `(x, y)` based on `(index % columns)` and `(index / columns)`.
/// 3. **FLIP Animation (`last_item_bounds`)**: During `prepaint()`, we capture the previous
///    `Bounds` for each index. If an item moves to a new line, we compare its old absolute
///    `Bounds` against its new `Bounds` to calculate a `delta_x` and `delta_y`.
/// 4. **Translation (`paint()`)**: We use `window.with_element_offset(delta)` over a 250ms
///    easing curve to visually slide the item into place.
///
/// # TASKS FOR NEXT SESSION (Instrucciones de Implementación)
/// - [ ] Implement `request_layout` to compute the total scrollable height:
///       `total_height = ceil(item_count / cols) * item_size.height`
/// - [ ] Implement `prepaint` to map `scroll_offset` + `viewport_height` into a visible
///       range of item indexes (`start_idx`..`end_idx`).
/// - [ ] In `prepaint`, invoke the `render_item` closure for visible items. Compare their
///       new `Bounds` against `last_item_bounds` to construct an active `Animation` state.
/// - [ ] In `paint`, check if an item has an active animation. If yes, compute its `offset`
///       based on the animation progress (`ease_out`), translate the canvas, and paint it.
/// - [ ] Wire this up in `crates/atlas-ui/src/ui/views/grid.rs` replacing `uniform_list`.
pub struct AnimatedVirtualGrid {
    id: ElementId,
    item_count: usize,
    cols: usize,
    item_size: gpui::Size<gpui::Pixels>,
    item_spacing: gpui::Size<gpui::Pixels>,
    scroll_offset: f32,
    viewport_height: f32,
    animate: bool,
    // Provide a closure to render individual items based on index and whether it is actively animating
    render_item: Box<dyn FnMut(usize, bool, &mut Window, &mut App) -> AnyElement>,
}

impl AnimatedVirtualGrid {
    pub fn new(
        id: impl Into<ElementId>,
        item_count: usize,
        cols: usize,
        item_size: gpui::Size<gpui::Pixels>,
        item_spacing: gpui::Size<gpui::Pixels>,
        scroll_offset: f32,
        viewport_height: f32,
        animate: bool,
        render_item: impl FnMut(usize, bool, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            item_count,
            cols,
            item_size,
            item_spacing,
            scroll_offset,
            viewport_height,
            animate,
            render_item: Box::new(render_item),
        }
    }
}

#[derive(Clone)]
pub struct ActiveAnimation {
    start_time: Instant,
    start_offset: gpui::Point<Pixels>,
}

#[derive(Clone, Default)]
pub struct AnimatedVirtualGridState {
    /// Maps child index to its laid out bounds from the *previous* frame.
    /// Used to calculate FLIP (First, Last, Invert, Play) translation offsets.
    last_item_bounds: HashMap<usize, Bounds<Pixels>>,
    animations: HashMap<usize, ActiveAnimation>,
}

impl IntoElement for AnimatedVirtualGrid {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AnimatedVirtualGrid {
    type RequestLayoutState = ();
    type PrepaintState = (Vec<(usize, AnyElement)>, HashMap<usize, ActiveAnimation>);

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let rows = if self.cols == 0 {
            0
        } else {
            (self.item_count + self.cols - 1) / self.cols
        };
        let _perf = crate::perf::span_detail("grid.request_layout", || {
            format!("items={} rows={} cols={}", self.item_count, rows, self.cols)
        });
        let pitch = f32::from(self.item_size.height) + f32::from(self.item_spacing.height);
        let total_height = if rows == 0 {
            gpui::px(0.0)
        } else {
            gpui::px(rows as f32 * pitch - f32::from(self.item_spacing.height))
        };

        let mut style = gpui::Style::default();
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Absolute(
            gpui::AbsoluteLength::Pixels(total_height),
        ));

        let layout_id = _window.request_layout(style, None, _cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let mut children = Vec::new();
        if self.item_count == 0 || self.cols == 0 {
            return (children, HashMap::new());
        }

        let item_height = f32::from(self.item_size.height) + f32::from(self.item_spacing.height);
        let item_width = f32::from(self.item_size.width) + f32::from(self.item_spacing.width);

        let scroll_y = self.scroll_offset.max(0.0);
        let viewport_height = self
            .viewport_height
            .max(item_height);

        let start_y = scroll_y;
        let end_y = start_y + viewport_height;

        let start_row = (start_y / item_height).floor().max(0.0) as usize;
        let end_row = (end_y / item_height).ceil() as usize + 1;
        let max_row = (self.item_count + self.cols - 1) / self.cols;
        let end_row = end_row.min(max_row);

        let start_idx = start_row * self.cols;
        let mut end_idx = end_row * self.cols;
        if end_idx > self.item_count {
            end_idx = self.item_count;
        }
        crate::perf::count_by(
            "grid.cells.prepaint_requested",
            end_idx.saturating_sub(start_idx) as u64,
        );
        let _perf = crate::perf::span_detail("grid.prepaint", || {
            format!(
                "items={} cols={} visible={}..{}",
                self.item_count, self.cols, start_idx, end_idx
            )
        });

        let animate = self.animate;
        let mut old_bounds = HashMap::new();
        let mut old_animations = HashMap::new();

        if animate {
            _window.with_optional_element_state(
                _id,
                |state_opt: Option<Option<AnimatedVirtualGridState>>, _window| {
                    let state = state_opt.flatten().unwrap_or_default();
                    old_bounds = state.last_item_bounds.clone();
                    old_animations = state.animations.clone();
                    ((), Some(state))
                },
            );
        }

        let mut new_bounds_map = HashMap::new();
        let mut new_animations = HashMap::new();

        for i in start_idx..end_idx {
            let row = i / self.cols;
            let col = i % self.cols;

            let logical_x = gpui::px(col as f32 * item_width);
            let logical_y = gpui::px(row as f32 * item_height);
            let child_x = _bounds.origin.x + logical_x;
            let child_y = _bounds.origin.y + logical_y;

            let logical_bounds = gpui::Bounds {
                origin: gpui::Point {
                    x: logical_x,
                    y: logical_y,
                },
                size: self.item_size,
            };
            new_bounds_map.insert(i, logical_bounds);

            let active_anim = old_animations.remove(&i);
            let mut current_dx = 0.0;
            let mut current_dy = 0.0;
            let mut is_animating = false;

            if let Some(old_b) = animate.then(|| old_bounds.get(&i)).flatten() {
                let dx = old_b.origin.x - logical_x;
                let dy = old_b.origin.y - logical_y;

                if f32::from(dx).abs() > 0.5 || f32::from(dy).abs() > 0.5 {
                    let start_offset = gpui::Point { x: dx, y: dy };
                    let anim = ActiveAnimation {
                        start_time: std::time::Instant::now(),
                        start_offset,
                    };
                    new_animations.insert(i, anim.clone());
                    current_dx = f32::from(dx);
                    current_dy = f32::from(dy);
                    is_animating = true;
                } else if active_anim.is_some() {
                    let anim = active_anim.unwrap();
                    let elapsed = anim.start_time.elapsed();
                    if elapsed < Duration::from_millis(250) {
                        let progress = elapsed.as_secs_f32() / 0.25;
                        let ease_out = 1.0 - (1.0 - progress) * (1.0 - progress);
                        current_dx = f32::from(anim.start_offset.x) * (1.0 - ease_out);
                        current_dy = f32::from(anim.start_offset.y) * (1.0 - ease_out);
                        new_animations.insert(i, anim);
                        is_animating = true;
                    }
                }
            }

            let mut child = (self.render_item)(i, is_animating, _window, _cx);

            let available_space = gpui::Size {
                width: gpui::AvailableSpace::Definite(self.item_size.width),
                height: gpui::AvailableSpace::Definite(self.item_size.height),
            };
            child.layout_as_root(available_space, _window, _cx);

            let anim_x = child_x + gpui::px(current_dx);
            let anim_y = child_y + gpui::px(current_dy);
            child.prepaint_at(
                gpui::Point {
                    x: anim_x,
                    y: anim_y,
                },
                _window,
                _cx,
            );

            children.push((i, child));
        }

        if !new_animations.is_empty() {
            crate::perf::event(|| {
                format!(
                    "grid.animations active={} visible={}..{}",
                    new_animations.len(),
                    start_idx,
                    end_idx
                )
            });
            _window.request_animation_frame();
        }

        _window.with_optional_element_state::<AnimatedVirtualGridState, _>(_id, |_, _window| {
            (
                (),
                Some(AnimatedVirtualGridState {
                    last_item_bounds: new_bounds_map,
                    animations: new_animations.clone(),
                }),
            )
        });

        (children, new_animations)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        let (children, _) = _prepaint;

        for (_, child) in children {
            child.paint(_window, _cx);
        }
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
}
