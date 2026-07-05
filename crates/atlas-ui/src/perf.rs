#[cfg(feature = "perf-log")]
mod imp {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    struct Config {
        enabled: bool,
        events: bool,
        threshold: Duration,
        started: Instant,
    }

    static CONFIG: OnceLock<Config> = OnceLock::new();
    static FRAME: AtomicU64 = AtomicU64::new(0);
    static COUNTERS: OnceLock<Mutex<HashMap<&'static str, u64>>> = OnceLock::new();
    static LAST_FRAME: OnceLock<Mutex<Instant>> = OnceLock::new();

    fn truthy(name: &str) -> bool {
        std::env::var(name)
            .ok()
            .is_some_and(|v| !matches!(v.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF"))
    }

    fn config() -> &'static Config {
        CONFIG.get_or_init(|| {
            let enabled = truthy("ATLAS_PERF");
            let events = truthy("ATLAS_PERF_EVENTS");
            let threshold_ms = std::env::var("ATLAS_PERF_MS")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(4.0)
                .max(0.0);
            Config {
                enabled: enabled || events,
                events,
                threshold: Duration::from_secs_f64(threshold_ms / 1000.0),
                started: Instant::now(),
            }
        })
    }

    pub(crate) fn next_frame() -> u64 {
        if config().enabled {
            FRAME.fetch_add(1, Ordering::Relaxed) + 1
        } else {
            0
        }
    }

    pub(crate) fn count(name: &'static str) {
        count_by(name, 1);
    }

    pub(crate) fn count_by(name: &'static str, amount: u64) {
        if !config().enabled {
            return;
        }
        let mut counters = COUNTERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        *counters.entry(name).or_default() += amount;
    }

    pub(crate) fn frame_summary(frame: u64, detail: impl FnOnce() -> String) {
        let cfg = config();
        if !cfg.events {
            return;
        }
        let now = Instant::now();
        let gap_ms = {
            let mut last = LAST_FRAME.get_or_init(|| Mutex::new(now)).lock().unwrap();
            let gap = now.duration_since(*last).as_secs_f64() * 1000.0;
            *last = now;
            gap
        };
        let counters = COUNTERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .drain()
            .collect::<Vec<_>>();
        if counters.is_empty() && frame > 1 {
            return;
        }
        let mut counters = counters;
        counters.sort_by_key(|(name, _)| *name);
        let counters = counters
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "[atlas-perf +{:>8.1}ms] frame_summary frame={frame} gap_ms={gap_ms:.1} {} {}",
            cfg.started.elapsed().as_secs_f64() * 1000.0,
            detail(),
            counters
        );
    }

    pub(crate) fn span(name: &'static str) -> Option<Span> {
        config().enabled.then(|| Span {
            name,
            detail: None,
            start: Instant::now(),
        })
    }

    pub(crate) fn span_detail(name: &'static str, detail: impl FnOnce() -> String) -> Option<Span> {
        config().enabled.then(|| Span {
            name,
            detail: Some(detail()),
            start: Instant::now(),
        })
    }

    pub(crate) fn event(detail: impl FnOnce() -> String) {
        let cfg = config();
        if cfg.events {
            eprintln!(
                "[atlas-perf +{:>8.1}ms] {}",
                cfg.started.elapsed().as_secs_f64() * 1000.0,
                detail()
            );
        }
    }

    pub(crate) struct Span {
        name: &'static str,
        detail: Option<String>,
        start: Instant,
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let cfg = config();
            let elapsed = self.start.elapsed();
            if elapsed < cfg.threshold {
                return;
            }
            let ms = elapsed.as_secs_f64() * 1000.0;
            match &self.detail {
                Some(detail) => eprintln!(
                    "[atlas-perf +{:>8.1}ms] {:>7.2}ms {} {}",
                    cfg.started.elapsed().as_secs_f64() * 1000.0,
                    ms,
                    self.name,
                    detail
                ),
                None => eprintln!(
                    "[atlas-perf +{:>8.1}ms] {:>7.2}ms {}",
                    cfg.started.elapsed().as_secs_f64() * 1000.0,
                    ms,
                    self.name
                ),
            }
        }
    }
}

#[cfg(feature = "perf-log")]
pub(crate) use imp::*;

#[cfg(not(feature = "perf-log"))]
pub(crate) fn next_frame() -> u64 {
    0
}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn count(_name: &'static str) {}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn count_by(_name: &'static str, _amount: u64) {}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn frame_summary(_frame: u64, _detail: impl FnOnce() -> String) {}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn span(_name: &'static str) -> Option<Span> {
    None
}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn span_detail(_name: &'static str, _detail: impl FnOnce() -> String) -> Option<Span> {
    None
}

#[cfg(not(feature = "perf-log"))]
pub(crate) fn event(_detail: impl FnOnce() -> String) {}

#[cfg(not(feature = "perf-log"))]
pub(crate) struct Span;
