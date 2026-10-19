//! Color and circular interpolation math, independent of the UI framework.
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hsv {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub alpha: f32,
}

impl Hsv {
    pub fn from_hsl([hue, saturation, lightness, alpha]: [f32; 4]) -> Self {
        let value = lightness + saturation * lightness.min(1.0 - lightness);
        Self {
            hue: hue.rem_euclid(1.0),
            saturation: if value > 0.0 {
                2.0 * (1.0 - lightness / value)
            } else {
                0.0
            },
            value,
            alpha,
        }
    }

    pub fn to_hsl(self) -> [f32; 4] {
        let lightness = self.value * (1.0 - self.saturation * 0.5);
        let saturation = if lightness > 0.0 && lightness < 1.0 {
            (self.value - lightness) / lightness.min(1.0 - lightness)
        } else {
            0.0
        };
        [
            self.hue.rem_euclid(1.0),
            saturation.clamp(0.0, 1.0),
            lightness.clamp(0.0, 1.0),
            self.alpha,
        ]
    }
}

pub(super) fn shortest_hue_delta(from: f32, to: f32) -> f32 {
    (to - from + 0.5).rem_euclid(1.0) - 0.5
}

#[derive(Clone, Copy, Debug)]
pub(super) struct HueTransition {
    from: f32,
    target: f32,
    started: Instant,
}

impl Default for HueTransition {
    fn default() -> Self {
        Self {
            from: 0.0,
            target: 0.0,
            started: Instant::now(),
        }
    }
}

impl HueTransition {
    pub fn retarget(&mut self, target: f32, now: Instant) {
        let from = self.sample(now).0;
        self.from = from;
        self.target = from + shortest_hue_delta(from, target);
        self.started = now;
    }

    pub fn sample(&self, now: Instant) -> (f32, bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f32() / 0.12).clamp(0.0, 1.0);
        let progress = 1.0 - (1.0 - t).powi(3);
        (
            (self.from + (self.target - self.from) * progress).rem_euclid(1.0),
            t < 1.0 && (self.target - self.from).abs() > 0.0001,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn hue_wrap_uses_two_degrees_instead_of_358() {
        assert!((shortest_hue_delta(359.0 / 360.0, 1.0 / 360.0) * 360.0 - 2.0).abs() < 0.001);
        assert!((shortest_hue_delta(1.0 / 360.0, 359.0 / 360.0) * 360.0 + 2.0).abs() < 0.001);
    }
    #[test]
    fn retargeting_preserves_the_current_position_and_settles() {
        let now = Instant::now();
        let mut transition = HueTransition::default();
        transition.retarget(359.0 / 360.0, now);
        transition.retarget(1.0 / 360.0, now + Duration::from_millis(120));
        assert!(
            (shortest_hue_delta(
                359.0 / 360.0,
                transition.sample(now + Duration::from_millis(120)).0
            ))
            .abs()
                < 0.0001
        );
        let (hue, active) = transition.sample(now + Duration::from_millis(240));
        assert!((hue - 1.0 / 360.0).abs() < 0.0001);
        assert!(!active);
    }
    #[test]
    fn hsv_roundtrips_hsl_and_alpha_including_black_and_white() {
        for color in [
            [0.0, 1.0, 0.5, 1.0],
            [0.65, 0.4, 0.35, 0.3],
            [0.2, 0.0, 0.5, 0.7],
            [0.8, 0.0, 0.0, 1.0],
            [0.1, 0.0, 1.0, 0.0],
        ] {
            let result = Hsv::from_hsl(color).to_hsl();
            for (actual, expected) in result.into_iter().zip(color) {
                assert!((actual - expected).abs() < 0.0001);
            }
        }
    }
}
