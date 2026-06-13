use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo};
use std::path::PathBuf;
use std::sync::Arc;

/// Fuzzy-matching the folder index doesn't need a worker per core. Passing `None`
/// to `Nucleo::new` would spawn `available_parallelism()` threads (one per core),
/// all kept alive for the whole session even while the goto bar is closed.
const NUCLEO_THREADS: usize = 2;

/// One indexed filesystem entry. `is_dir` comes from the directory walk (no
/// extra metadata call at render time).
#[derive(Clone)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

/// `match_paths` tunes the scoring bonuses for `/`-separated haystacks
/// (filename and path-prefix bonuses), which is exactly what we index.
const fn config() -> Config {
    Config::DEFAULT.match_paths()
}

pub struct NucleoPool {
    pub nucleo: Nucleo<IndexEntry>,
}

impl NucleoPool {
    pub fn new() -> Self {
        Self {
            nucleo: Nucleo::new(config(), Arc::new(|| {}), Some(NUCLEO_THREADS), 1),
        }
    }

    pub fn injector(&self) -> nucleo::Injector<IndexEntry> {
        self.nucleo.injector()
    }

    pub fn set_pattern(&mut self, query: &str) {
        self.nucleo
            .pattern
            .reparse(0, query, CaseMatching::Smart, Normalization::Smart, false);
    }

    pub fn tick(&mut self) {
        self.nucleo.tick(10);
    }

    pub fn results(&self, limit: usize) -> Vec<IndexEntry> {
        let snapshot = self.nucleo.snapshot();
        let count = (snapshot.matched_item_count() as usize).min(limit);
        (0..count)
            .filter_map(|i| snapshot.get_matched_item(i as u32))
            .map(|item| item.data.clone())
            .collect()
    }

    pub fn reset(&mut self) {
        self.nucleo = Nucleo::new(config(), Arc::new(|| {}), Some(NUCLEO_THREADS), 1);
    }
}
