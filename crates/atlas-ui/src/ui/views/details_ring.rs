//! Circular row slot buffer for Details paint.
//!
//! Shape work (`shape_line`) only runs on slot miss. Scroll and split reuse slots
//! whose identity (row + shape_key) still matches. Name truncation is paint-time
//! clip (`ContentMask`), so column resize does not invalidate shaped names.

use crate::app::DetailsColumn;
use gpui::{Pixels, ShapedLine};
use std::sync::Arc;

/// One virtualized row's GPU-ready text/icons.
#[derive(Clone)]
pub(crate) struct DetailsRowSlot {
    /// Visible-list index (`start_idx..end_idx`).
    pub row: usize,
    pub entry_idx: usize,
    /// Hash of everything that forces a re-shape (text, font, colors baked in runs).
    pub shape_key: u64,
    pub selected: bool,
    pub hovered: bool,
    pub flashing: bool,
    pub odd: bool,
    pub dimmed: bool,
    /// Full name line (not pre-truncated). Clip at paint.
    pub name_s: ShapedLine,
    pub meta_cols: Vec<DetailsRingMeta>,
    pub icon_tex: Option<Arc<gpui::RenderImage>>,
    pub fallback_icon_path: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) struct DetailsRingMeta {
    pub column: DetailsColumn,
    pub line: ShapedLine,
}

/// Ring of row slots: capacity ≈ viewport rows (+ overscan). Index `row % cap`.
#[derive(Clone, Default)]
pub(crate) struct DetailsRowRing {
    slots: Vec<Option<DetailsRowSlot>>,
}

impl DetailsRowRing {
    /// Ensure at least `n` slots (typically visible count + small overscan).
    pub fn ensure_capacity(&mut self, n: usize) {
        let n = n.max(1);
        if self.slots.len() < n {
            self.slots.resize_with(n, || None);
        }
    }

    fn index(&self, row: usize) -> usize {
        debug_assert!(!self.slots.is_empty());
        row % self.slots.len()
    }

    /// Sync one row into the ring. Runs `fill` only on miss.
    pub fn sync_row(
        &mut self,
        row: usize,
        shape_key: u64,
        fill: impl FnOnce() -> DetailsRowSlot,
    ) -> &DetailsRowSlot {
        if self.slots.is_empty() {
            self.ensure_capacity(1);
        }
        let ix = self.index(row);
        let hit = matches!(
            &self.slots[ix],
            Some(s) if s.row == row && s.shape_key == shape_key
        );
        if hit {
            crate::perf::count("details.ring.hit");
        } else {
            crate::perf::count("details.ring.miss");
            self.slots[ix] = Some(fill());
        }
        self.slots[ix].as_ref().expect("slot just filled")
    }

    /// Update paint-only flags without reshaping (hover/stripe can change every frame).
    pub fn touch_flags(
        &mut self,
        row: usize,
        shape_key: u64,
        selected: bool,
        hovered: bool,
        flashing: bool,
        odd: bool,
        dimmed: bool,
    ) {
        if self.slots.is_empty() {
            return;
        }
        let ix = self.index(row);
        if let Some(s) = self.slots[ix].as_mut() {
            if s.row == row && s.shape_key == shape_key {
                s.selected = selected;
                s.hovered = hovered;
                s.flashing = flashing;
                s.odd = odd;
                s.dimmed = dimmed;
            }
        }
    }

    pub fn get(&self, row: usize) -> Option<&DetailsRowSlot> {
        if self.slots.is_empty() {
            return None;
        }
        let ix = self.index(row);
        self.slots[ix].as_ref().filter(|s| s.row == row)
    }

    /// Collect a dense list for one paint pass (clones Arc layouts inside ShapedLine).
    pub fn collect_rows(&self, start: usize, end: usize) -> Vec<DetailsRowSlot> {
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        for row in start..end {
            if let Some(s) = self.get(row) {
                out.push(s.clone());
            }
        }
        out
    }
}

/// Hash inputs that force re-shape. Width is intentionally excluded: names are
/// clipped at paint, not re-split when the column shrinks.
pub(crate) fn details_row_shape_key(
    entry_idx: usize,
    name: &str,
    display_size: &str,
    display_date: &str,
    display_type: &str,
    kind_disc: u8,
    extension: Option<&str>,
    selected: bool,
    dimmed: bool,
    font_size: Pixels,
    name_color_bits: u32,
    meta_color_bits: u32,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entry_idx.hash(&mut h);
    name.hash(&mut h);
    display_size.hash(&mut h);
    display_date.hash(&mut h);
    display_type.hash(&mut h);
    kind_disc.hash(&mut h);
    extension.hash(&mut h);
    selected.hash(&mut h);
    dimmed.hash(&mut h);
    f32::from(font_size).to_bits().hash(&mut h);
    name_color_bits.hash(&mut h);
    meta_color_bits.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_line() -> ShapedLine {
        ShapedLine::default()
    }

    fn slot(row: usize, key: u64) -> DetailsRowSlot {
        DetailsRowSlot {
            row,
            entry_idx: row,
            shape_key: key,
            selected: false,
            hovered: false,
            flashing: false,
            odd: row % 2 == 1,
            dimmed: false,
            name_s: dummy_line(),
            meta_cols: Vec::new(),
            icon_tex: None,
            fallback_icon_path: None,
        }
    }

    #[test]
    fn ring_hits_same_row_key_and_misses_on_change() {
        let mut ring = DetailsRowRing::default();
        ring.ensure_capacity(8);
        let _ = ring.sync_row(3, 10, || slot(3, 10));
        let _ = ring.sync_row(3, 10, || slot(3, 10)); // hit
        let _ = ring.sync_row(3, 11, || slot(3, 11)); // miss
        assert_eq!(ring.get(3).map(|s| s.shape_key), Some(11));
    }

    #[test]
    fn ring_wraps_and_evicts_old_row() {
        let mut ring = DetailsRowRing::default();
        ring.ensure_capacity(4);
        let _ = ring.sync_row(0, 1, || slot(0, 1));
        let _ = ring.sync_row(4, 2, || slot(4, 2)); // same index as 0
        assert!(ring.get(0).is_none());
        assert_eq!(ring.get(4).map(|s| s.row), Some(4));
    }
}
