//! Polar layout shared by paint, labels, keyboard and hit testing. Every
//! option gets a sector: rings are added outward instead of paginating.
use std::f32::consts::{FRAC_PI_2, TAU};

/// Center disc radius (base px, before window-fit scaling).
pub(crate) const HUB: f32 = 52.0;
/// Radial thickness of one ring of sectors, gap included.
pub(crate) const BAND: f32 = 50.0;
/// Space between neighbouring sectors and between rings.
pub(crate) const GAP: f32 = 3.0;
/// Arc length a sector needs at mid-radius for a readable label.
const MIN_ARC: f32 = 56.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ring {
    pub start: usize,
    pub len: usize,
    pub inner: f32,
    pub outer: f32,
}

impl Ring {
    pub(crate) fn mid(&self) -> f32 {
        (self.inner + self.outer) * 0.5
    }
    fn contains(&self, index: usize) -> bool {
        (self.start..self.start + self.len).contains(&index)
    }
}

fn capacity(ring: usize) -> usize {
    let mid = HUB + BAND * (ring as f32 + 0.5);
    ((TAU * mid / MIN_ARC) as usize).max(1)
}

/// Fewest rings that hold `count`; options are shared proportionally to each
/// ring's capacity so an outer ring never ends up with one lonely sector.
/// The first (most common) formats land on the inner, easiest-to-hit ring.
pub(crate) fn rings(count: usize) -> Vec<Ring> {
    let mut caps = Vec::new();
    while caps.iter().sum::<usize>() < count {
        caps.push(capacity(caps.len()));
    }
    let total: usize = caps.iter().sum();
    let mut lens: Vec<usize> = caps.iter().map(|cap| count * cap / total.max(1)).collect();
    let mut rest = count - lens.iter().sum::<usize>();
    while rest > 0 {
        for (len, cap) in lens.iter_mut().zip(&caps) {
            if rest > 0 && *len < *cap {
                *len += 1;
                rest -= 1;
            }
        }
    }
    let mut start = 0;
    lens.into_iter()
        .enumerate()
        .map(|(k, len)| {
            let ring = Ring {
                start,
                len,
                inner: HUB + BAND * k as f32,
                outer: HUB + BAND * (k + 1) as f32 - GAP,
            };
            start += len;
            ring
        })
        .collect()
}

/// Outer radius of the whole wheel in base px.
pub(crate) fn radius(rings: &[Ring]) -> f32 {
    rings.last().map_or(HUB + BAND, |ring| ring.outer + GAP)
}

pub(crate) fn angle(index: usize, count: usize) -> f32 {
    -FRAC_PI_2 + TAU * index as f32 / count.max(1) as f32
}

pub(crate) fn ring_of(rings: &[Ring], index: usize) -> Option<&Ring> {
    rings.iter().find(|ring| ring.contains(index))
}

/// Center angle of option `index`, whichever ring holds it.
pub(crate) fn option_angle(rings: &[Ring], index: usize) -> f32 {
    ring_of(rings, index).map_or(-FRAC_PI_2, |ring| angle(index - ring.start, ring.len))
}

/// `x`/`y` are relative to the wheel center, in base px.
pub(crate) fn hit(x: f32, y: f32, rings: &[Ring]) -> Option<usize> {
    let distance = x.hypot(y);
    let ring = rings
        .iter()
        .find(|ring| distance >= ring.inner && distance <= ring.outer)?;
    let step = TAU / ring.len as f32;
    let t = (y.atan2(x) + FRAC_PI_2 + step * 0.5).rem_euclid(TAU);
    let within = t.rem_euclid(step) * distance;
    if ring.len > 1 && (within < GAP * 0.5 || step * distance - within < GAP * 0.5) {
        return None;
    }
    Some(ring.start + ((t / step) as usize).min(ring.len - 1))
}

/// Rotate within the current ring (clockwise for positive `delta`).
pub(crate) fn rotate(rings: &[Ring], index: usize, delta: isize) -> usize {
    let Some(ring) = ring_of(rings, index) else {
        return index;
    };
    let local = (index - ring.start) as isize + delta;
    ring.start + local.rem_euclid(ring.len as isize) as usize
}

/// Jump to the neighbouring ring (outward for positive `delta`), keeping the
/// pointer direction: the sector under the same angle is chosen.
pub(crate) fn hop(rings: &[Ring], index: usize, delta: isize) -> usize {
    let Some(current) = rings.iter().position(|ring| ring.contains(index)) else {
        return index;
    };
    let target = (current as isize + delta).clamp(0, rings.len() as isize - 1) as usize;
    if target == current {
        return index;
    }
    let ring = &rings[target];
    let step = TAU / ring.len as f32;
    let t = (option_angle(rings, index) + FRAC_PI_2 + step * 0.5).rem_euclid(TAU);
    ring.start + ((t / step) as usize).min(ring.len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_count_fits_without_pages_and_inner_rings_fill_first() {
        for count in 1..=120 {
            let layout = rings(count);
            assert_eq!(layout.iter().map(|r| r.len).sum::<usize>(), count);
            assert!(layout.iter().all(|r| r.len >= 1));
            for (k, ring) in layout.iter().enumerate() {
                assert!(ring.len <= capacity(k), "{count}: ring {k}");
            }
            for index in 0..count {
                let ring = ring_of(&layout, index).unwrap();
                let a = option_angle(&layout, index);
                let (x, y) = (ring.mid() * a.cos(), ring.mid() * a.sin());
                assert_eq!(hit(x, y, &layout), Some(index), "{count}: {index}");
            }
        }
        assert_eq!(rings(8).len(), 1);
        assert_eq!(rings(9).len(), 2);
    }

    #[test]
    fn sectors_follow_the_reference_clockwise_from_the_top() {
        let layout = rings(4);
        let r = layout[0].mid();
        assert_eq!(hit(0., -r, &layout), Some(0));
        assert_eq!(hit(r, 0., &layout), Some(1));
        assert_eq!(hit(0., r, &layout), Some(2));
        assert_eq!(hit(-r, 0., &layout), Some(3));
        assert_eq!(angle(0, 4), -FRAC_PI_2);
    }

    #[test]
    fn hub_outside_and_gutters_are_not_conversion_targets() {
        let layout = rings(4);
        let r = layout[0].mid();
        assert_eq!(hit(0., 0., &layout), None);
        assert_eq!(hit(radius(&layout) + 1., 0., &layout), None);
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(hit(d, -d, &layout), None);
        assert_eq!(hit(0., -r, &[]), None);
    }

    #[test]
    fn keyboard_rotates_within_a_ring_and_hops_by_direction() {
        let layout = rings(12);
        let (inner, outer) = (layout[0], layout[1]);
        assert_eq!(rotate(&layout, 0, -1), inner.len - 1);
        assert_eq!(rotate(&layout, inner.len - 1, 1), 0);
        assert_eq!(hop(&layout, 0, 1), outer.start);
        assert_eq!(hop(&layout, outer.start, -1), 0);
        assert_eq!(hop(&layout, 0, -1), 0);
        assert_eq!(hop(&layout, outer.start, 1), outer.start);
    }
}
