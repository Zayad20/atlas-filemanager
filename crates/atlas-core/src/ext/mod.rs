//! Extension system: manifest-described extensions (declarative command tools or
//! JSON-lines provider sidecars) discovered at runtime.
//!
//! Users install extensions into the extensions directory (from a `.zip`/folder),
//! enable/disable them, and their contributions — commands in the palette and the
//! right-click menu — appear without recompiling Atlas.

pub mod install;
pub mod manifest;
pub mod marketplace;
pub mod provider_sidecar;
pub mod registry;
pub mod runner;

pub use registry::Registry;

use std::path::PathBuf;

/// Where installed extensions live: `<data>/extensions/<id>/extension.toml`.
pub fn extensions_dir() -> PathBuf {
    crate::util::config::data_dir().join("extensions")
}
