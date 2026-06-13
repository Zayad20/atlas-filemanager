//! FASE J: `state` now lives in the `atlas-model` crate; this shim re-exports it so
//! the binary's existing `crate::state::…` paths keep resolving unchanged.
//! (`space_lens` moved to `atlas-core` as a scan engine.)
pub use atlas_model::*;
