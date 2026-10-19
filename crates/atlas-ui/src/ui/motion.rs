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
// Tabs need enough time for their layout slot to collapse while neighbouring
// tabs visibly settle into the released space. Shorter values read as a snap.
pub const TAB_MS: u64 = 150;
pub const PANE_MS: u64 = 90;
pub const GLOW_MS: u64 = 90;
// Layout motion has its own budget: entrances explain the new split, while
// exits release space sooner. Neither curve overshoots the pane's neighbours.
pub const SPLIT_OPEN_MS: u64 = 240;
pub const SPLIT_CLOSE_MS: u64 = 140;

pub const PALETTE_GROW_FROM: f32 = 0.80;

// Item appear (rows fading/sliding in on tab create / navigation).
pub const ITEM_APPEAR_MS: u64 = 90;
pub const ITEM_STAGGER_MS: u64 = 4;
pub const ITEM_STAGGER_CAP: usize = 6;

pub fn ease_out_quart(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

/// Motion presets expressed as frequency and damping; GPUI CE owns the solver.
#[derive(Clone, Copy)]
pub struct Spring {
    /// Natural angular frequency (rad/s). Higher = snappier.
    pub omega: f32,
    /// Damping ratio: <1 overshoots, 1 critical (no overshoot), >1 sluggish.
    pub zeta: f32,
}

impl Spring {
    pub const fn config(self) -> gpui::SpringConfig {
        gpui::spring(self.omega * self.omega, 2.0 * self.zeta * self.omega, 1.0)
    }

    pub fn value(self, t: f32) -> f32 {
        self.config()
            .step(gpui::SpringState::default(), 1.0, t.max(0.0))
            .position
    }

    pub fn settle_secs(self, eps: f32) -> f32 {
        self.config()
            .settle_time(gpui::SpringState::default(), 1.0, eps)
            .as_secs_f32()
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

/// Table neighbours settle without bouncing into adjacent hit targets.
pub const TABLE_SPRING: Spring = Spring {
    omega: 32.0,
    zeta: 1.0,
};

/// Rows carry more visual mass than column dividers; they settle more gently.
pub const ROW_REORDER_SPRING: Spring = Spring {
    omega: 32.0,
    zeta: 1.0,
};
pub const ROW_REORDER_FADE_MS: f32 = 120.0;

pub const MENU_REVEAL_MS: u64 = 140;
pub const MENU_STAGGER_MS: u64 = 9;
pub const MENU_STAGGER_CAP: usize = 6;

/// One bounded timeline for the whole menu. Filtering and async shell results
/// do not restart it; every row remains immediately usable while revealing.
pub fn menu_row_progress(elapsed_ms: f32, row: usize) -> f32 {
    let delay = row.min(MENU_STAGGER_CAP) as f32 * MENU_STAGGER_MS as f32;
    ease_out_quart(((elapsed_ms - delay) / MENU_REVEAL_MS as f32).clamp(0.0, 1.0))
}

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
    Animation::new(gpui::Motion::new(Duration::from_millis(ms)).with_easing(ease_out_quart))
}

/// Linear timeline over a `Duration` — for animations whose curve lives in the
/// `with_animation` closure (e.g. a [`Spring`]), so the easing gpui sees stays in [0,1].
pub fn linear_anim(d: Duration) -> Animation {
    Animation::new(gpui::Motion::new(d))
}

/// Retain a native spring across renders and retarget it from its current value.
/// Completion comes from the solver, so a panel stays mounted for its whole exit.
pub fn panel_transition(
    window: &mut gpui::Window,
    cx: &mut gpui::App,
    key: &'static str,
    initial: f32,
    target: f32,
    config: gpui::SpringConfig,
    instant: bool,
) -> gpui::AnimatedSample<f32> {
    let state = window.use_keyed_state(key, cx, |_, _| gpui::Animated::new(initial, config));
    let sample = state.update(cx, |state, _| {
        sample_panel_transition(state, target, config, instant, std::time::Instant::now())
    });
    if sample.is_active {
        window.request_animation_frame();
    }
    sample
}

fn sample_panel_transition(
    state: &mut gpui::Animated<f32>,
    target: f32,
    config: gpui::SpringConfig,
    instant: bool,
    now: std::time::Instant,
) -> gpui::AnimatedSample<f32> {
    if instant {
        state.jump_to(target);
    } else {
        state.set(target, config, now);
    }
    state.sample(now)
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
    fn menu_cascade_has_one_bounded_timeline() {
        assert!(menu_row_progress(45.0, 0) > menu_row_progress(45.0, 4));
        let done = (MENU_REVEAL_MS + MENU_STAGGER_MS * MENU_STAGGER_CAP as u64) as f32;
        for row in 0..100 {
            assert_eq!(menu_row_progress(done, row), 1.0);
            assert_eq!(menu_row_progress(f32::MAX, row), 1.0);
        }
    }

    #[test]
    fn hover_motion_uses_transparency_without_replacing_authored_materials() {
        let mut bare = gpui::div().motion_feedback();
        assert_eq!(
            bare.style().background,
            Some(gpui::transparent_black().into())
        );
        let mut filled = gpui::div().bg(gpui::white()).motion_feedback();
        assert_eq!(filled.style().background, Some(gpui::white().into()));
    }

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

    #[test]
    fn panel_can_reverse_mid_flight_and_finishes_its_exit() {
        let now = std::time::Instant::now();
        let config = gpui::spring(1100.0, 66.0, 1.0);
        let mut state = gpui::Animated::new(0.0, config);
        sample_panel_transition(&mut state, 1.0, config, false, now);
        let interrupted_at = now + Duration::from_millis(80);
        let before = state.sample(interrupted_at).value;
        assert!(before > 0.0 && before < 1.0);
        let closing = sample_panel_transition(&mut state, 0.0, config, false, interrupted_at);
        assert!(
            (closing.value - before).abs() < 0.0001,
            "retargeting must not jump"
        );
        assert!(
            closing.is_active,
            "the inspector must stay mounted during its exit"
        );
        let still_closing = sample_panel_transition(
            &mut state,
            0.0,
            config,
            false,
            interrupted_at + Duration::from_millis(100),
        );
        assert!(
            still_closing.is_active,
            "the old 100 ms teardown cuts off this spring"
        );
        assert!(still_closing.value < before);
        let finished =
            sample_panel_transition(&mut state, 0.0, config, false, now + Duration::from_secs(2));
        assert!(!finished.is_active);
        assert_eq!(finished.value, 0.0);
    }

    #[test]
    fn disabling_motion_or_resizing_settles_a_running_panel_immediately() {
        let now = std::time::Instant::now();
        let config = gpui::spring(900.0, 60.0, 1.0);
        let mut state = gpui::Animated::new(260.0, config);
        sample_panel_transition(&mut state, 56.0, config, false, now);
        let now = now + Duration::from_millis(40);
        assert!(state.sample(now).is_active);
        let instant = sample_panel_transition(&mut state, 56.0, config, true, now);
        assert_eq!(instant.value, 56.0);
        assert!(!instant.is_active);
        let unchanged = sample_panel_transition(&mut state, 56.0, config, false, now);
        assert_eq!(unchanged.value, 56.0);
        assert!(
            !unchanged.is_active,
            "reenabling motion must not replay the old target"
        );
    }
}

pub trait MotionFeedbackExt: gpui::InteractiveElement + gpui::Styled + Sized {
    #[track_caller]
    fn motion_feedback(mut self) -> Self {
        // An absent background is transparent. GPUI's default Fill is opaque
        // black, so author the transparent endpoint before asking it to animate.
        if self.style().background.is_none() {
            self.style().background = Some(gpui::transparent_black().into());
        }
        self.transitions(|t| {
            t.bg(gpui::millis(120))
                .text_color(gpui::millis(120))
                .opacity(gpui::millis(120))
        })
    }
}

impl<T: gpui::InteractiveElement + gpui::Styled + Sized> MotionFeedbackExt for T {}
