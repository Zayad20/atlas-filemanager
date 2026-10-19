//! Time-based line-wheel easing. Pixel input and programmatic scrolling bypass it.

use std::time::Instant;

#[derive(Default, Debug)]
pub(super) struct WheelScroll {
    target: Option<[f32; 2]>,
    sampled_at: Option<Instant>,
}

impl WheelScroll {
    pub(super) fn cancel(&mut self) {
        self.target = None;
        self.sampled_at = None;
    }

    pub(super) fn is_active(&self) -> bool {
        self.target.is_some()
    }

    pub(super) fn push(&mut self, offset: [f32; 2], delta: [f32; 2], max: [f32; 2], now: Instant) {
        let mut target = self.target.unwrap_or(offset);
        for axis in 0..2 {
            // Reverse from the visible position, without first finishing the old glide.
            let remaining = target[axis] - offset[axis];
            if delta[axis] * remaining < 0.0 {
                target[axis] = offset[axis];
            }
            target[axis] = (target[axis] + delta[axis]).clamp(-max[axis].max(0.0), 0.0);
        }
        self.target = Some(target);
        self.sampled_at.get_or_insert(now);
    }

    pub(super) fn sample(
        &mut self,
        offset: [f32; 2],
        max: [f32; 2],
        ease: f32,
        now: Instant,
    ) -> Option<[f32; 2]> {
        let mut target = self.target?;
        for axis in 0..2 {
            target[axis] = target[axis].clamp(-max[axis].max(0.0), 0.0);
        }
        self.target = Some(target);
        let elapsed = now
            .saturating_duration_since(self.sampled_at.unwrap_or(now))
            .as_secs_f32();
        self.sampled_at = Some(now);
        // A suspended/occluded window resumes from its visible position. Time
        // spent asleep is not an animation frame and must not consume the glide.
        let elapsed = if elapsed > 0.25 || elapsed == 0.0 { 0.0 } else { elapsed.max(0.001) };
        // `ease` retains its public meaning at 60 Hz. The exponential's time
        // constant makes extra layout passes and faster displays travel identically.
        let alpha = if ease <= 0.0 || ease >= 1.0 {
            1.0
        } else {
            let lambda = -(1.0 - ease).log2() * 60.0;
            1.0 - (-lambda * elapsed).exp2()
        };
        if (0..2).all(|axis| (target[axis] - offset[axis]).abs() < 0.5) {
            self.cancel();
            return Some(target);
        }
        let next = std::array::from_fn(|axis| offset[axis] + (target[axis] - offset[axis]) * alpha);
        if alpha >= 1.0 {
            self.cancel();
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn equal_elapsed_time_gives_equal_travel_at_any_refresh_rate() {
        let now = Instant::now();
        let mut results = Vec::new();
        for hz in [60, 120, 144, 240] {
            let mut scroll = WheelScroll::default();
            let mut offset = [0.0; 2];
            scroll.push(offset, [0.0, -240.0], [1000.0; 2], now);
            for frame in 1..=hz / 6 {
                offset = scroll
                    .sample(
                        offset,
                        [1000.0; 2],
                        0.32,
                        now + Duration::from_secs_f64(f64::from(frame) / f64::from(hz)),
                    )
                    .unwrap();
            }
            results.push(offset[1]);
        }
        assert!(results.iter().all(|y| (*y - results[0]).abs() < 0.001));
    }

    #[test]
    fn reversing_one_axis_keeps_the_other_axis_target() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        scroll.push([-100.0; 2], [-120.0; 2], [1000.0; 2], now);
        scroll.push([-110.0; 2], [30.0, 0.0], [1000.0; 2], now);
        assert_eq!(scroll.target, Some([-80.0, -220.0]));
    }

    #[test]
    fn boundary_input_does_not_build_up_a_hidden_backlog() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        for _ in 0..20 {
            scroll.push([0.0, -100.0], [0.0, -120.0], [100.0; 2], now);
        }
        scroll.push([0.0, -100.0], [0.0, 30.0], [100.0; 2], now);
        assert_eq!(scroll.target, Some([0.0, -70.0]));
    }

    #[test]
    fn content_shrink_clamps_and_settles_without_more_frames() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        scroll.push([0.0; 2], [0.0, -100.0], [1000.0; 2], now);
        assert_eq!(scroll.sample([0.0; 2], [0.0; 2], 0.32, now), Some([0.0; 2]));
        assert!(!scroll.is_active());
    }

    #[test]
    fn repeated_layout_at_the_same_time_does_not_advance_scroll() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        scroll.push([0.0; 2], [0.0, -100.0], [1000.0; 2], now);
        let time = now + Duration::from_millis(16);
        let next = scroll.sample([0.0; 2], [1000.0; 2], 0.32, time).unwrap();
        assert_eq!(scroll.sample(next, [1000.0; 2], 0.32, time), Some(next));
    }

    #[test]
    fn cancellation_and_instant_motion_do_not_replay_the_target() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        scroll.push([0.0; 2], [0.0, -100.0], [1000.0; 2], now);
        scroll.cancel();
        assert_eq!(scroll.sample([0.0; 2], [1000.0; 2], 0.32, now), None);
        scroll.push([0.0; 2], [0.0, -100.0], [1000.0; 2], now);
        assert_eq!(
            scroll.sample([0.0; 2], [1000.0; 2], 0.0, now),
            Some([0.0, -100.0])
        );
        assert!(!scroll.is_active());
    }

    #[test]
    fn resuming_after_idle_has_zero_delta_then_continues() {
        let now = Instant::now();
        let mut scroll = WheelScroll::default();
        scroll.push([0.0; 2], [0.0, -240.0], [1000.0; 2], now);
        let resumed = now + Duration::from_secs(10);
        assert_eq!(
            scroll.sample([0.0; 2], [1000.0; 2], 0.32, resumed),
            Some([0.0; 2])
        );
        let next = scroll
            .sample(
                [0.0; 2],
                [1000.0; 2],
                0.32,
                resumed + Duration::from_millis(16),
            )
            .unwrap();
        assert!(next[1] < 0.0 && next[1] > -240.0);
    }

    #[test]
    fn positive_submillisecond_ticks_use_the_minimum_timestep() {
        let now = Instant::now();
        let sample = |dt| {
            let mut scroll = WheelScroll::default();
            scroll.push([0.0; 2], [0.0, -240.0], [1000.0; 2], now);
            scroll.sample([0.0; 2], [1000.0; 2], 0.32, now + dt).unwrap()
        };
        assert_eq!(sample(Duration::from_micros(100)), sample(Duration::from_millis(1)));
        assert_eq!(sample(Duration::ZERO), [0.0; 2]);
    }
}
