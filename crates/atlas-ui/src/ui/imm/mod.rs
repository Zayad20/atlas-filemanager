//! Immediate-mode UI Kit for Atlas.
//!
//! Reusable immediate-mode primitives adhering to:
//! - Construction every frame.
//! - Widgets are plain functions (Composable functional helpers).
//! - Deterministic two-pass / rectangle slicing layout.
//! - Structural error elimination (saturating bounds).

pub mod cut;
pub mod input;
mod input_bridge;
pub(crate) mod menu_panel;
pub(crate) mod menu;
pub(crate) mod motion;
pub(crate) mod scroll;
pub(crate) mod table;

pub use cut::RectCut;
pub use input::ImmTextInputState;
