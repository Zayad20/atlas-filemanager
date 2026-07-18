use anyhow::Context as _;
use atlas_core::search::volume_index::{
    REASON_FILE_CREATE, REASON_FILE_DELETE, REASON_RENAME_NEW_NAME, REASON_RENAME_OLD_NAME,
};
use atlas_win_space::ntfs::JournalRecord;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const FIND_MAGIC: &[u8; 8] = b"ATLSFND4";
const FIND_HEADER: usize = 24;
const FIND_ROW: usize = 20;
const POSTING_BITS: usize = 36;

#[derive(Clone)]
pub struct OverlayRecord {
    pub parent: u64,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

pub struct RecordRef<'a> {
    pub id: u64,
    pub parent: u64,
    pub name: &'a str,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

pub struct MappedVolume {
    map: Mmap,
    find_map: Mmap,
    count: usize,
    pub journal_id: u64,
    pub next_usn: i64,
    pub root_id: u64,
    pub overlay: HashMap<u64, OverlayRecord>,
    deleted: HashSet<u64>,
}

impl MappedVolume {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let map = unsafe { memmap2::MmapOptions::new().map(&file) }?;
        let mut cursor = Cursor::new(&map);
        anyhow::ensure!(cursor.take(8)? == b"ATLSIDX2", "invalid Atlas index");
        let _schema = cursor.varint()?;
        let volume_len = cursor.varint()? as usize;
        cursor.skip(volume_len)?;
        let journal_id = cursor.varint()?;
        let next_usn = unzigzag(cursor.varint()?);
        let stored_root_id = cursor.varint()?;
        let count = cursor.varint()? as usize;
        let source_len = map.len() as u64;
        let find_path = path.with_extension("atlasfind2");
        if let Ok(find_map) = open_find_map(&find_path, source_len, count) {
            return Ok(Self {
                map,
                find_map,
                count,
                journal_id,
                next_usn,
                root_id: stored_root_id,
                overlay: HashMap::new(),
                deleted: HashSet::new(),
            });
        }
        let mut ids = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        let mut masks = Vec::with_capacity(count);
        let mut id = 0_u64;
        for _ in 0..count {
            offsets.push(u32::try_from(cursor.position).context("index exceeds 4 GiB")?);
            id = id.saturating_add(cursor.varint()?);
            ids.push(id);
            cursor.varint()?;
            cursor.skip(1)?;
            cursor.varint()?;
            cursor.varint()?;
            let name_len = cursor.varint()? as usize;
            let name = cursor.take(name_len)?;
            masks.push(ascii_mask(name));
        }
        const RECORD_NUMBER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
        let root_record_number = stored_root_id & RECORD_NUMBER_MASK;
        let root_id = ids
            .iter()
            .copied()
            .find(|id| id & RECORD_NUMBER_MASK == root_record_number)
            .unwrap_or(stored_root_id);
        build_find_map(&find_path, source_len, &ids, &offsets, &masks)?;
        let find_map = open_find_map(&find_path, source_len, count)?;
        Ok(Self {
            map,
            find_map,
            count,
            journal_id,
            next_usn,
            root_id,
            overlay: HashMap::new(),
            deleted: HashSet::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.count + self.overlay.len() - self.deleted.len().min(self.count)
    }

    pub fn for_each_name_candidate(
        &self,
        required_mask: u64,
        mut visit: impl FnMut(RecordRef<'_>) -> bool,
    ) {
        let mut visit_index = |index: usize| {
            let id = self.row_id(index);
            if self.deleted.contains(&id) || self.overlay.contains_key(&id) {
                return true;
            }
            let name_mask = self.row_u64(index, 12);
            if name_mask & required_mask != required_mask {
                return true;
            }
            let mut cursor = Cursor {
                bytes: &self.map,
                position: self.row_u32(index, 8) as usize,
            };
            if cursor.varint().is_err() {
                return false;
            }
            let Ok(record) = parse_after_id(&mut cursor, id) else {
                return false;
            };
            visit(record)
        };
        let required_bits: Vec<_> = (0..POSTING_BITS)
            .filter(|bit| required_mask & (1_u64 << bit) != 0)
            .collect();
        if !required_bits.is_empty() {
            let bitmap_bytes = self.count.div_ceil(8);
            let bitmap_start = FIND_HEADER + self.count * FIND_ROW;
            for byte_index in 0..bitmap_bytes {
                let mut matches = 0xff_u8;
                for &bit in &required_bits {
                    matches &= self.find_map[bitmap_start + bit * bitmap_bytes + byte_index];
                    if matches == 0 {
                        break;
                    }
                }
                while matches != 0 {
                    let bit = matches.trailing_zeros() as usize;
                    let ordinal = byte_index * 8 + bit;
                    if ordinal < self.count && !visit_index(ordinal) {
                        return;
                    }
                    matches &= matches - 1;
                }
            }
        } else {
            for index in 0..self.count {
                if !visit_index(index) {
                    return;
                }
            }
        }
        for (&id, record) in &self.overlay {
            if !self.deleted.contains(&id)
                && ascii_mask(record.name.as_bytes()) & required_mask == required_mask
                && !visit(RecordRef {
                    id,
                    parent: record.parent,
                    name: &record.name,
                    is_dir: record.is_dir,
                    size: record.size,
                    modified: record.modified,
                })
            {
                return;
            }
        }
    }

    pub fn record(&self, id: u64) -> Option<RecordRef<'_>> {
        if self.deleted.contains(&id) {
            return None;
        }
        if let Some(record) = self.overlay.get(&id) {
            return Some(RecordRef {
                id,
                parent: record.parent,
                name: &record.name,
                is_dir: record.is_dir,
                size: record.size,
                modified: record.modified,
            });
        }
        let index = self.find_id(id)?;
        let mut cursor = Cursor {
            bytes: &self.map,
            position: self.row_u32(index, 8) as usize,
        };
        cursor.varint().ok()?;
        parse_after_id(&mut cursor, id).ok()
    }

    fn row_start(&self, index: usize) -> usize {
        FIND_HEADER + index * FIND_ROW
    }
    fn row_id(&self, index: usize) -> u64 {
        self.row_u64(index, 0)
    }
    fn row_u32(&self, index: usize, offset: usize) -> u32 {
        let start = self.row_start(index) + offset;
        u32::from_le_bytes(self.find_map[start..start + 4].try_into().unwrap())
    }
    fn row_u64(&self, index: usize, offset: usize) -> u64 {
        let start = self.row_start(index) + offset;
        u64::from_le_bytes(self.find_map[start..start + 8].try_into().unwrap())
    }
    fn find_id(&self, id: u64) -> Option<usize> {
        let mut low = 0;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            match self.row_id(middle).cmp(&id) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }
    pub fn path_for(&self, id: u64, root: &Path) -> Option<PathBuf> {
        const RECORD_NUMBER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
        let root_record_number = self.root_id & RECORD_NUMBER_MASK;
        if id & RECORD_NUMBER_MASK == root_record_number {
            return Some(root.to_path_buf());
        }
        let mut current = id;
        let mut components = Vec::new();
        let mut seen = HashSet::new();
        while current & RECORD_NUMBER_MASK != root_record_number {
            if !seen.insert(current) {
                return None;
            }
            let record = self.record(current)?;
            components.push(record.name.to_owned());
            current = record.parent;
        }
        let mut path = root.to_path_buf();
        for component in components.into_iter().rev() {
            path.push(component);
        }
        Some(path)
    }

    pub fn apply(&mut self, change: JournalRecord) {
        if change.reason & REASON_FILE_DELETE != 0 {
            self.overlay.remove(&change.file_reference);
            self.deleted.insert(change.file_reference);
            return;
        }
        if change.reason & REASON_RENAME_OLD_NAME != 0
            && change.reason & REASON_RENAME_NEW_NAME == 0
        {
            return;
        }
        if change.reason & (REASON_FILE_CREATE | REASON_RENAME_NEW_NAME) != 0 {
            let old = self.record(change.file_reference);
            let size = old.as_ref().map_or(0, |record| record.size);
            let modified = old.and_then(|record| record.modified);
            self.deleted.remove(&change.file_reference);
            self.overlay.insert(
                change.file_reference,
                OverlayRecord {
                    parent: change.parent_reference,
                    name: change.name,
                    is_dir: change.is_dir,
                    size,
                    modified,
                },
            );
        }
    }
}

fn open_find_map(path: &Path, source_len: u64, count: usize) -> anyhow::Result<Mmap> {
    let file = File::open(path)?;
    let map = unsafe { memmap2::MmapOptions::new().map(&file) }?;
    anyhow::ensure!(
        map.get(..8) == Some(FIND_MAGIC.as_slice()),
        "invalid search map"
    );
    anyhow::ensure!(read_u64(&map, 8) == source_len, "stale search map");
    anyhow::ensure!(read_u64(&map, 16) == count as u64, "stale search map");
    anyhow::ensure!(
        map.len() == FIND_HEADER + count * FIND_ROW + POSTING_BITS * count.div_ceil(8),
        "truncated search map"
    );
    Ok(map)
}

fn build_find_map(
    path: &Path,
    source_len: u64,
    ids: &[u64],
    offsets: &[u32],
    name_masks: &[u64],
) -> anyhow::Result<()> {
    eprintln!("building persistent fuzzy lookup for {} records", ids.len());
    let temporary = path.with_extension("atlasfind.tmp");
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, File::create(&temporary)?);
    writer.write_all(FIND_MAGIC)?;
    writer.write_all(&source_len.to_le_bytes())?;
    writer.write_all(&(ids.len() as u64).to_le_bytes())?;
    for index in 0..ids.len() {
        writer.write_all(&ids[index].to_le_bytes())?;
        writer.write_all(&offsets[index].to_le_bytes())?;
        writer.write_all(&name_masks[index].to_le_bytes())?;
    }
    let bitmap_bytes = ids.len().div_ceil(8);
    for bit in 0..POSTING_BITS {
        let mut bitmap = vec![0_u8; bitmap_bytes];
        for (ordinal, &mask) in name_masks.iter().enumerate() {
            if mask & (1_u64 << bit) != 0 {
                bitmap[ordinal / 8] |= 1 << (ordinal % 8);
            }
        }
        writer.write_all(&bitmap)?;
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub fn ascii_mask(bytes: &[u8]) -> u64 {
    let mut mask = 0_u64;
    for &byte in bytes {
        let byte = byte.to_ascii_lowercase();
        let bit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'z' => 10 + byte - b'a',
            0x80..=0xff => 63,
            _ => continue,
        };
        mask |= 1_u64 << bit;
    }
    mask
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn parse_after_id<'a>(cursor: &mut Cursor<'a>, id: u64) -> anyhow::Result<RecordRef<'a>> {
    let parent = cursor.varint()?;
    let is_dir = cursor.take(1)?[0] != 0;
    let size = cursor.varint()?;
    let modified = match cursor.varint()? {
        0 => None,
        value => Some(value - 1),
    };
    let name_len = cursor.varint()? as usize;
    let name = std::str::from_utf8(cursor.take(name_len)?)?;
    Ok(RecordRef {
        id,
        parent,
        name,
        is_dir,
        size,
        modified,
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, len: usize) -> anyhow::Result<&'a [u8]> {
        let end = self.position.checked_add(len).context("index overflow")?;
        let value = self
            .bytes
            .get(self.position..end)
            .context("truncated index")?;
        self.position = end;
        Ok(value)
    }
    fn skip(&mut self, len: usize) -> anyhow::Result<()> {
        self.take(len).map(|_| ())
    }
    fn varint(&mut self) -> anyhow::Result<u64> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = self.take(1)?[0];
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        anyhow::bail!("invalid varint")
    }
}
const fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}
