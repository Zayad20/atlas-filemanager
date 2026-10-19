use std::f32::consts::{FRAC_PI_2, TAU};

pub(crate) fn angle(index: usize, count: usize) -> f32 {
    -FRAC_PI_2 + TAU * index as f32 / count.max(1) as f32
}

pub(crate) fn hit(x: f32, y: f32, radius: f32, count: usize) -> Option<usize> {
    let distance = x.hypot(y);
    if count == 0 || distance < radius * 0.40 || distance > radius - 5.0 {
        return None;
    }
    let step = TAU / count as f32;
    let t = (y.atan2(x) + FRAC_PI_2 + step * 0.5).rem_euclid(TAU);
    let within = t.rem_euclid(step);
    if within < 0.035 || step - within < 0.035 {
        return None;
    }
    Some(((t / step) as usize).min(count - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sectors_follow_the_reference_clockwise_from_the_top() {
        assert_eq!(hit(0., -120., 180., 4), Some(0));
        assert_eq!(hit(120., 0., 180., 4), Some(1));
        assert_eq!(hit(0., 120., 180., 4), Some(2));
        assert_eq!(hit(-120., 0., 180., 4), Some(3));
        assert_eq!(angle(0, 4), -FRAC_PI_2);
    }
    #[test]
    fn center_outside_and_gutters_are_not_conversion_targets() {
        assert_eq!(hit(0., 0., 180., 4), None);
        assert_eq!(hit(181., 0., 180., 4), None);
        assert_eq!(hit(100., -100., 180., 4), None);
        assert_eq!(hit(0., -120., 180., 0), None);
    }
}
