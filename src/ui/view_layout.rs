use egui::{Pos2, Rect, Vec2};

/// Analytical row-major grid layout expressed in *content* (scroll-independent)
/// coordinates.
///
/// The point of this type is that it can compute the rect of **any** item — not
/// just the ones currently rendered by `ScrollArea::show_rows`. Rubber-band
/// selection relies on that: while you drag and the list auto-scrolls, items that
/// leave the viewport must stay selected, which is impossible if you only know the
/// rects of visible rows.
///
/// Positions are *visible positions* (0-based index into the post-filter
/// `visible_indices`), not raw entry indices.
#[derive(Clone, Copy)]
pub struct GridLayout {
    pub item_size: Vec2,
    pub spacing: Vec2,
    pub columns: usize,
    pub count: usize,
}

impl GridLayout {
    pub fn new(item_size: Vec2, spacing: Vec2, columns: usize, count: usize) -> Self {
        Self {
            item_size,
            spacing,
            columns: columns.max(1),
            count,
        }
    }

    /// How many columns fit in `avail_w` for an item of width `item_w` with `spacing_x`.
    pub fn columns_for_width(avail_w: f32, item_w: f32, spacing_x: f32) -> usize {
        ((avail_w + spacing_x) / (item_w + spacing_x))
            .floor()
            .max(1.0) as usize
    }

    /// Center a fixed-width grid while distributing a bounded amount of the
    /// available horizontal slack between columns.
    pub fn horizontal_distribution(
        avail_w: f32,
        item_w: f32,
        columns: usize,
        min_spacing: f32,
        max_spacing: f32,
    ) -> (f32, f32) {
        let columns = columns.max(1) as f32;
        let free = (avail_w - columns * item_w).max(0.0);
        let spacing = (free / (columns + 1.0)).clamp(min_spacing, max_spacing);
        let grid_width = columns * item_w + (columns - 1.0) * spacing;
        let side_margin = ((avail_w - grid_width) * 0.5).max(0.0);
        (spacing, side_margin)
    }

    pub fn rows(&self) -> usize {
        self.count.div_ceil(self.columns)
    }

    /// Rect of the item at visible position `pos`, in content coordinates
    /// (content top-left is the origin).
    pub fn local_item_rect(&self, pos: usize) -> Rect {
        let row = pos / self.columns;
        let col = pos % self.columns;
        let x = col as f32 * (self.item_size.x + self.spacing.x);
        let y = row as f32 * (self.item_size.y + self.spacing.y);
        Rect::from_min_size(Pos2::new(x, y), self.item_size)
    }

    /// Visible positions whose item rect intersects `rect` (content coordinates).
    pub fn indices_in_local_rect(&self, rect: Rect) -> Vec<usize> {
        if self.count == 0 {
            return Vec::new();
        }
        let stride_y = self.item_size.y + self.spacing.y;
        let first_row = (rect.min.y / stride_y).floor().max(0.0) as usize;
        let last_row = (rect.max.y / stride_y).floor().max(0.0) as usize;
        let mut out = Vec::new();
        for row in first_row..=last_row {
            for col in 0..self.columns {
                let pos = row * self.columns + col;
                if pos >= self.count {
                    break;
                }
                if self.local_item_rect(pos).intersects(rect) {
                    out.push(pos);
                }
            }
        }
        out
    }

    /// Visible position at a content-space point, if it lands on an item (not a gap).
    pub fn pos_at(&self, point: Pos2) -> Option<usize> {
        if self.count == 0 || point.x < 0.0 || point.y < 0.0 {
            return None;
        }
        let col = (point.x / (self.item_size.x + self.spacing.x)).floor() as usize;
        if col >= self.columns {
            return None;
        }
        let row = (point.y / (self.item_size.y + self.spacing.y)).floor() as usize;
        let pos = row * self.columns + col;
        (pos < self.count && self.local_item_rect(pos).contains(point)).then_some(pos)
    }

    /// Screen position of content-local `(0, 0)`, derived from one rendered cell
    /// `(sample_pos, sample_rect)`. This anchors the analytical layout to the real
    /// rendered layout regardless of margins/scroll, so screen↔content conversions
    /// stay exact.
    pub fn content_origin(&self, sample_pos: usize, sample_rect: Rect) -> Pos2 {
        sample_rect.min - self.local_item_rect(sample_pos).min.to_vec2()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn layout() -> GridLayout {
        // 3 columns, 100x40 items, 10px spacing, 10 items
        GridLayout::new(vec2(100.0, 40.0), vec2(10.0, 10.0), 3, 10)
    }

    #[test]
    fn item_rect_and_pos_at_round_trip() {
        let l = layout();
        for pos in 0..l.count {
            let rect = l.local_item_rect(pos);
            assert_eq!(l.pos_at(rect.center()), Some(pos), "center of {pos}");
        }
    }

    #[test]
    fn pos_at_returns_none_in_gaps() {
        let l = layout();
        // x in [100, 110) is the spacing gap between column 0 and column 1.
        assert_eq!(l.pos_at(pos2(105.0, 20.0)), None);
        // y in [40, 50) is the gap between row 0 and row 1.
        assert_eq!(l.pos_at(pos2(50.0, 45.0)), None);
    }

    #[test]
    fn indices_in_rect_selects_full_rows() {
        let l = layout();
        // A band covering the whole first row (y 0..40) and all columns.
        let band = Rect::from_min_max(pos2(0.0, 0.0), pos2(330.0, 39.0));
        let mut got = l.indices_in_local_rect(band);
        got.sort_unstable();
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn indices_in_rect_spans_scrolled_off_rows() {
        let l = layout();
        // A tall band reaching the last (4th) row — position 9 lives at row 3.
        let band = Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 200.0));
        let got = l.indices_in_local_rect(band);
        assert!(got.contains(&0));
        assert!(got.contains(&3)); // row 1, col 0
        assert!(got.contains(&9)); // row 3, col 0 (last item)
    }

    #[test]
    fn content_origin_recovers_screen_offset() {
        let l = layout();
        // Pretend visible position 4 (row 1, col 1) was rendered at this screen rect.
        let screen = Rect::from_min_size(pos2(1000.0 + 110.0, 500.0 + 50.0), vec2(100.0, 40.0));
        let origin = l.content_origin(4, screen);
        assert_eq!(origin, pos2(1000.0, 500.0));
        // And the inverse holds for another position.
        let screen0 = l.local_item_rect(0).translate(origin.to_vec2());
        assert_eq!(screen0.min, pos2(1000.0, 500.0));
    }

    #[test]
    fn horizontal_distribution_centers_grid_and_caps_gaps() {
        let (spacing, side_margin) =
            GridLayout::horizontal_distribution(1_000.0, 240.0, 3, 8.0, 24.0);
        assert_eq!(spacing, 24.0);
        assert_eq!(side_margin, 116.0);
    }
}
