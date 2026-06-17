//! 7z archive provider (read-only, supports AES encryption).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use sevenz_rust2::{Archive, ArchiveReader, Password};

use super::{make_entry, ArchiveCaps, ArchiveProvider};
use atlas_model::file_entry::FileEntry;

/// Pre-indexed, read-only view of a `.7z` file.
///
/// Metadata is parsed at construction; `read_bytes` / `extract_to` re-open
/// the archive so the provider is `Send + Sync`.
pub struct SevenzProvider {
    path: PathBuf,
    /// `inner_dir` (with trailing '/') → direct children entries
    index: HashMap<String, Vec<FileEntry>>,
    password: Option<String>,
    encrypted: bool,
}

impl SevenzProvider {
    pub fn new(path: &Path, password: Option<&str>) -> anyhow::Result<Self> {
        let pw = password_obj(password);
        let archive = match Archive::open_with_password(path, &pw) {
            Ok(a) => a,
            Err(sevenz_rust2::Error::PasswordRequired) => {
                return Ok(Self {
                    path: path.to_path_buf(),
                    index: HashMap::new(),
                    password: password.map(String::from),
                    encrypted: true,
                });
            }
            Err(e) => anyhow::bail!("failed to open 7z archive: {e}"),
        };

        let mut index: HashMap<String, Vec<FileEntry>> = HashMap::new();
        index.entry(String::new()).or_default();

        for entry in &archive.files {
            if entry.is_anti_item {
                continue;
            }
            let normalized = entry.name().replace('\\', "/");
            let is_dir = entry.is_directory();
            let inner = PathBuf::from(normalized.trim_end_matches('/'));
            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            let size = if is_dir { None } else { Some(entry.size) };

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

            // Ensure ancestor keys exist and insert synthetic Dir entries.
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

            let file_entry = make_entry(path, inner.clone(), name, is_dir, size);
            index.entry(parent_key).or_default().push(file_entry);
        }

        Ok(Self {
            path: path.to_path_buf(),
            index,
            password: password.map(String::from),
            encrypted: false,
        })
    }

    fn ensure_unlocked(&self) -> anyhow::Result<()> {
        if self.encrypted {
            anyhow::bail!("PasswordRequired");
        }
        Ok(())
    }

    fn read_entry_bytes(&self, inner_path: &Path) -> anyhow::Result<Vec<u8>> {
        self.ensure_unlocked()?;
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let pw = password_obj(self.password.as_deref());
        let mut reader = ArchiveReader::open(&self.path, pw)
            .map_err(|e| anyhow::anyhow!("failed to open 7z: {e}"))?;
        reader
            .read_file(&name)
            .map_err(|e| anyhow::anyhow!("failed to read entry '{name}': {e}"))
    }
}

impl ArchiveProvider for SevenzProvider {
    fn caps(&self) -> ArchiveCaps {
        ArchiveCaps {
            write: false,
            passwords: true,
            preserves_compression_on_rewrite: false,
        }
    }

    fn list(&self, inner_dir: &Path) -> anyhow::Result<Vec<FileEntry>> {
        self.ensure_unlocked()?;
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
        let data = self.read_entry_bytes(inner_path)?;
        Ok(Box::new(Cursor::new(data)))
    }

    fn extract_to(&self, inner_path: &Path, dest: &Path) -> anyhow::Result<()> {
        let data = self.read_entry_bytes(inner_path)?;
        if let Some(parent) = dest.parent() {
            #[allow(clippy::disallowed_methods)]
            // reason: trait is sync; runs under spawn_blocking
            std::fs::create_dir_all(parent)?;
        }
        #[allow(clippy::disallowed_methods)] // reason: trait is sync; runs under spawn_blocking
        std::fs::write(dest, data)?;
        Ok(())
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}

fn password_obj(pw: Option<&str>) -> Password {
    match pw {
        Some(s) => Password::from(s),
        None => Password::empty(),
    }
}
