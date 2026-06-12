//! ZIP archive provider.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use super::{make_entry, ArchiveProvider};
use atlas_model::file_entry::FileEntry;

/// Pre-indexed, read-only view of a `.zip` file.
///
/// The central directory is parsed once at construction; subsequent `list()`
/// calls are O(entries in directory) with no disk I/O.
pub struct ZipProvider {
    path: PathBuf,
    /// `inner_dir` (with trailing '/') → direct children entries
    index: HashMap<String, Vec<FileEntry>>,
}

impl ZipProvider {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut index: HashMap<String, Vec<FileEntry>> = HashMap::new();

        // Seed the root so `list("")` works even for empty archives.
        index.entry(String::new()).or_default();

        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            // enclosed_name() rejects zip-slip paths (../, absolute, etc.).
            let inner = match entry.enclosed_name() {
                Some(n) => n.to_path_buf(),
                None => continue,
            };
            let raw_name = entry.name();
            // Skip __MACOSX cruft and other meta entries.
            if raw_name.starts_with("__MACOSX/") || raw_name.ends_with(".DS_Store") {
                continue;
            }
            let is_dir = entry.is_dir();
            let size = if is_dir { None } else { Some(entry.size()) };
            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            // Parent dir key (empty string = root).
            let parent_key = inner
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().replace('\\', "/");
                    if s.is_empty() {
                        String::new()
                    } else {
                        format!("{s}/")
                    }
                })
                .unwrap_or_default();

            // Ensure all ancestor keys exist and synthetic Dir entries are
            // visible in each ancestor's parent listing (needed for zips that
            // omit explicit directory entries, e.g. Python zipfile / 7-Zip).
            let mut cur = PathBuf::new();
            for comp in inner.parent().into_iter().flat_map(|p| p.components()) {
                let prev = cur.clone();
                cur.push(comp);
                let k = {
                    let s = cur.to_string_lossy().replace('\\', "/");
                    format!("{s}/")
                };
                if !index.contains_key(&k) {
                    index.entry(k).or_default();
                    // Insert synthetic Dir entry into the parent listing.
                    let dir_name = cur
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !dir_name.is_empty() {
                        let prev_key = {
                            let s = prev.to_string_lossy().replace('\\', "/");
                            if s.is_empty() {
                                String::new()
                            } else {
                                format!("{s}/")
                            }
                        };
                        let synthetic = make_entry(path, cur.clone(), dir_name.clone(), true, None);
                        let parent_list = index.entry(prev_key).or_default();
                        if !parent_list.iter().any(|e| e.name == dir_name) {
                            parent_list.push(synthetic);
                        }
                    }
                }
            }

            let file_entry = make_entry(path, inner.clone(), name, is_dir, size);
            index.entry(parent_key).or_default().push(file_entry);
        }

        Ok(Self {
            path: path.to_path_buf(),
            index,
        })
    }
}

impl ArchiveProvider for ZipProvider {
    fn list(&self, inner_dir: &Path) -> anyhow::Result<Vec<FileEntry>> {
        let key = {
            let s = inner_dir.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                String::new()
            } else {
                format!("{s}/")
            }
        };
        Ok(self.index.get(&key).cloned().unwrap_or_default())
    }

    fn read_bytes(&self, inner_path: &Path) -> anyhow::Result<Box<dyn Read + Send>> {
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let file = std::fs::File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut entry = zip.by_name(&name)?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        Ok(Box::new(Cursor::new(buf)))
    }

    fn extract_to(&self, inner_path: &Path, dest: &Path) -> anyhow::Result<()> {
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let file = std::fs::File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut entry = zip.by_name(&name)?;
        if let Some(parent) = dest.parent() {
            #[allow(clippy::disallowed_methods)]
            // reason: trait is sync; runs under spawn_blocking
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(dest)?;
        std::io::copy(&mut entry, &mut out)?;
        Ok(())
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}
