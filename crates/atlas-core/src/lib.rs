//! `atlas-core`: backends and background work — filesystem, search, cloud,
//! extensions, tag DB, watchers, jobs. Depends on `atlas-model`; never on the UI.
//!
//! Moved here from the legacy `src/{fs,cloud,ext,search,util}` trees (Phase J); the
//! binary re-exports each as `crate::{fs,cloud,ext,search,util}` via shims.

pub mod convert;
pub mod cleanup;
pub mod ext;
pub mod fs;
pub mod license;
pub mod search;
pub mod space_lens;
pub mod update;
pub mod usage;
pub mod util;
pub mod vault;

#[cfg(feature = "provider-host")]
pub mod cloud;
