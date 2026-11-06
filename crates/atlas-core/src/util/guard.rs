//! Panic containment for work that must not take its caller down.
//!
//! Long-lived worker loops (watchers, STA shell workers, sidecar readers) die
//! with their thread on an unwinding panic, silently disabling a feature until
//! restart. Running each unit of work through [`contain`] turns the panic into
//! an `Err` the loop can report and survive. The panic hook still records the
//! panic for the crash log.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Runs `work`, converting a panic into `Err(message)`.
///
/// `AssertUnwindSafe` is deliberate: callers pass owned or lock-guarded state,
/// and poisoned locks are recovered via `atlas_model::sync::recover`.
pub fn contain<T>(label: &str, work: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(work)).map_err(|payload| {
        let message = panic_message(payload.as_ref());
        tracing::error!(target: "atlas::panic", worker = label, %message, "contained panic");
        message
    })
}

/// Best-effort text of a panic payload (`&str` or `String`, as `panic!` makes).
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

#[cfg(test)]
mod tests {
    use super::contain;

    #[test]
    fn panics_become_errors() {
        assert_eq!(contain("ok", || 3), Ok(3));
        let err = contain("boom", || -> u8 { panic!("bad {}", 7) }).unwrap_err();
        assert_eq!(err, "bad 7");
    }
}
