use crate::fs::preview::PreviewResult;
use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::PathBuf;

pub const MULTI_PREVIEW_LIMIT: usize = 4;

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
    pub textures: Vec<(PathBuf, egui::TextureHandle)>,
}

#[derive(Default)]
pub struct PreviewState {
    pub paths: HashMap<TileId, PathBuf>,
    pub results: HashMap<TileId, PreviewResult>,
    pub textures: HashMap<TileId, egui::TextureHandle>,
    pub multi: HashMap<TileId, MultiPreviewState>,
}

impl PreviewState {
    pub fn clear(&mut self, tab: TileId) {
        self.paths.remove(&tab);
        self.results.remove(&tab);
        self.textures.remove(&tab);
        self.multi.remove(&tab);
    }

    pub fn clear_all(&mut self) {
        self.paths.clear();
        self.results.clear();
        self.textures.clear();
        self.multi.clear();
    }
}
