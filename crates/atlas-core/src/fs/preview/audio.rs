use std::path::Path;

use atlas_model::preview_cache::PreviewResult;

use super::image::decode_cover;

pub(super) fn preview_audio(path: &Path) -> PreviewResult {
    use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};

    let tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(e) => return PreviewResult::Error(format!("Could not read audio: {e}")),
    };
    let props = tagged.properties();
    let duration_secs = props.duration().as_secs();
    let bitrate = props.audio_bitrate().or_else(|| props.overall_bitrate());

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let (title, artist, album, mut cover) = match tag {
        Some(tag) => {
            let cover = tag
                .pictures()
                .first()
                .and_then(|pic| decode_cover(pic.data()));
            (
                tag.title().map(|c| c.to_string()),
                tag.artist().map(|c| c.to_string()),
                tag.album().map(|c| c.to_string()),
                cover,
            )
        }
        None => (None, None, None, None),
    };

    if cover.is_none() {
        cover = find_folder_cover(path);
    }

    PreviewResult::Audio {
        title,
        artist,
        album,
        duration_secs,
        bitrate,
        cover,
    }
}

pub fn extract_audio_thumbnail(path: &Path, max_edge: u32) -> Option<(Vec<u8>, usize, usize)> {
    use lofty::prelude::TaggedFileExt;

    if let Ok(tagged) = lofty::read_from_path(path) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                if let Ok(img) = image::load_from_memory(pic.data()) {
                    let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
                    let (w, h) = (thumb.width() as usize, thumb.height() as usize);
                    return Some((thumb.into_raw(), w, h));
                }
            }
        }
    }

    // Fallback: look for album cover file in the same directory
    let parent = path.parent()?;
    for name in &["cover.jpg", "cover.png", "folder.jpg", "folder.png", "album.jpg", "front.jpg"] {
        let candidate = parent.join(name);
        if candidate.is_file() {
            if let Ok(img) = image::open(&candidate) {
                let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
                let (w, h) = (thumb.width() as usize, thumb.height() as usize);
                return Some((thumb.into_raw(), w, h));
            }
        }
    }
    None
}

fn find_folder_cover(path: &Path) -> Option<atlas_model::preview_cache::ImageData> {
    let parent = path.parent()?;
    for name in &["cover.jpg", "cover.png", "folder.jpg", "folder.png", "album.jpg", "front.jpg"] {
        let candidate = parent.join(name);
        if candidate.is_file() {
            if let Ok(img) = image::open(&candidate) {
                let thumb = img
                    .thumbnail(super::MAX_MULTI_THUMBNAIL_EDGE, super::MAX_MULTI_THUMBNAIL_EDGE)
                    .to_rgba8();
                let (w, h) = (thumb.width() as usize, thumb.height() as usize);
                return Some(atlas_model::preview_cache::ImageData {
                    rgba: thumb.into_raw(),
                    width: w,
                    height: h,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_audio_thumbnail_folder_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("song.mp3");
        // Create an empty dummy file
        std::fs::write(&audio_path, b"dummy").unwrap();

        let cover_path = dir.path().join("cover.png");
        let img = image::RgbaImage::new(50, 50);
        img.save(&cover_path).unwrap();

        let thumb = extract_audio_thumbnail(&audio_path, 32);
        assert!(thumb.is_some());
        let (rgba, w, h) = thumb.unwrap();
        assert_eq!(w, 32);
        assert_eq!(h, 32);
        assert_eq!(rgba.len(), 32 * 32 * 4);
    }
}

