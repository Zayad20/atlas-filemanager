use crate::fs::preview::PreviewResult;
use crate::state::layout_tree::TileId;
use std::collections::HashMap;
use std::path::PathBuf;

pub const MULTI_PREVIEW_LIMIT: usize = 4;

/// Framework-neutral decoded image: tightly-packed RGBA8 plus dimensions. The UI
/// layer uploads this to a GPU texture and caches the handle on its own side, so
/// `state` never holds a renderer texture handle.
#[derive(Clone, Debug)]
pub struct ImageData {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiPreviewKey {
    pub fingerprint: u64,
    pub selected_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewRequest {
    Single(PathBuf),
    Multiple {
        key: MultiPreviewKey,
        paths: Vec<PathBuf>,
    },
}

pub struct MultiPreviewState {
    pub key: MultiPreviewKey,
    pub paths: Vec<PathBuf>,
    /// Decoded thumbnails, in arrival order. The UI uploads these to textures.
    pub images: Vec<(PathBuf, ImageData)>,
}

#[derive(Default)]
pub struct PreviewState {
    pub paths: HashMap<TileId, PathBuf>,
    pub results: HashMap<TileId, PreviewResult>,
    pub multi: HashMap<TileId, MultiPreviewState>,
}

impl PreviewState {
    pub fn clear(&mut self, tab: TileId) {
        self.paths.remove(&tab);
        self.results.remove(&tab);
        self.multi.remove(&tab);
    }

    pub fn clear_all(&mut self) {
        self.paths.clear();
        self.results.clear();
        self.multi.clear();
    }
}
