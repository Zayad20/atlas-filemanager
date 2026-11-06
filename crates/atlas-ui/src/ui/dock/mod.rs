/// One gutter for the whole workspace (window inset, sidebar gap, split seams),
/// so islands read as a single system the way iPadOS split view does.
pub(crate) const GUTTER: f32 = 6.0;

pub mod drag;
pub mod render;
pub mod resize;
