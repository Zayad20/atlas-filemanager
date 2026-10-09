//! GPU texture cache for Icons8 Liquid Glass default file and folder icons.
//!
//! Pre-decodes embedded PNG assets into Arc<RenderImage> with GPUI-expected
//! straight BGRA channel ordering, enabling zero-allocation instant rendering
//! in both high-performance custom virtual elements (DetailsElement, GridSurfaceElement)
//! and standard GPUI views (miller, list, inspector, etc.).

use std::collections::HashMap;
use std::sync::Arc;

use atlas_model::file_entry::FileKind;
use gpui::RenderImage;

use crate::assets::liquid_glass_filetype;
use crate::ui::icons::{file_type_icon_kind, FileTypeIcon};

const ALL_ICONS: &[FileTypeIcon] = &[
    FileTypeIcon::Folder,
    FileTypeIcon::File,
    FileTypeIcon::Document,
    FileTypeIcon::Pdf,
    FileTypeIcon::Image,
    FileTypeIcon::Audio,
    FileTypeIcon::Video,
    FileTypeIcon::Code,
    FileTypeIcon::Python,
    FileTypeIcon::Archive,
    FileTypeIcon::Presentation,
];

pub(crate) struct LiquidGlassCache {
    textures: HashMap<FileTypeIcon, Arc<RenderImage>>,
    fallback: Arc<RenderImage>,
}

impl LiquidGlassCache {
    pub(crate) fn new() -> Self {
        let mut textures = HashMap::with_capacity(ALL_ICONS.len());

        for &kind in ALL_ICONS {
            let path = kind.path();
            if let Some(bytes) = liquid_glass_filetype(path) {
                if let Some(tex) = decode_png_to_render_image(bytes) {
                    textures.insert(kind, tex);
                }
            }
        }

        let fallback = textures
            .get(&FileTypeIcon::File)
            .cloned()
            .unwrap_or_else(create_dummy_texture);

        Self { textures, fallback }
    }

    #[inline]
    pub(crate) fn texture_for(&self, icon: FileTypeIcon) -> Arc<RenderImage> {
        self.textures.get(&icon).cloned().unwrap_or_else(|| self.fallback.clone())
    }

    #[inline]
    pub(crate) fn texture_for_entry(&self, kind: &FileKind, extension: Option<&str>) -> Arc<RenderImage> {
        let icon_kind = file_type_icon_kind(kind, extension);
        self.texture_for(icon_kind)
    }
}

/// Decode raw PNG bytes to an Arc<RenderImage> with straight BGRA format.
fn decode_png_to_render_image(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut raw = img.into_raw();

    // Swap R <-> B for GPUI's straight-BGRA GPU pipeline.
    for px in raw.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    let buf = image::RgbaImage::from_raw(w, h, raw)?;
    let frame = image::Frame::new(buf);
    Some(Arc::new(RenderImage::new(vec![frame])))
}

fn create_dummy_texture() -> Arc<RenderImage> {
    let buf = image::RgbaImage::from_raw(1, 1, vec![128, 128, 128, 255]).expect("1x1 buffer");
    Arc::new(RenderImage::new(vec![image::Frame::new(buf)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_eleven_liquid_glass_icons() {
        let cache = LiquidGlassCache::new();
        assert_eq!(cache.textures.len(), 11, "All 11 Liquid Glass PNGs must decode");

        for &kind in ALL_ICONS {
            let tex = cache.texture_for(kind);
            let sz = tex.size(0);
            assert!(sz.width.0 > 0 && sz.height.0 > 0, "Icon {:?} has invalid dimensions", kind);
        }

        // Test entry resolution
        let folder_tex = cache.texture_for_entry(&FileKind::Dir, None);
        assert_eq!(folder_tex.size(0), cache.texture_for(FileTypeIcon::Folder).size(0));

        let code_tex = cache.texture_for_entry(&FileKind::File, Some("rs"));
        assert_eq!(code_tex.size(0), cache.texture_for(FileTypeIcon::Code).size(0));
    }
}

