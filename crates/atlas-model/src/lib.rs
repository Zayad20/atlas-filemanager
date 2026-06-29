//! `atlas-model`: framework-neutral application state.
//!
//! IDs, serializable state, requests/results — no filesystem, cloud, extension or
//! UI runtime. Moved here from the legacy `src/state` tree (Fase J); the binary
//! re-exports it as `crate::state` via a shim so existing paths keep resolving.

pub mod app_state;
pub mod batch_rename;
#[cfg(feature = "cloud")]
pub mod cloud_model;
#[cfg(feature = "cloud")]
pub mod cloud_state;
pub mod ext_model;
pub mod ext_state;
pub mod file_entry;
pub mod layout_tree;
pub mod location;
pub mod ops;
pub mod overlay;
pub mod pane;
pub mod preview_cache;
pub mod profiles;
pub mod settings;
pub mod tab;
pub mod tag_cache;
pub mod undo;
