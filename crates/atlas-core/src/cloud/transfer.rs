//! Compatibility re-export for transfer model types.
//!
//! Transfer records are state/model data. Queue orchestration stays in
//! `cloud::worker`.

pub use atlas_model::cloud_model::{Transfer, TransferId, TransferKind, TransferState};
