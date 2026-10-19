//! Platform-independent one-shot frame demand shared with the VSync worker.
use std::{
    collections::HashSet,
    sync::{Condvar, Mutex},
};

pub(crate) struct FrameDemand {
    pending: Mutex<Option<HashSet<isize>>>,
    ready: Condvar,
}

impl FrameDemand {
    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(Some(HashSet::new())),
            ready: Condvar::new(),
        }
    }

    pub(crate) fn request_window(&self, window: isize) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(windows) = pending.as_mut() {
            windows.insert(window);
            self.ready.notify_one();
        }
    }

    pub(crate) fn wait(&self) -> Option<HashSet<isize>> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        while pending.as_ref().is_some_and(HashSet::is_empty) {
            pending = self.ready.wait(pending).unwrap_or_else(|e| e.into_inner());
        }
        pending.as_mut().map(std::mem::take)
    }

    pub(crate) fn stop(&self) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    #[test]
    fn demand_blocks_until_request_and_parks_again_until_stop() {
        let demand = Arc::new(FrameDemand::new());
        let (tx, rx) = mpsc::channel();
        let worker = demand.clone();
        let thread = std::thread::spawn(move || {
            tx.send(worker.wait()).unwrap();
            tx.send(worker.wait()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        demand.request_window(123);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Some(HashSet::from([123]))
        );
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        demand.stop();
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_none());
        thread.join().unwrap();
    }

    #[test]
    fn requests_coalesce_by_window_without_losing_other_windows() {
        let demand = FrameDemand::new();
        demand.request_window(123);
        demand.request_window(123);
        demand.request_window(456);
        assert_eq!(demand.wait(), Some(HashSet::from([123, 456])));
    }
}
