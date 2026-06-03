//! gpui UI layer.
//!
//! The old egui modules (file_list, layout, sidebar, inspector, …) remain on disk
//! as a porting reference but are intentionally NOT declared here, so they are
//! excluded from compilation during the migration. They are ported phase by phase
//! into gpui views and re-added below as they land.
pub mod root;
