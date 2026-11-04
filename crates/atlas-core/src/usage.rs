//! Local usage counters behind the Free edition's daily limits. Soft limits by
//! design: the counter lives on the user's machine, so it only paces honest use.
// reason: tiny JSON read once per process and cached; writes happen off the UI thread.
#![allow(clippy::disallowed_methods)]

use crate::util::config::config_dir;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Counter {
    day: String,
    conversions: usize,
}

static STATE: Mutex<Option<Counter>> = Mutex::new(None);

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// A counter from a previous day starts again at zero.
fn rolled(counter: Counter, today: &str) -> Counter {
    if counter.day == today {
        counter
    } else {
        Counter {
            day: today.to_owned(),
            conversions: 0,
        }
    }
}

#[cfg(not(test))]
fn path() -> std::path::PathBuf {
    config_dir().join("usage.json")
}

/// Tests never touch the real config directory.
#[cfg(not(test))]
fn load() -> Counter {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
fn load() -> Counter {
    Counter::default()
}

#[cfg(not(test))]
fn persist(counter: &Counter) {
    if let Ok(bytes) = serde_json::to_vec(counter) {
        let _ = std::fs::create_dir_all(config_dir());
        let _ = std::fs::write(path(), bytes);
    }
}

#[cfg(test)]
fn persist(_: &Counter) {}

fn with_counter<R>(f: impl FnOnce(&mut Counter) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let current = rolled(guard.take().unwrap_or_else(load), &today());
    let result = f(guard.insert(current));
    drop(guard);
    result
}

/// Files converted so far today (local date).
pub fn conversions_today() -> usize {
    with_counter(|counter| counter.conversions)
}

const fn add(counter: &mut Counter, count: usize) {
    counter.conversions = counter.conversions.saturating_add(count);
}

/// Adds successfully converted files to today's total. Inert under test so the
/// process-wide counter never leaks between unrelated conversion tests.
pub fn record_conversions(count: usize) {
    if count == 0 || cfg!(test) {
        return;
    }
    with_counter(|counter| {
        add(counter, count);
        persist(counter);
    });
}

/// Loads the saved counter so the first UI check never reads the disk.
pub fn preload() {
    conversions_today();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_day_resets_the_counter_and_the_same_day_keeps_it() {
        let yesterday = Counter {
            day: "2026-11-01".into(),
            conversions: 9,
        };
        assert_eq!(
            rolled(yesterday.clone(), "2026-11-02"),
            Counter {
                day: "2026-11-02".into(),
                conversions: 0
            }
        );
        assert_eq!(rolled(yesterday.clone(), "2026-11-01"), yesterday);
    }

    #[test]
    fn adding_accumulates_and_saturates() {
        let mut counter = Counter::default();
        add(&mut counter, 3);
        add(&mut counter, 2);
        assert_eq!(counter.conversions, 5);
        add(&mut counter, usize::MAX);
        assert_eq!(counter.conversions, usize::MAX);
    }
}
