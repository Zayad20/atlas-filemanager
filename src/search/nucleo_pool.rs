use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo};
use std::path::PathBuf;
use std::sync::Arc;

pub struct NucleoPool {
    pub nucleo: Nucleo<PathBuf>,
}

impl NucleoPool {
    pub fn new() -> Self {
        Self {
            nucleo: Nucleo::new(Config::DEFAULT, Arc::new(|| {}), None, 1),
        }
    }

    pub fn injector(&self) -> nucleo::Injector<PathBuf> {
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

    pub fn results(&self, limit: usize) -> Vec<PathBuf> {
        let snapshot = self.nucleo.snapshot();
        let count = (snapshot.matched_item_count() as usize).min(limit);
        (0..count)
            .filter_map(|i| snapshot.get_matched_item(i as u32))
            .map(|item| item.data.clone())
            .collect()
    }

    pub fn reset(&mut self) {
        self.nucleo = Nucleo::new(Config::DEFAULT, Arc::new(|| {}), None, 1);
    }
}
