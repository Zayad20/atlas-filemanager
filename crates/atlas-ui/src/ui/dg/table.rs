//! Priority compression is a projection of preferences, never a mutation of them.
//! Compress every column to its readable minimum before collapsing any column.
//! Collapse consumes the remaining deficit continuously, so no breakpoint jumps.

#[derive(Clone, Copy)]
pub(crate) struct ColumnSpec {
    pub preferred: f32,
    pub minimum: f32,
    /// Lower values surrender space first, independent of the visual order.
    pub priority: u8,
}

pub(crate) fn fit_columns<const N: usize>(
    available: f32,
    specs: [ColumnSpec; N],
    flexible: usize,
) -> [f32; N] {
    let available = if available.is_finite() {
        available.max(0.0)
    } else {
        0.0
    };
    let minima = specs.map(|s| {
        if s.minimum.is_finite() {
            s.minimum.max(0.0)
        } else {
            0.0
        }
    });
    let mut widths = std::array::from_fn(|i| {
        if specs[i].preferred.is_finite() {
            specs[i].preferred.max(minima[i])
        } else {
            minima[i]
        }
    });
    let mut priority: [usize; N] = std::array::from_fn(|i| i);
    priority.sort_by_key(|&i| specs[i].priority);
    let total: f32 = widths.iter().sum();
    if available >= total {
        if let Some(width) = widths.get_mut(flexible) {
            *width += available - total;
        }
        return widths;
    }
    let mut deficit = total - available;
    // Phase one: content compresses, including the primary column last.
    for &i in &priority {
        let take = deficit.min(widths[i] - minima[i]);
        widths[i] -= take;
        deficit -= take;
    }
    // Phase two: secondary slots close; the primary slot is the last survivor.
    for &i in &priority {
        let take = deficit.min(widths[i]);
        widths[i] -= take;
        deficit -= take;
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> [ColumnSpec; 4] {
        [
            ColumnSpec {
                preferred: 300.0,
                minimum: 120.0,
                priority: 3,
            },
            ColumnSpec {
                preferred: 90.0,
                minimum: 46.0,
                priority: 2,
            },
            ColumnSpec {
                preferred: 150.0,
                minimum: 92.0,
                priority: 0,
            },
            ColumnSpec {
                preferred: 110.0,
                minimum: 56.0,
                priority: 1,
            },
        ]
    }

    #[test]
    fn compresses_in_importance_order_before_closing_slots() {
        assert_eq!(fit_columns(620.0, specs(), 0), [300.0, 90.0, 120.0, 110.0]);
        assert_eq!(fit_columns(550.0, specs(), 0), [300.0, 90.0, 92.0, 68.0]);
        assert_eq!(fit_columns(314.0, specs(), 0), [120.0, 46.0, 92.0, 56.0]);
        assert_eq!(fit_columns(280.0, specs(), 0), [120.0, 46.0, 58.0, 56.0]);
        assert_eq!(fit_columns(222.0, specs(), 0), [120.0, 46.0, 0.0, 56.0]);
        assert_eq!(fit_columns(166.0, specs(), 0), [120.0, 46.0, 0.0, 0.0]);
        assert_eq!(fit_columns(80.0, specs(), 0), [80.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn every_pixel_conserves_space_and_has_no_breakpoint_jump() {
        let mut previous = fit_columns(0.0, specs(), 0);
        for step in 1..=4000 {
            let available = step as f32 * 0.25;
            let widths = fit_columns(available, specs(), 0);
            assert!((widths.iter().sum::<f32>() - available).abs() < 0.001);
            for i in 0..4 {
                assert!(widths[i] >= previous[i]);
                assert!(widths[i] - previous[i] <= 0.251);
            }
            previous = widths;
        }
    }

    #[test]
    fn invalid_dimensions_saturate_and_visual_order_does_not_change_priority() {
        assert_eq!(fit_columns(f32::NAN, specs(), 0), [0.0; 4]);
        assert_eq!(fit_columns(-1.0, specs(), 0), [0.0; 4]);
        let s = specs();
        assert_eq!(
            fit_columns(280.0, [s[0], s[3], s[1], s[2]], 0),
            [120.0, 56.0, 46.0, 58.0]
        );
    }
}
