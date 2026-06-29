//! TAR archive provider (supports plain tar, .tar.gz, .tar.bz2, .tar.xz).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use bzip2::read::BzDecoder;
use xz2::read::XzDecoder;

use super::common::{build_synthetic_ancestors, path_to_dir_key, write_to_dest};
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

            let parent_key = inner.parent().map(path_to_dir_key).unwrap_or_default();
            build_synthetic_ancestors(path, &inner, &mut index);

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
        let key = path_to_dir_key(inner_dir);
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
        write_to_dest(dest, cached.data.as_deref().unwrap_or(&[]))
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}
