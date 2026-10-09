/// Half-resolution pixels needed by the separable blur of a visible composite.
/// Keep the full viewport/UV mapping; only restrict fragment work. The margin
/// includes the Gaussian taps, bilinear sampling and the composite's AA fringe.
/// The DX sampler wraps, so support crossing an edge needs that entire axis.
pub(super) fn blur_work_rect(
    bounds: [f32; 4],
    mask: [f32; 4],
    full_size: [u32; 2],
    kernel_extent: f32,
) -> Option<[i32; 4]> {
    let mut rect = [0; 4];
    for axis in 0..2 {
        let full = full_size[axis] as f32;
        if full <= 0.0 {
            return None;
        }
        let half = full_size[axis].div_ceil(2).max(1) as f32;
        let low = bounds[axis].max(mask[axis]).max(0.0);
        let high = bounds[axis + 2].min(mask[axis + 2]).min(full);
        if high <= low {
            return None;
        }
        let margin = kernel_extent.ceil() + 2.0;
        let start = ((low - 1.0) * half / full).floor() - margin;
        let end = ((high + 1.0) * half / full).ceil() + margin;
        let (start, end) = if start < 0.0 || end > half {
            (0.0, half)
        } else {
            (start, end)
        };
        rect[axis] = start as i32;
        rect[axis + 2] = end as i32;
    }
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::blur_work_rect;

    #[test]
    fn clipped_sliver_limits_work_instead_of_blurring_the_entire_pane() {
        let rect = blur_work_rect(
            [0.0, 0.0, 1600.0, 1000.0],
            [800.0, 200.0, 840.0, 800.0],
            [1600, 1000],
            18.0,
        )
        .unwrap();
        assert_eq!(rect, [379, 79, 441, 421]);
        assert!((rect[2] - rect[0]) * (rect[3] - rect[1]) < 800 * 500 / 16);
    }

    #[test]
    fn wraparound_taps_keep_the_opposite_window_edge_available() {
        assert_eq!(
            blur_work_rect(
                [0.0, 300.0, 40.0, 500.0],
                [0.0, 0.0, 1600.0, 1000.0],
                [1600, 1000],
                18.0,
            ),
            Some([0, 129, 800, 271])
        );
    }

    #[test]
    fn invisible_filters_do_no_blur_work() {
        assert_eq!(
            blur_work_rect(
                [0.0, 0.0, 400.0, 400.0],
                [450.0, 0.0, 800.0, 600.0],
                [800, 600],
                18.0,
            ),
            None
        );
    }

    #[test]
    fn odd_window_dimensions_preserve_the_original_uv_mapping() {
        let rect = blur_work_rect(
            [300.0, 200.0, 350.0, 250.0],
            [0.0, 0.0, 801.0, 601.0],
            [801, 601],
            2.0,
        )
        .unwrap();
        assert_eq!(rect, [145, 95, 180, 130]);
    }
}
