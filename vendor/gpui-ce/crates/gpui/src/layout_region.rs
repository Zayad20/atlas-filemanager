//! Attribute layout-tree nodes to named UI regions (sidebar, dock, …).
//!
//! Apps wrap subtrees with [`enter_layout_region`] so each `Window::request_layout`
//! increments a per-region counter for the frame.
//! Zero-cost unless `GPUI_FRAME_LOG` or `GPUI_LAYOUT_REGIONS` is set.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

thread_local! {
    static STACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static COUNTS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
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
