//! TAR archive provider (supports plain tar, .tar.gz, .tar.bz2, .tar.xz).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use bzip2::read::BzDecoder;
use xz2::read::XzDecoder;

use super::{make_entry, ArchiveProvider};
use atlas_model::file_entry::FileEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zst,
}

struct CachedEntry {
    /// Raw bytes (None for directories).
    data: Option<Vec<u8>>,
}

pub struct TarProvider {
    path: PathBuf,
    /// `inner_dir` key → direct children `FileEntry` list
    index: HashMap<String, Vec<FileEntry>>,
    /// `inner_path` string → raw bytes cache for `read_bytes`
    content: HashMap<String, CachedEntry>,
}

impl TarProvider {
    pub fn new(path: &Path, compression: Compression) -> anyhow::Result<Self> {
        let mut index: HashMap<String, Vec<FileEntry>> = HashMap::new();
        let mut content: HashMap<String, CachedEntry> = HashMap::new();
        index.entry(String::new()).or_default();

        let file = std::fs::File::open(path)?;
        let reader: Box<dyn Read + Send> = match compression {
            Compression::None => Box::new(file),
            Compression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
            Compression::Bzip2 => Box::new(BzDecoder::new(file)),
            Compression::Xz => Box::new(XzDecoder::new(file)),
            Compression::Zst => anyhow::bail!("zstd tar archives are not supported yet"),
        };
        let mut ar = tar::Archive::new(reader);

        for entry_result in ar.entries()? {
            let mut entry = entry_result?;
            let raw_path = entry.path()?.into_owned();
            let raw_str = raw_path.to_string_lossy().replace('\\', "/");
            let inner = PathBuf::from(raw_str.trim_end_matches('/'));
            let is_dir = entry.header().entry_type().is_dir();
            let size = if is_dir {
                None
            } else {
                entry.header().size().ok()
            };

            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

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

            // Ensure ancestor keys exist and synthetic Dir entries appear in
            // each parent listing (handles tars without explicit dir entries).
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

            // Cache content for files.
            let data = if is_dir {
                None
            } else {
                let mut buf = Vec::with_capacity(size.unwrap_or(0) as usize);
                entry.read_to_end(&mut buf)?;
                Some(buf)
            };

            let inner_key = inner.to_string_lossy().replace('\\', "/");
            content.insert(inner_key, CachedEntry { data });

            let file_entry = make_entry(path, inner.clone(), name, is_dir, size);
            index.entry(parent_key).or_default().push(file_entry);
        }

        Ok(Self {
            path: path.to_path_buf(),
            index,
            content,
        })
    }
}

impl ArchiveProvider for TarProvider {
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
        let key = inner_path.to_string_lossy().replace('\\', "/");
        let cached = self
            .content
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("entry not found: {key}"))?;
        let data = cached.data.clone().unwrap_or_default();
        Ok(Box::new(Cursor::new(data)))
    }

    fn extract_to(&self, inner_path: &Path, dest: &Path) -> anyhow::Result<()> {
        let key = inner_path.to_string_lossy().replace('\\', "/");
        let cached = self
            .content
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("entry not found: {key}"))?;
        if let Some(parent) = dest.parent() {
            #[allow(clippy::disallowed_methods)]
            // reason: trait is sync; runs under spawn_blocking
            std::fs::create_dir_all(parent)?;
        }
        let data = cached.data.as_deref().unwrap_or(&[]);
        #[allow(clippy::disallowed_methods)] // reason: trait is sync; runs under spawn_blocking
        std::fs::write(dest, data)?;
        Ok(())
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}
