//! Attribute layout-tree nodes to named UI regions (sidebar, dock, …).
//!
//! Apps wrap subtrees with [`enter_layout_region`] so each `Window::request_layout`
//! increments a per-region counter for the frame.
//! Zero-cost unless `GPUI_FRAME_LOG` or `GPUI_LAYOUT_REGIONS` is set.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

thread_local! {
    static STACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static COUNTS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static LAYOUT_TIMINGS: RefCell<LayoutTimings> = RefCell::new(LayoutTimings::default());
}

/// Inclusive solver wall time. Leaf measurements are a subset, not extra work.
#[derive(Default, Clone, Copy)]
pub(crate) struct LayoutTimings {
    pub solve: Duration,
    pub measure: Duration,
    pub solve_calls: u64,
    pub measure_calls: u64,
}

pub(crate) fn record_layout_solve(elapsed: Duration) {
    LAYOUT_TIMINGS.with(|timings| {
        let mut timings = timings.borrow_mut();
        timings.solve += elapsed;
        timings.solve_calls += 1;
    });
}

pub(crate) fn record_layout_measure(elapsed: Duration, calls: u64) {
    LAYOUT_TIMINGS.with(|timings| {
        let mut timings = timings.borrow_mut();
        timings.measure += elapsed;
        timings.measure_calls += calls;
    });
}

pub(crate) fn layout_timings() -> LayoutTimings {
    LAYOUT_TIMINGS.with(|timings| *timings.borrow())
}

pub(crate) fn reset_layout_timings() {
    LAYOUT_TIMINGS.with(|timings| *timings.borrow_mut() = LayoutTimings::default());
}

/// Top-level draw buckets are disjoint. `layout_measure` is a solver submetric.
#[derive(Default)]
pub(crate) struct FramePhases {
    pub root_request_other: Duration,
    pub hit_test: Duration,
    pub paint: Duration,
    pub a11y_finish: Duration,
}

impl FramePhases {
    fn prepaint_other(&self, prepaint: Duration, layout: LayoutTimings) -> Duration {
        prepaint.saturating_sub(self.root_request_other + layout.solve + self.hit_test)
    }
}

fn tracking_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("GPUI_LAYOUT_REGIONS")
            .ok()
            .is_some_and(|v| !matches!(v.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF"))
            || std::env::var("GPUI_FRAME_LOG").is_ok()
    })
}

/// RAII guard: layout nodes created while this is alive count toward `name`.
pub struct LayoutRegionGuard {
    _private: (),
}

/// Push a layout-attribution region for the rest of the current scope.
pub fn enter_layout_region(name: &'static str) -> LayoutRegionGuard {
    if tracking_enabled() {
        STACK.with(|s| s.borrow_mut().push(name));
    }
    LayoutRegionGuard { _private: () }
}

impl Drop for LayoutRegionGuard {
    fn drop(&mut self) {
        if tracking_enabled() {
            STACK.with(|s| {
                s.borrow_mut().pop();
            });
        }
    }
}

/// Record one layout node under the current region (or `"untagged"`).
pub(crate) fn record_layout_node() {
    if !tracking_enabled() {
        return;
    }
    let name = STACK.with(|s| s.borrow().last().copied().unwrap_or("untagged"));
    COUNTS.with(|c| {
        *c.borrow_mut().entry(name).or_default() += 1;
    });
}

/// Clear per-frame region counts (call at the start of each draw).
pub(crate) fn clear_layout_region_counts() {
    if !tracking_enabled() {
        return;
    }
    COUNTS.with(|c| c.borrow_mut().clear());
}

/// Drain region → node counts for this frame (sorted by count desc).
pub fn take_layout_region_counts() -> Vec<(&'static str, u64)> {
    if !tracking_enabled() {
        return Vec::new();
    }
    COUNTS.with(|c| {
        let mut v: Vec<_> = std::mem::take(&mut *c.borrow_mut()).into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        v
    })
}

/// Optional threshold for Atlas-compatible per-frame diagnostics.
pub(crate) fn frame_log_threshold() -> Option<std::time::Duration> {
    static THRESHOLD: OnceLock<Option<std::time::Duration>> = OnceLock::new();
    *THRESHOLD.get_or_init(|| {
        std::env::var("GPUI_FRAME_LOG").ok().map(|value| {
            let millis = value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(16.0);
            std::time::Duration::from_secs_f64((millis / 1000.0).min(86400.0))
        })
    })
}

pub(crate) fn log_frame_regions(
    total: std::time::Duration,
    prepaint: std::time::Duration,
    scene_ops: usize,
    phases: FramePhases,
) {
    let regions = take_layout_region_counts();
    let threshold = frame_log_threshold();
    if threshold.is_some_and(|threshold| total < threshold) {
        return;
    }
    if threshold.is_some() {
        eprintln!(
            "[gpui-frame] total={:.2}ms prepaint={:.2}ms paint={:.2}ms scene_ops={scene_ops}",
            total.as_secs_f64() * 1000.0,
            prepaint.as_secs_f64() * 1000.0,
            total.saturating_sub(prepaint).as_secs_f64() * 1000.0
        );
        let layout = layout_timings();
        eprintln!(
            "[gpui-phases] root_request_other={:.3}ms layout_solve={:.3}ms prepaint_other={:.3}ms hit_test={:.3}ms paint={:.3}ms a11y_finish={:.3}ms layout_measure={:.3}ms layout_solve_ex_measure={:.3}ms solve_calls={} measure_calls={}",
            phases.root_request_other.as_secs_f64() * 1000.0,
            layout.solve.as_secs_f64() * 1000.0,
            phases.prepaint_other(prepaint, layout).as_secs_f64() * 1000.0,
            phases.hit_test.as_secs_f64() * 1000.0,
            phases.paint.as_secs_f64() * 1000.0,
            phases.a11y_finish.as_secs_f64() * 1000.0,
            layout.measure.as_secs_f64() * 1000.0,
            layout.solve.saturating_sub(layout.measure).as_secs_f64() * 1000.0,
            layout.solve_calls,
            layout.measure_calls,
        );
    }
    if !regions.is_empty() {
        let sum: u64 = regions.iter().map(|(_, count)| count).sum();
        let parts: Vec<_> = regions
            .into_iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect();
        eprintln!("[gpui-layout] nodes_created={sum} {}", parts.join(" "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_buckets_do_not_count_leaf_measure_twice() {
        let phases = FramePhases {
            root_request_other: Duration::from_millis(3),
            hit_test: Duration::from_millis(1),
            ..Default::default()
        };
        let layout = LayoutTimings {
            solve: Duration::from_millis(5),
            measure: Duration::from_millis(2),
            ..Default::default()
        };
        assert_eq!(
            phases.prepaint_other(Duration::from_millis(16), layout),
            Duration::from_millis(7)
        );
    }

    #[test]
    fn layout_timings_reset_between_draws() {
        reset_layout_timings();
        record_layout_solve(Duration::from_millis(4));
        record_layout_measure(Duration::from_millis(1), 3);
        assert_eq!(layout_timings().solve_calls, 1);
        assert_eq!(layout_timings().measure_calls, 3);
        reset_layout_timings();
        assert_eq!(layout_timings().solve, Duration::ZERO);
        assert_eq!(layout_timings().measure, Duration::ZERO);
        assert_eq!(layout_timings().solve_calls, 0);
        assert_eq!(layout_timings().measure_calls, 0);
    }
}
