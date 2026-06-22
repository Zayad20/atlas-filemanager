//! Shared motion language for Atlas UI.
//!
//! RAD Debugger's UI feels consistent because most animated surfaces use the
//! same small set of rates and eased values. Keep those knobs centralized here
//! so panes, overlays, menus, and inputs move as one system instead of each
//! component inventing its own timing.

use gpui::{point, px};
use gpui::{Animation, Div, Pixels, SharedString, Styled};
use std::time::Duration;

// "demo snappy" profile — ~35% shorter than baseline for video feel.
// Revert to 120/150/170/220/280 after recording (git checkout this file).
pub const MICRO_MS: u64 = 80;
pub const PANEL_MS: u64 = 100;
pub const TAB_MS: u64 = 70;
pub const PANE_MS: u64 = 90;
pub const GLOW_MS: u64 = 90;

pub const PALETTE_GROW_FROM: f32 = 0.80;

// Item appear (rows fading/sliding in on tab create / navigation).
pub const ITEM_APPEAR_MS: u64 = 90;
pub const ITEM_STAGGER_MS: u64 = 4;
pub const ITEM_STAGGER_CAP: usize = 6;

pub fn ease_out_quart(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

/// A spring, as the analytic step response of a second-order system — the same
/// physics alt-drag/iOS use for "alive" motion, but in closed form: no per-frame
/// integrator, so it's frame-rate independent and can't drift or blow up.
///
/// `value(t)` returns the spring's position at `t` *seconds* for a unit step (0→1).
/// Underdamped (`zeta` < 1) overshoots once and settles — the momentum feel. The
/// output exceeds 1.0 on the overshoot, so it must drive a `with_animation` *closure*
/// (unchecked), NOT `with_easing` (gpui `debug_assert!`s easings stay in [0,1]).
#[derive(Clone, Copy)]
pub struct Spring {
    /// Natural angular frequency (rad/s). Higher = snappier.
    pub omega: f32,
    /// Damping ratio: <1 overshoots, 1 critical (no overshoot), >1 sluggish.
    pub zeta: f32,
}

impl Spring {
    pub fn value(self, t: f32) -> f32 {
        let (w, z) = (self.omega, self.zeta);
        if t <= 0.0 {
            return 0.0;
        }
        if z < 1.0 {
            // Underdamped: decaying cosine envelope → one soft overshoot.
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-z * w * t).exp();
            1.0 - e * ((wd * t).cos() + (z * w / wd) * (wd * t).sin())
        } else {
            // Critically damped: smooth decel, no overshoot (good for exits).
            let e = (-w * t).exp();
            (w * t).mul_add(-e, 1.0 - e)
        }
    }

    /// Seconds until the response has decayed within `eps` of rest. Drives the
    /// `with_animation` timeline length so the tween ends exactly as the spring stills.
    pub fn settle_secs(self, eps: f32) -> f32 {
        -eps.ln() / (self.zeta * self.omega)
    }

    pub fn settle(self) -> Duration {
        Duration::from_secs_f32(self.settle_secs(0.02))
    }
}

/// Pane intro spring: snappy with a gentle ~8% overshoot. Used for new-tab / split
/// entrance so panes glide in with momentum and settle, instead of a flat tween.
pub const PANE_SPRING: Spring = Spring {
    omega: 26.0,
    zeta: 0.62,
};

/// Total animation duration for the appear of a row at (visible) index `row`,
/// including its stagger delay. Each row is its own `with_animation`; the delay is
/// folded into the duration and unwound in [`item_appear`] so rows cascade.
pub fn item_appear_total_ms(row: usize) -> u64 {
    ITEM_APPEAR_MS + (row.min(ITEM_STAGGER_CAP) as u64) * ITEM_STAGGER_MS
}

/// Apply the staggered appear transform to a row. `t` is the `with_animation`
/// progress (0→1) over `item_appear_total_ms(row)`; the leading `delay` slice keeps
/// the row hidden, then it fades + slides up into place (GPU: opacity + paint_offset).
pub fn item_appear(this: Div, t: f32, row: usize) -> Div {
    let total = item_appear_total_ms(row) as f32;
    let delay = (row.min(ITEM_STAGGER_CAP) as f32) * ITEM_STAGGER_MS as f32;
    let local = (t.mul_add(total, -delay) / ITEM_APPEAR_MS as f32).clamp(0.0, 1.0);
    let e = ease_out_quart(local);
    this.opacity(0.88 + 0.12 * e)
        .paint_offset(point(px(0.0), px(4.0 * (1.0 - e))))
}

pub fn animation(ms: u64) -> Animation {
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quart)
}

/// Linear timeline over a `Duration` — for animations whose curve lives in the
/// `with_animation` closure (e.g. a [`Spring`]), so the easing gpui sees stays in [0,1].
pub fn linear_anim(d: Duration) -> Animation {
    Animation::new(d).with_easing(|t| t)
}

pub fn keyed(
    prefix: &str,
    id: impl std::fmt::Display,
    generation: impl std::fmt::Debug,
) -> SharedString {
    SharedString::from(format!("{prefix}-{id}-{generation:?}"))
}

pub fn panel_pop(this: Div, openness: f32, width: f32) -> Div {
    let grow = (1.0 - PALETTE_GROW_FROM).mul_add(openness, PALETTE_GROW_FROM);
    this.opacity(openness).w(gpui::px(width * grow))
}

pub fn pane_intro(this: Div, openness: f32, offset: Pixels, vertical: bool) -> Div {
    let distance = offset * (1.0 - openness);
    let this = this.opacity(openness);
    if vertical {
        this.mt(distance)
    } else {
        this.ml(distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_step_response() {
        let s = PANE_SPRING;
        assert_eq!(s.value(0.0), 0.0, "starts at rest position 0");
        // Underdamped → overshoots past 1.0 somewhere before settling.
        let peak = (0..200)
            .map(|i| s.value(i as f32 * 0.001))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.0, "underdamped spring overshoots, got {peak}");
        // Settled within eps of 1.0 by its own settle time.
        let v = s.value(s.settle_secs(0.02));
        assert!((v - 1.0).abs() < 0.05, "settles to ~1.0, got {v}");
    }
}
