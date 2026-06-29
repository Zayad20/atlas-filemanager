//! Compatibility re-export for cloud model types.
//!
//! The data model lives in `state::cloud_model`; the cloud module owns only
//! backend behavior.

pub use atlas_model::cloud_model::{AccessKind, CloudRemote, RemoteId, RemoteKind, RemoteStatus};
