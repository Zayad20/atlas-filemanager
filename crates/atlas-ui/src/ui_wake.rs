//! Coalesced worker/OS notifications for the foreground maintenance task.
//! Receivers park without a timer while their window has no pending work.
use std::sync::Mutex;

static LISTENERS: Mutex<Vec<flume::Sender<()>>> = Mutex::new(Vec::new());

pub(crate) fn subscribe() -> flume::Receiver<()> {
    let (tx, rx) = flume::bounded(1);
    LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
    rx
}

pub(crate) fn wake() {
    LISTENERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|tx| !matches!(tx.try_send(()), Err(flume::TrySendError::Disconnected(()))));
}
