//! Persistent parent/child catalog used by MFT and USN producers.

use super::{CatalogEntry, SearchOrigin};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const REASON_FILE_CREATE: u32 = 0x0000_0100;
pub const REASON_FILE_DELETE: u32 = 0x0000_0200;
pub const REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
pub const REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: u64,
    pub parent: u64,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeRecord {
    pub id: u64,
    pub parent: u64,
    pub name: String,
    pub is_dir: bool,
    pub reason: u32,
}

#[derive(Serialize, Deserialize)]
pub struct VolumeIndex {
    pub schema: u32,
    pub volume_id: String,
    pub journal_id: u64,
    pub next_usn: i64,
    pub root_id: u64,
    records: HashMap<u64, FileRecord>,
}

impl VolumeIndex {
    pub fn new(volume_id: String, journal_id: u64, next_usn: i64, root_id: u64) -> Self {
        Self {
            schema: 1,
            volume_id,
            journal_id,
            next_usn,
            root_id,
            records: HashMap::new(),
        }
    }

    pub fn replace_records(&mut self, records: impl IntoIterator<Item = FileRecord>) {
        self.records = records
            .into_iter()
            .map(|record| (record.id, record))
            .collect();
    }

    pub fn apply(&mut self, change: ChangeRecord) {
        if change.reason & REASON_FILE_DELETE != 0 {
            self.records.remove(&change.id);
            return;
        }
        // OLD_NAME merely pairs with NEW_NAME. Applying it would temporarily lose a valid entry
        // if a checkpoint happens between the two records.
        if change.reason & REASON_RENAME_OLD_NAME != 0
            && change.reason & REASON_RENAME_NEW_NAME == 0
        {
            return;
        }
        if change.reason & (REASON_FILE_CREATE | REASON_RENAME_NEW_NAME) != 0 {
            let old = self.records.get(&change.id);
            self.records.insert(
                change.id,
                FileRecord {
                    id: change.id,
                    parent: change.parent,
                    name: change.name,
                    is_dir: change.is_dir,
                    size: old.map_or(0, |record| record.size),
                    modified: old.and_then(|record| record.modified),
                },
            );
        }
    }

    pub fn entries(&self, root: &Path) -> Vec<CatalogEntry> {
        self.records
            .values()
            .filter_map(|record| {
                self.path_for(record.id, root).map(|path| CatalogEntry {
                    path,
                    is_dir: record.is_dir,
                    size: record.size,
                    modified: record.modified,
                    origin: SearchOrigin::WindowsService,
                })
            })
            .collect()
    }

    pub fn path_for(&self, id: u64, root: &Path) -> Option<PathBuf> {
        if id == self.root_id {
            return Some(root.to_path_buf());
        }
        let mut current = id;
        let mut components = Vec::new();
        let mut seen = HashSet::new();
        while current != self.root_id {
            if !seen.insert(current) {
                return None;
            }
            let record = self.records.get(&current)?;
            components.push(record.name.as_str());
            current = record.parent;
        }
        let mut path = root.to_path_buf();
        for component in components.into_iter().rev() {
            path.push(component);
        }
        Some(path)
    }

    pub fn records_len(&self) -> usize {
        self.records.len()
    }

    pub fn records(&self) -> impl Iterator<Item = &FileRecord> {
        self.records.values()
    }

    pub fn record(&self, id: u64) -> Option<&FileRecord> {
        self.records.get(&id)
    }

    /// Compact schema-v2 representation. Integer fields use unsigned LEB128 and names share the
    /// surrounding byte stream, avoiding JSON field names, escaping and decimal expansion.
    pub fn write_binary(&self, mut writer: impl Write) -> io::Result<()> {
        writer.write_all(b"ATLSIDX2")?;
        write_varint(&mut writer, self.schema as u64)?;
        write_bytes(&mut writer, self.volume_id.as_bytes())?;
        write_varint(&mut writer, self.journal_id)?;
        write_varint(&mut writer, zigzag(self.next_usn))?;
        write_varint(&mut writer, self.root_id)?;
        write_varint(&mut writer, self.records.len() as u64)?;
        let mut records: Vec<_> = self.records.values().collect();
        records.sort_unstable_by_key(|record| record.id);
        let mut previous_id = 0;
        for record in records {
            write_varint(&mut writer, record.id.saturating_sub(previous_id))?;
            previous_id = record.id;
            write_varint(&mut writer, record.parent)?;
            writer.write_all(&[u8::from(record.is_dir)])?;
            write_varint(&mut writer, record.size)?;
            write_varint(
                &mut writer,
                record.modified.map_or(0, |value| value.saturating_add(1)),
            )?;
            write_bytes(&mut writer, record.name.as_bytes())?;
        }
        Ok(())
    }

    pub fn read_binary(mut reader: impl Read) -> io::Result<Self> {
        let mut magic = [0; 8];
        reader.read_exact(&mut magic)?;
        if &magic != b"ATLSIDX2" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Atlas index",
            ));
        }
        let schema = read_varint(&mut reader)? as u32;
        let volume_id = String::from_utf8(read_bytes(&mut reader)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let journal_id = read_varint(&mut reader)?;
        let next_usn = unzigzag(read_varint(&mut reader)?);
        let root_id = read_varint(&mut reader)?;
        let count = read_varint(&mut reader)? as usize;
        let mut records = HashMap::with_capacity(count);
        let mut previous_id = 0_u64;
        for _ in 0..count {
            let id = previous_id.saturating_add(read_varint(&mut reader)?);
            previous_id = id;
            let parent = read_varint(&mut reader)?;
            let mut flag = [0];
            reader.read_exact(&mut flag)?;
            let size = read_varint(&mut reader)?;
            let modified = match read_varint(&mut reader)? {
                0 => None,
                value => Some(value - 1),
            };
            let name = String::from_utf8(read_bytes(&mut reader)?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            records.insert(
                id,
                FileRecord {
                    id,
                    parent,
                    name,
                    is_dir: flag[0] != 0,
                    size,
                    modified,
                },
            );
        }
        Ok(Self {
            schema,
            volume_id,
            journal_id,
            next_usn,
            root_id,
            records,
        })
    }
}

fn write_bytes(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    write_varint(writer, bytes.len() as u64)?;
    writer.write_all(bytes)
}
fn read_bytes(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let length = read_varint(reader)? as usize;
    if length > 32 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized index field",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn write_varint(writer: &mut impl Write, mut value: u64) -> io::Result<()> {
    while value >= 0x80 {
        writer.write_all(&[(value as u8) | 0x80])?;
        value >>= 7;
    }
    writer.write_all(&[value as u8])
}
fn read_varint(reader: &mut impl Read) -> io::Result<u64> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "invalid varint"))
}
const fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}
const fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(id: u64, parent: u64, name: &str, dir: bool) -> FileRecord {
        FileRecord {
            id,
            parent,
            name: name.into(),
            is_dir: dir,
            size: 0,
            modified: None,
        }
    }

    #[test]
    fn reconstructs_out_of_order_records() {
        let mut index = VolumeIndex::new("vol".into(), 1, 1, 5);
        index.replace_records([
            record(9, 7, "file.txt", false),
            record(7, 5, "folder", true),
        ]);
        assert_eq!(
            index.path_for(9, Path::new("C:\\")),
            Some(PathBuf::from("C:\\folder\\file.txt"))
        );
    }

    #[test]
    fn rename_and_move_update_descendant_paths() {
        let mut index = VolumeIndex::new("vol".into(), 1, 1, 5);
        index.replace_records([
            record(7, 5, "old", true),
            record(9, 7, "file", false),
            record(8, 5, "other", true),
        ]);
        index.apply(ChangeRecord {
            id: 7,
            parent: 8,
            name: "new".into(),
            is_dir: true,
            reason: REASON_RENAME_NEW_NAME,
        });
        assert_eq!(
            index.path_for(9, Path::new("C:\\")),
            Some(PathBuf::from("C:\\other\\new\\file"))
        );
    }

    #[test]
    fn delete_removes_result_and_orphans_children() {
        let mut index = VolumeIndex::new("vol".into(), 1, 1, 5);
        index.replace_records([record(7, 5, "folder", true), record(9, 7, "file", false)]);
        index.apply(ChangeRecord {
            id: 7,
            parent: 5,
            name: "folder".into(),
            is_dir: true,
            reason: REASON_FILE_DELETE,
        });
        assert!(index.path_for(9, Path::new("C:\\")).is_none());
    }

    #[test]
    fn binary_round_trip_is_compact() {
        let mut index = VolumeIndex::new("C:".into(), 7, 99, 5);
        index.replace_records([
            record(7, 5, "folder", true),
            record(9, 7, "résumé.txt", false),
        ]);
        let mut bytes = Vec::new();
        index.write_binary(&mut bytes).unwrap();
        let restored = VolumeIndex::read_binary(bytes.as_slice()).unwrap();
        assert_eq!(
            restored.path_for(9, Path::new("C:\\")),
            Some(PathBuf::from("C:\\folder\\résumé.txt"))
        );
        assert!(bytes.len() < 100);
    }
}
