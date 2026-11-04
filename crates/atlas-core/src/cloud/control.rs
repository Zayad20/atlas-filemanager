//! Cooperative cancellation shared by the host and provider. A cancellation
//! request is distinct from acknowledgement that the backend stopped writing.
#[derive(Clone, Debug)]
pub struct TransferControl {
    cancelled: tokio::sync::watch::Sender<bool>,
    byte_limit: Option<u64>,
}

impl Default for TransferControl {
    fn default() -> Self {
        Self {
            cancelled: tokio::sync::watch::channel(false).0,
            byte_limit: None,
        }
    }
}

impl TransferControl {
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }
    pub fn with_byte_limit(mut self, limit: Option<u64>) -> Self {
        self.byte_limit = limit;
        self
    }
    pub fn byte_limit(&self) -> Option<u64> {
        self.byte_limit
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.cancelled.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[derive(Debug)]
pub struct TransferCancelled;
impl std::fmt::Display for TransferCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("transfer cancelled")
    }
}
impl std::error::Error for TransferCancelled {}

#[derive(Debug)]
pub struct OutcomeUnknown(pub String);
impl std::fmt::Display for OutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for OutcomeUnknown {}
