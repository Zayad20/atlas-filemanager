//! Panic-free clamping for bounds computed at runtime.
//!
//! `f32::clamp` / `Ord::clamp` assert `min <= max` (and non-NaN bounds for
//! floats). Layout code derives both bounds from window sizes, zoom and text
//! widths, so a narrow window or an empty list can invert them — and with a
//! panic that kills the whole app. These variants never panic: when the
//! bounds cross, `max` wins (the value must fit the available space), and NaN
//! bounds are ignored.

pub trait ClampLenient: Sized {
    fn clamp_lenient(self, min: Self, max: Self) -> Self;
}

macro_rules! float_impl {
    ($($t:ty),*) => {$(
        impl ClampLenient for $t {
            #[inline]
            fn clamp_lenient(self, min: Self, max: Self) -> Self {
                // `max`/`min` return the non-NaN operand, so NaN bounds drop out.
                self.max(min).min(max)
            }
        }
    )*};
}

macro_rules! ord_impl {
    ($($t:ty),*) => {$(
        impl ClampLenient for $t {
            #[inline]
            fn clamp_lenient(self, min: Self, max: Self) -> Self {
                Ord::min(Ord::max(self, min), max)
            }
        }
    )*};
}

float_impl!(f32, f64);
ord_impl!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

#[cfg(test)]
mod tests {
    use super::ClampLenient;

    #[test]
    fn inverted_bounds_prefer_max() {
        assert_eq!(50.0_f32.clamp_lenient(16.0, 8.0), 8.0);
        assert_eq!(3_isize.clamp_lenient(0, -1), -1);
    }

    #[test]
    fn nan_bounds_are_ignored() {
        assert_eq!(5.0_f32.clamp_lenient(f32::NAN, 4.0), 4.0);
        assert_eq!(5.0_f32.clamp_lenient(0.0, f32::NAN), 5.0);
    }

    #[test]
    fn matches_clamp_for_valid_bounds() {
        for v in [-3.0_f32, 0.0, 2.5, 9.0] {
            assert_eq!(v.clamp_lenient(0.0, 4.0), v.clamp(0.0, 4.0));
        }
    }
}
