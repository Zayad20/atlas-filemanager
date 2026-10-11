//! Circular row slot buffer for Details paint.
//!
//! Shape work (`shape_line`) only runs on slot miss. Scroll and split reuse slots
//! whose identity (row + shape_key) still matches. Name truncation is paint-time
//! clip (`ContentMask`), so column resize does not invalidate shaped names.

use crate::app::DetailsColumn;
use gpui::{Font, Pixels, ShapedLine};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

const DETAILS_ROW_TEXT_CACHE_CAPACITY: usize = 8_192;

/// Immutable, GPU-ready text shared by every pane that renders the same row
/// with the same font geometry. Dynamic color/state deliberately lives outside.
pub(crate) struct DetailsRowText {
    /// Full name line (not pre-truncated). Clip at paint.
    pub name_s: ShapedLine,
    pub meta_cols: Vec<DetailsRingMeta>,
}

/// Bounded cross-pane cache for Details shaping. A split clones the directory
/// snapshot, so identical panes can reuse one set of shaped rows.
pub(crate) struct DetailsRowTextCache {
    rows: HashMap<u64, Arc<DetailsRowText>>,
    insertion_order: VecDeque<u64>,
    capacity: usize,
}

impl Default for DetailsRowTextCache {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            insertion_order: VecDeque::new(),
            capacity: DETAILS_ROW_TEXT_CACHE_CAPACITY,
        }
    }
}

impl DetailsRowTextCache {
    pub fn get_or_insert_with(
        &mut self,
        key: u64,
        fill: impl FnOnce() -> DetailsRowText,
    ) -> Arc<DetailsRowText> {
        if let Some(row) = self.rows.get(&key) {
            crate::perf::count("details.text.shared.hit");
            return row.clone();
        }

        crate::perf::count("details.text.shared.miss");
        let row = Arc::new(fill());
        while self.rows.len() >= self.capacity {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            self.rows.remove(&oldest);
        }
        self.rows.insert(key, row.clone());
        self.insertion_order.push_back(key);
        row
    }
}

/// One virtualized row's GPU-ready text/icons.
#[derive(Clone)]
pub(crate) struct DetailsRowSlot {
    /// Visible-list index (`start_idx..end_idx`).
    pub row: usize,
    /// Hash of everything that changes the immutable row payload.
    pub shape_key: u64,
    pub selected: bool,
    pub cursor: bool,
    /// Starts only on an observed selection transition, never merely because a
    /// virtual row was shaped while already selected.
    pub selected_at: Option<Instant>,
    pub hovered: bool,
    pub flashing: bool,
    pub odd: bool,
    pub dimmed: bool,
    /// Heavy immutable payload is shared across panes; paint snapshots only clone
    /// this Arc while dynamic flags remain local to the compact row record.
    pub text: Arc<DetailsRowText>,
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
    // Slots own only compact dynamic state plus Arc handles to immutable text/icons.
    // Paint snapshots clone these small records into one contiguous Vec.
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
        cursor: bool,
        hovered: bool,
        flashing: bool,
        odd: bool,
        dimmed: bool,
    ) {
        if self.slots.is_empty() {
            return;
        }
        let ix = self.index(row);
        let Some(current) = self.slots[ix].as_ref() else {
            return;
        };
        if current.row != row || current.shape_key != shape_key {
            return;
        }
        if current.selected == selected
            && current.cursor == cursor
            && current.hovered == hovered
            && current.flashing == flashing
            && current.odd == odd
            && current.dimmed == dimmed
        {
            return;
        }

        let s = self.slots[ix].as_mut().expect("slot checked above");
        if s.selected != selected {
            s.selected_at = selected.then(Instant::now);
        }
        s.selected = selected;
        s.cursor = cursor;
        s.hovered = hovered;
        s.flashing = flashing;
        s.odd = odd;
        s.dimmed = dimmed;
    }

    pub fn get(&self, row: usize) -> Option<&DetailsRowSlot> {
        if self.slots.is_empty() {
            return None;
        }
        let ix = self.index(row);
        self.slots[ix].as_ref().filter(|s| s.row == row)
    }

    /// Collect a dense, contiguous paint snapshot. Heavy payloads remain shared;
    /// each clone copies only scalars and Arc handles.
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

/// Hash inputs that force re-shape. Width and paint-only state are intentionally
/// excluded: truncation and color are applied while painting.
pub(crate) fn details_row_shape_key(
    entry_idx: usize,
    name: &str,
    display_size: &str,
    display_date: &str,
    display_type: &str,
    kind_disc: u8,
    extension: Option<&str>,
    font: &Font,
    font_size: Pixels,
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
    font.hash(&mut h);
    f32::from(font_size).to_bits().hash(&mut h);
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
            shape_key: key,
            selected_at: None,
            selected: false,
            cursor: false,
            hovered: false,
            flashing: false,
            odd: row % 2 == 1,
            dimmed: false,
            text: Arc::new(DetailsRowText {
                name_s: dummy_line(),
                meta_cols: Vec::new(),
            }),
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

    #[test]
    fn paint_snapshot_keeps_old_flags_and_shared_text() {
        let mut ring = DetailsRowRing::default();
        ring.ensure_capacity(4);
        let _ = ring.sync_row(1, 7, || slot(1, 7));
        let snapshot = ring.collect_rows(1, 2);

        ring.touch_flags(1, 7, true, false, true, false, true, false);

        let current = ring.get(1).expect("ring row");
        assert!(!snapshot[0].selected);
        assert!(current.selected);
        assert!(Arc::ptr_eq(&snapshot[0].text, &current.text));
    }

    #[test]
    fn shared_text_cache_reuses_payload_and_is_bounded() {
        let mut cache = DetailsRowTextCache {
            rows: HashMap::new(),
            insertion_order: VecDeque::new(),
            capacity: 2,
        };
        let first = cache.get_or_insert_with(1, || DetailsRowText {
            name_s: dummy_line(),
            meta_cols: Vec::new(),
        });
        let same = cache.get_or_insert_with(1, || panic!("cache hit must not refill"));
        assert!(Arc::ptr_eq(&first, &same));

        let _ = cache.get_or_insert_with(2, || DetailsRowText {
            name_s: dummy_line(),
            meta_cols: Vec::new(),
        });
        let _ = cache.get_or_insert_with(3, || DetailsRowText {
            name_s: dummy_line(),
            meta_cols: Vec::new(),
        });
        assert!(!cache.rows.contains_key(&1));
        assert_eq!(cache.rows.len(), 2);
    }
}
