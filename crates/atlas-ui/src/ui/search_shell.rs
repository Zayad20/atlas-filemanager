use crate::ui::icons::{icon, AtlasIcon};
use gpui::{AnyElement, Hsla};

#[allow(dead_code)]
pub(crate) fn search_icon(color: Hsla) -> AnyElement {
    icon(AtlasIcon::Search, 16.0, color)
}
