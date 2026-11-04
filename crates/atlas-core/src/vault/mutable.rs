//! Mutable, authenticated copy-on-write blocks. Manifest replacement is the commit.
use super::*;
use std::io::{Seek, SeekFrom};

const MAX_BLOCKS: u64 = 1_048_576; // 1 TiB/file; bounds authenticated manifest allocation.

impl Session {
    pub fn resolve(&self, path: &Path) -> Result<Option<String>> {
        let mut parent = None;
        for component in path.components() {
            let Component::Normal(name) = component else {
                ensure!(
                    matches!(component, Component::RootDir),
                    "vault_invalid_path"
                );
                continue;
            };
            let name = name.to_str().context("vault_invalid_name")?;
            validate_name(name)?;
            let entry = self
                .manifest
                .entries
                .iter()
                .find(|e| e.parent == parent && same_name(&e.name, name))
                .context("vault_missing_entry")?;
            parent = Some(entry.id.clone());
        }
        Ok(parent)
    }

    pub fn read_range(&self, id: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let entry = self.entry(id).context("vault_missing_entry")?;
        ensure!(!entry.directory, "vault_not_file");
        let n = buffer.len().min(entry.size.saturating_sub(offset) as usize);
        let mut copied = 0;
        while copied < n {
            let position = offset + copied as u64;
            let block = position / CHUNK as u64;
            let within = (position % CHUNK as u64) as usize;
            let bytes = self.read_block(entry, block)?;
            let take = (n - copied).min(CHUNK - within);
            buffer[copied..copied + take].fill(0);
            if within < bytes.len() {
                let present = take.min(bytes.len() - within);
                buffer[copied..copied + present].copy_from_slice(&bytes[within..within + present]);
            }
            copied += take;
        }
        Ok(n)
    }

    fn block_aad(&self, id: &str, index: u64) -> Vec<u8> {
        [
            b"atlas-block-v2".as_slice(),
            self.envelope.id.as_bytes(),
            id.as_bytes(),
            &index.to_le_bytes(),
        ]
        .concat()
    }

    fn read_block(&self, entry: &Entry, index: u64) -> Result<Zeroizing<Vec<u8>>> {
        if let Some(blocks) = &entry.blocks {
            let Some(Some(id)) = blocks.get(index as usize) else {
                return Ok(Zeroizing::new(Vec::new()));
            };
            ensure!(valid_id(id), "vault_invalid_manifest");
            let path = self.root.join(id);
            let meta = plain_metadata(&path)?;
            ensure!(meta.len() <= CHUNK as u64 + 40, "vault_damaged");
            let bytes = fs::read(path)?;
            return open(&self.key, &bytes, &self.block_aad(&entry.id, index));
        }
        if index * CHUNK as u64 >= entry.size {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let path = self.root.join(&entry.id);
        let meta = plain_metadata(&path)?;
        let chunks = entry.size.div_ceil(CHUNK as u64).max(1);
        ensure!(meta.len() == 32 + entry.size + chunks * 16, "vault_damaged");
        let mut source = File::open(path)?;
        let mut header = [0; 32];
        source.read_exact(&mut header)?;
        ensure!(
            &header[..8] == MAGIC && header[24..] == entry.size.to_le_bytes(),
            "vault_damaged"
        );
        source.seek(SeekFrom::Start(32 + index * (CHUNK as u64 + 16)))?;
        let size = (entry.size - index * CHUNK as u64).min(CHUNK as u64) as usize;
        let mut encrypted = vec![0; size + 16];
        source.read_exact(&mut encrypted)?;
        let mut nonce = [0; 24];
        nonce[..16].copy_from_slice(&header[8..24]);
        nonce[16..].copy_from_slice(&index.to_le_bytes());
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_slice())
            .map_err(|_| anyhow::anyhow!("vault_invalid_key"))?;
        Ok(Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &encrypted,
                        aad: &self.object_aad(entry, index),
                    },
                )
                .map_err(|_| anyhow::anyhow!("vault_damaged"))?,
        ))
    }

    fn put_block(
        &self,
        entry: &Entry,
        index: u64,
        bytes: &[u8],
        added: &mut Vec<String>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        // Record before creation so any partial encrypted object is cleaned up on error.
        added.push(id.clone());
        let encrypted = seal(&self.key, bytes, &self.block_aad(&entry.id, index))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(self.root.join(&id))?;
        file.write_all(&encrypted)?;
        file.sync_all()?;
        Ok(id)
    }

    fn commit_entries(&mut self, entries: Vec<Entry>) -> Result<()> {
        let next = Manifest {
            name: self.manifest.name.clone(),
            entries,
        };
        validate_manifest(&next)?;
        self.save_manifest(&next)?;
        self.manifest = next;
        Ok(())
    }

    pub fn create_entry(
        &mut self,
        parent: Option<&str>,
        name: &str,
        directory: bool,
    ) -> Result<String> {
        validate_name(name)?;
        if let Some(parent) = parent {
            ensure!(
                self.entry(parent).is_some_and(|e| e.directory),
                "vault_missing_entry"
            );
        }
        ensure!(
            !self
                .children(parent)
                .iter()
                .any(|e| same_name(&e.name, name)),
            "vault_name_exists"
        );
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut entries = self.manifest.entries.clone();
        entries.push(Entry {
            id: id.clone(),
            parent: parent.map(str::to_owned),
            name: name.to_owned(),
            directory,
            size: 0,
            modified: now(),
            digest: [0; 32],
            blocks: if directory { None } else { Some(vec![]) },
            attributes: 0,
        });
        self.commit_entries(entries)?;
        Ok(id)
    }

    pub fn write_at(&mut self, id: &str, offset: u64, bytes: &[u8]) -> Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let entry = self.entry(id).context("vault_missing_entry")?;
        let size = offset
            .checked_add(bytes.len() as u64)
            .context("vault_file_too_large")?
            .max(entry.size);
        self.edit_file(id, size, Some((offset, bytes)))?;
        Ok(bytes.len())
    }

    pub fn resize(&mut self, id: &str, size: u64) -> Result<()> {
        self.edit_file(id, size, None)
    }

    fn edit_file(&mut self, id: &str, size: u64, write: Option<(u64, &[u8])>) -> Result<()> {
        ensure!(self.envelope.version >= 2, "vault_invalid_manifest");
        ensure!(
            size.div_ceil(CHUNK as u64) <= MAX_BLOCKS,
            "vault_file_too_large"
        );
        let entry = self.entry(id).context("vault_missing_entry")?.clone();
        ensure!(!entry.directory, "vault_not_file");
        let mut added = Vec::new();
        let mut retired = Vec::new();
        let result = (|| {
            let mut blocks = if let Some(blocks) = &entry.blocks {
                blocks.clone()
            } else {
                let mut blocks = Vec::new();
                for i in 0..entry.size.min(size).div_ceil(CHUNK as u64) {
                    let bytes = self.read_block(&entry, i)?;
                    blocks.push(Some(self.put_block(&entry, i, &bytes, &mut added)?));
                }
                retired.push(entry.id.clone());
                blocks
            };
            let count = size.div_ceil(CHUNK as u64) as usize;
            if blocks.len() > count {
                retired.extend(blocks.drain(count..).flatten());
            }
            blocks.resize(count, None);
            // Truncate the last block so growing again cannot reveal retired bytes.
            if size < entry.size && size % CHUNK as u64 != 0 {
                let i = count - 1;
                let mut bytes = self.read_block(&entry, i as u64)?;
                bytes.truncate((size % CHUNK as u64) as usize);
                if let Some(old) = blocks[i].take() {
                    retired.push(old);
                }
                if !bytes.is_empty() {
                    blocks[i] = Some(self.put_block(&entry, i as u64, &bytes, &mut added)?);
                }
            }
            if let Some((offset, input)) = write {
                let mut copied = 0;
                while copied < input.len() {
                    let position = offset + copied as u64;
                    let i = (position / CHUNK as u64) as usize;
                    let within = (position % CHUNK as u64) as usize;
                    let take = (input.len() - copied).min(CHUNK - within);
                    let mut bytes = self.read_block(&entry, i as u64)?;
                    let length = bytes.len().max(within + take);
                    bytes.resize(length, 0);
                    bytes[within..within + take].copy_from_slice(&input[copied..copied + take]);
                    if let Some(old) = blocks[i].take() {
                        retired.push(old);
                    }
                    blocks[i] = Some(self.put_block(&entry, i as u64, &bytes, &mut added)?);
                    copied += take;
                }
            }
            let mut entries = self.manifest.entries.clone();
            let live = entries.iter_mut().find(|e| e.id == id);
            let detached = live.is_none();
            let next = match live {
                Some(entry) => entry,
                None => self
                    .retired
                    .iter_mut()
                    .find(|e| e.id == id)
                    .context("vault_missing_entry")?,
            };
            next.size = size;
            next.modified = now();
            next.blocks = Some(blocks);
            next.digest = [0; 32];
            if detached {
                Ok(())
            } else {
                self.commit_entries(entries)
            }
        })();
        let cleanup = if result.is_ok() { &retired } else { &added };
        for id in cleanup {
            let _ = fs::remove_file(self.root.join(id));
        }
        result
    }

    pub fn rename_entry(
        &mut self,
        id: &str,
        parent: Option<&str>,
        name: &str,
        replace: bool,
    ) -> Result<()> {
        validate_name(name)?;
        let entry = self
            .manifest
            .entries
            .iter()
            .find(|e| e.id == id)
            .context("vault_missing_entry")?;
        if let Some(parent) = parent {
            ensure!(
                self.entry(parent).is_some_and(|e| e.directory),
                "vault_missing_entry"
            );
        }
        let destination = self
            .children(parent)
            .into_iter()
            .find(|e| e.id != id && same_name(&e.name, name));
        if let Some(dest) = &destination {
            ensure!(
                replace && !entry.directory && !dest.directory,
                "vault_name_exists"
            );
        }
        let mut entries = self.manifest.entries.clone();
        if let Some(dest) = &destination {
            entries.retain(|e| e.id != dest.id);
        }
        let next = entries.iter_mut().find(|e| e.id == id).unwrap();
        next.parent = parent.map(str::to_owned);
        next.name = name.to_owned();
        self.commit_entries(entries)?;
        if let Some(dest) = destination {
            self.retired.push(dest);
        }
        Ok(())
    }

    pub fn delete_entry(&mut self, id: &str) -> Result<()> {
        let entry = self
            .manifest
            .entries
            .iter()
            .find(|e| e.id == id)
            .context("vault_missing_entry")?
            .clone();
        ensure!(
            !entry.directory || self.children(Some(id)).is_empty(),
            "vault_directory_not_empty"
        );
        let entries = self
            .manifest
            .entries
            .iter()
            .filter(|e| e.id != id)
            .cloned()
            .collect();
        self.commit_entries(entries)?;
        self.retired.push(entry);
        Ok(())
    }

    // Called after the last native handle closes. Deleted/replaced files stay
    // readable to pre-existing handles until this point (Windows sharing semantics).
    pub fn release_entry(&mut self, id: &str) {
        if let Some(index) = self.retired.iter().position(|entry| entry.id == id) {
            let entry = self.retired.remove(index);
            self.remove_objects(&entry);
        }
    }

    pub fn collect_orphans(&self) -> Result<()> {
        let referenced: HashSet<_> = self
            .manifest
            .entries
            .iter()
            .chain(&self.retired)
            .flat_map(|e| {
                if let Some(blocks) = &e.blocks {
                    blocks.iter().flatten().cloned().collect::<Vec<_>>()
                } else if e.directory {
                    vec![]
                } else {
                    vec![e.id.clone()]
                }
            })
            .collect();
        for file in fs::read_dir(&self.root)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if valid_id(&name) && !referenced.contains(&name) {
                fs::remove_file(file.path())?;
            }
        }
        Ok(())
    }

    fn remove_objects(&self, entry: &Entry) {
        if let Some(blocks) = &entry.blocks {
            for id in blocks.iter().flatten() {
                let _ = fs::remove_file(self.root.join(id));
            }
        } else if !entry.directory {
            let _ = fs::remove_file(self.root.join(&entry.id));
        }
    }

    pub fn set_metadata(&mut self, id: &str, attributes: u32, modified: Option<u64>) -> Result<()> {
        if let Some(entry) = self.retired.iter_mut().find(|entry| entry.id == id) {
            entry.attributes = attributes;
            if let Some(modified) = modified {
                entry.modified = modified;
            }
            return Ok(());
        }
        let mut entries = self.manifest.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .context("vault_missing_entry")?;
        entry.attributes = attributes;
        if let Some(modified) = modified {
            entry.modified = modified;
        }
        self.commit_entries(entries)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
