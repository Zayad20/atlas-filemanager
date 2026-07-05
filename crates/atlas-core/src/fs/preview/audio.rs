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
    let (title, artist, album, cover) = match tag {
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

    PreviewResult::Audio {
        title,
        artist,
        album,
        duration_secs,
        bitrate,
        cover,
    }
}
