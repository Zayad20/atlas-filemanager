//! Shared edge auto-scroll physics.
//!
//! Interaction owners decide *when* scrolling is allowed (drag proximity versus
//! rubber-band directional intent). This module owns the common geometry,
//! time-based velocity, and `ScrollHandle` mutation so all owners feel alike.

use gpui::{point, px, Bounds, Pixels, Point, ScrollHandle};

pub(crate) const EDGE_ZONE: f32 = 44.0;
const MIN_SPEED: f32 = 120.0;
const MAX_SPEED: f32 = 1_100.0;
const MAX_FRAME_SECONDS: f32 = 1.0 / 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeDirection {
    Up,
    Down,
    /// Horizontal scrollers (column-major List).
    Left,
    Right,
}

impl EdgeDirection {
    pub(crate) fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// Pointer travel towards this edge between two points (positive = outward).
    pub(crate) fn outward(self, from: Point<Pixels>, to: Point<Pixels>) -> f32 {
        match self {
            Self::Up => f32::from(from.y - to.y),
            Self::Down => f32::from(to.y - from.y),
            Self::Left => f32::from(from.x - to.x),
            Self::Right => f32::from(to.x - from.x),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgeActivation {
    pub(crate) direction: EdgeDirection,
    /// 0 at the inside edge of the activation zone, 1 at the viewport edge.
    pub(crate) strength: f32,
}

/// Returns the active vertical edge only while the pointer remains inside the
/// viewport horizontally. Keeping the pointer outside a container never scrolls it.
pub(crate) fn activation_at(bounds: Bounds<Pixels>, pos: Point<Pixels>) -> Option<EdgeActivation> {
    let x = f32::from(pos.x);
    let left = f32::from(bounds.left());
    let right = f32::from(bounds.right());
    if x < left || x > right {
        return None;
    }

    let y = f32::from(pos.y);
    let top = f32::from(bounds.top());
    let bottom = f32::from(bounds.bottom());
    if y < top + EDGE_ZONE {
        Some(EdgeActivation {
            direction: EdgeDirection::Up,
            strength: ((top + EDGE_ZONE - y) / EDGE_ZONE).clamp(0.0, 1.0),
        })
    } else if y > bottom - EDGE_ZONE {
        Some(EdgeActivation {
            direction: EdgeDirection::Down,
            strength: ((y - (bottom - EDGE_ZONE)) / EDGE_ZONE).clamp(0.0, 1.0),
        })
    } else {
        None
    }
}

/// Horizontal counterpart of [`activation_at`]: active left/right edge while the
/// pointer stays inside the viewport vertically.
pub(crate) fn activation_at_horizontal(
    bounds: Bounds<Pixels>,
    pos: Point<Pixels>,
) -> Option<EdgeActivation> {
    let y = f32::from(pos.y);
    if y < f32::from(bounds.top()) || y > f32::from(bounds.bottom()) {
        return None;
    }
    let x = f32::from(pos.x);
    let left = f32::from(bounds.left());
    let right = f32::from(bounds.right());
    if x < left + EDGE_ZONE {
        Some(EdgeActivation {
            direction: EdgeDirection::Left,
            strength: ((left + EDGE_ZONE - x) / EDGE_ZONE).clamp(0.0, 1.0),
        })
    } else if x > right - EDGE_ZONE {
        Some(EdgeActivation {
            direction: EdgeDirection::Right,
            strength: ((x - (right - EDGE_ZONE)) / EDGE_ZONE).clamp(0.0, 1.0),
        })
    } else {
        None
    }
}

pub(crate) fn velocity(strength: f32) -> f32 {
    let t = strength.clamp(0.0, 1.0);
    // Smoothstep avoids the perceptible speed kink at the activation boundary.
    let eased = t * t * (3.0 - 2.0 * t);
    MIN_SPEED + (MAX_SPEED - MIN_SPEED) * eased
}

pub(crate) fn next_offset(
    current: f32,
    max_offset: f32,
    direction: EdgeDirection,
    strength: f32,
    elapsed_seconds: f32,
) -> f32 {
    let dt = elapsed_seconds.clamp(0.0, MAX_FRAME_SECONDS);
    let distance = velocity(strength) * dt;
    match direction {
        EdgeDirection::Up | EdgeDirection::Left => (current + distance).min(0.0),
        EdgeDirection::Down | EdgeDirection::Right => {
            (current - distance).max(-max_offset.max(0.0))
        }
    }
}

/// Advance one GPUI scroll handle according to a previously resolved activation.
/// Returns true only when the handle actually moved.
pub(crate) fn tick_handle(
    handle: &ScrollHandle,
    activation: EdgeActivation,
    elapsed_seconds: f32,
) -> bool {
    let offset = handle.offset();
    let horizontal = activation.direction.is_horizontal();
    let (current, max) = if horizontal {
        (f32::from(offset.x), f32::from(handle.max_offset().x))
    } else {
        (f32::from(offset.y), f32::from(handle.max_offset().y))
    };
    let next = next_offset(
        current,
        max,
        activation.direction,
        activation.strength,
        elapsed_seconds,
    );
    if (next - current).abs() < 0.01 {
        return false;
    }
    handle.set_offset(if horizontal {
        point(px(next), offset.y)
    } else {
        point(offset.x, px(next))
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_is_proportional_and_requires_horizontal_containment() {
        let bounds = Bounds::from_corners(point(px(10.), px(20.)), point(px(210.), px(220.)));
        assert_eq!(
            activation_at(bounds, point(px(100.), px(198.))).map(|a| a.direction),
            Some(EdgeDirection::Down)
        );
        assert!(activation_at(bounds, point(px(100.), px(100.))).is_none());
        assert!(activation_at(bounds, point(px(9.), px(219.))).is_none());
    }

    #[test]
    fn velocity_increases_smoothly_with_closeness() {
        assert!(velocity(0.0) < velocity(0.5));
        assert!(velocity(0.5) < velocity(1.0));
    }

    #[test]
    fn offset_is_time_based_and_clamped() {
        let first = next_offset(0.0, 500.0, EdgeDirection::Down, 1.0, 1.0 / 60.0);
        let second = next_offset(first, 500.0, EdgeDirection::Down, 1.0, 1.0 / 60.0);
        assert!(first < 0.0 && second < first);
        assert_eq!(
            next_offset(-490.0, 500.0, EdgeDirection::Down, 1.0, 1.0),
            -500.0
        );
        assert_eq!(next_offset(-5.0, 500.0, EdgeDirection::Up, 1.0, 1.0), 0.0);
    }
}
