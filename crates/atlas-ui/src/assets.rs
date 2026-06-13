//! Combined asset source: our embedded Phosphor icons (served under `icons/ph/…`)
//! plus everything `gpui-component` bundles (its own `IconName` SVGs, fonts, …).
//!
//! gpui renders `svg()` as an alpha mask tinted by the current `text_color`, and the
//! Phosphor SVGs use `fill="currentColor"`, so a single `svg().path("icons/ph/…")`
//! picks up whatever color we set on it.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// App asset source. Phosphor icons are embedded here; anything else delegates to the
/// `gpui-component` asset bundle so its built-in `IconName` set keeps working.
pub struct AtlasAssets;

macro_rules! phosphor_icons {
    ($($name:literal),* $(,)?) => {
        /// Resolve an `icons/ph/<name>.svg` path to its embedded bytes.
        fn phosphor(path: &str) -> Option<&'static [u8]> {
            match path {
                $(
                    concat!("icons/ph/", $name, ".svg") => Some(include_bytes!(
                        concat!("../assets/icons/ph/", $name, ".svg")
                    )),
                )*
                _ => None,
            }
        }
    };
}

macro_rules! liquid_glass_filetype_icons {
    ($($name:literal),* $(,)?) => {
        /// Resolve an `icons/filetypes/liquid-glass/<name>.png` path to embedded bytes.
        fn liquid_glass_filetype(path: &str) -> Option<&'static [u8]> {
            match path {
                $(
                    concat!("icons/filetypes/liquid-glass/", $name, ".png") => Some(include_bytes!(
                        concat!("../assets/icons/filetypes/liquid-glass/", $name, ".png")
                    )),
                )*
                _ => None,
            }
        }
    };
}

phosphor_icons!(
    "house",
    "monitor",
    "file",
    "file-text",
    "download-simple",
    "image",
    "music-notes",
    "film-strip",
    "folder",
    "cloud",
    "tag",
    "clock-counter-clockwise",
    "hard-drives",
    "bookmark-simple",
    "caret-down",
    "caret-right",
    "magnifying-glass",
    "dots-three",
    "trash",
    "globe-simple",
    "grid-four",
    "list-bullets",
    "list-dashes",
    "columns",
    "copy-simple",
    // Palette engine: section/command icons + per-item buttons.
    "star",
    "star-fill",
    "plus",
    "lightning",
    "gear",
    "palette",
    "user-switch",
    "puzzle-piece",
    "arrows-left-right",
    "split-horizontal",
    "split-vertical",
    "eye",
    "pencil-simple",
    "arrow-counter-clockwise",
    "arrow-clockwise",
    "selection-all",
    "selection-inverse",
    "sort-ascending",
    "tree-structure",
    "x",
    "arrow-left",
    "arrow-right",
);

liquid_glass_filetype_icons!(
    "archive",
    "audio",
    "code",
    "document",
    "file",
    "folder",
    "image",
    "pdf",
    "presentation",
    "python",
    "video",
);

impl AssetSource for AtlasAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = phosphor(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = liquid_glass_filetype(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
    }
}
