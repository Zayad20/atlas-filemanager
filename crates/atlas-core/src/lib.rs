//! `atlas-core`: backends and background work — filesystem, search, cloud,
//! extensions, tag DB, watchers, jobs. Depends on `atlas-model`; never on the UI.
//!
//! Moved here from the legacy `src/{fs,cloud,ext,search,util}` trees (Fase J); the
//! binary re-exports each as `crate::{fs,cloud,ext,search,util}` via shims.

pub mod ext;
pub mod fs;
pub mod search;
pub mod space_lens;
pub mod update;
pub mod util;

#[cfg(feature = "cloud")]
pub mod cloud;
