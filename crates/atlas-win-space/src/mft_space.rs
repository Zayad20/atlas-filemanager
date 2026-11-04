//! Read-only NTFS 3.x MFT streaming. Sizes come from $DATA attributes, never
//! OpenFileById per file. Unsupported layouts fail back to native enumeration.
//! Format references: Microsoft FILE_RECORD_SEGMENT_HEADER,
//! ATTRIBUTE_RECORD_HEADER, MULTI_SECTOR_HEADER and FSCTL_GET_NTFS_VOLUME_DATA.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{Ioctl::FSCTL_GET_NTFS_VOLUME_DATA, IO::DeviceIoControl},
};

const BLOCK: usize = 4 * 1024 * 1024;
const BUDGET: usize = 768 * 1024 * 1024;
const MAX_RECORDS: usize = 8_000_000;
const MASK: u64 = (1 << 48) - 1;
pub mod helper;

pub fn supported(root: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
    let name: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut fs = [0u16; 16];
    (unsafe {
        GetVolumeInformationW(
            name.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            16,
        )
    }) != 0
        && String::from_utf16_lossy(&fs).trim_end_matches('\0') == "NTFS"
}

#[derive(Clone, Debug)]
pub struct Record {
    pub reference: u64,
    pub parent: u64,
    pub name: String,
    pub bytes: u64,
    pub attributes: u32,
    pub directory: bool,
}
pub struct Index {
    pub records: Vec<Record>,
    pub skipped: u64,
}
#[derive(Clone, Copy, Debug)]
struct Run {
    logical: u64,
    physical: u64,
    bytes: u64,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Unsupported or inconsistent NTFS metadata",
    )
}
fn u16_at(b: &[u8], at: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2).ok_or_else(invalid)?.try_into().unwrap(),
    ))
}
fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).ok_or_else(invalid)?.try_into().unwrap(),
    ))
}
fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(at..at + 8).ok_or_else(invalid)?.try_into().unwrap(),
    ))
}

fn fixup(record: &mut [u8]) -> io::Result<()> {
    if record.get(..4) != Some(b"FILE") || record.len() < 48 {
        return Err(invalid());
    }
    let offset = u16_at(record, 4)? as usize;
    let count = u16_at(record, 6)? as usize;
    // NTFS update sequence protection uses 512-byte strides, including 4Kn.
    if count < 2 || (count - 1) * 512 != record.len() || offset < 8 || offset + count * 2 > 510 {
        return Err(invalid());
    }
    let marker = u16_at(record, offset)?;
    for sector in 1..count {
        let at = sector * 512 - 2;
        if u16_at(record, at)? != marker {
            return Err(invalid());
        }
        let replacement = u16_at(record, offset + sector * 2)?.to_le_bytes();
        record[at..at + 2].copy_from_slice(&replacement);
    }
    Ok(())
}

fn attributes(record: &[u8], mut emit: impl FnMut(u32, &[u8]) -> io::Result<()>) -> io::Result<()> {
    let used = u32_at(record, 24)? as usize;
    let mut at = u16_at(record, 20)? as usize;
    if used > record.len() || at < 48 || at > used {
        return Err(invalid());
    }
    while at + 4 <= used {
        let kind = u32_at(record, at)?;
        if kind == u32::MAX {
            return Ok(());
        }
        let length = u32_at(record, at + 4)? as usize;
        if length < 24 || !length.is_multiple_of(8) || at + length > used {
            return Err(invalid());
        }
        let attr = &record[at..at + length];
        if attr[8] > 1 {
            return Err(invalid());
        }
        emit(kind, attr)?;
        at += length;
    }
    Err(invalid())
}
fn resident(attr: &[u8]) -> io::Result<&[u8]> {
    if attr[8] != 0 {
        return Err(invalid());
    }
    let len = u32_at(attr, 16)? as usize;
    let offset = u16_at(attr, 20)? as usize;
    if offset < 24 {
        return Err(invalid());
    }
    attr.get(offset..offset.checked_add(len).ok_or_else(invalid)?)
        .ok_or_else(invalid)
}

fn runs(attr: &[u8], cluster: u64, volume_bytes: u64) -> io::Result<Vec<Run>> {
    if attr[8] != 1 || attr[9] != 0 || u64_at(attr, 16)? != 0 {
        return Err(invalid());
    }
    let mut at = u16_at(attr, 32)? as usize;
    if at < 64 || at >= attr.len() {
        return Err(invalid());
    }
    let mut lcn = 0i64;
    let mut logical = 0u64;
    let mut result = Vec::new();
    loop {
        let header = *attr.get(at).ok_or_else(invalid)?;
        at += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 15) as usize;
        let off_size = (header >> 4) as usize;
        // Sparse/ambiguous extents cannot be used to read $MFT.
        if !(1..=8).contains(&len_size) || !(1..=8).contains(&off_size) {
            return Err(invalid());
        }
        let end = at.checked_add(len_size + off_size).ok_or_else(invalid)?;
        let encoded = attr.get(at..end).ok_or_else(invalid)?;
        let mut count = [0; 8];
        count[..len_size].copy_from_slice(&encoded[..len_size]);
        let mut delta = if encoded[len_size + off_size - 1] & 0x80 != 0 {
            [0xff; 8]
        } else {
            [0; 8]
        };
        delta[..off_size].copy_from_slice(&encoded[len_size..]);
        lcn = lcn
            .checked_add(i64::from_le_bytes(delta))
            .filter(|n| *n >= 0)
            .ok_or_else(invalid)?;
        let bytes = u64::from_le_bytes(count)
            .checked_mul(cluster)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let physical = (lcn as u64).checked_mul(cluster).ok_or_else(invalid)?;
        if physical.checked_add(bytes).is_none_or(|n| n > volume_bytes) || result.len() >= 65_536 {
            return Err(invalid());
        }
        result.push(Run {
            logical,
            physical,
            bytes,
        });
        logical = logical.checked_add(bytes).ok_or_else(invalid)?;
        at = end;
    }
    if result.is_empty() {
        return Err(invalid());
    }
    Ok(result)
}

fn parse_record(record: &[u8], number: u64) -> io::Result<Option<Record>> {
    let flags = u16_at(record, 22)?;
    if flags & 1 == 0 || u64_at(record, 32)? != 0 {
        return Ok(None);
    }
    let sequence = u16_at(record, 16)? as u64;
    if sequence == 0 {
        return Err(invalid());
    }
    let mut name = None;
    let mut namespace = 0u8;
    let mut parent = 0;
    let mut attrs = 0;
    let mut bytes = 0;
    let mut data_found = false;
    let mut extension = false;
    attributes(record, |kind, attr| {
        match kind {
            0x10 => {
                let value = resident(attr)?;
                attrs = u32_at(value, 32)?;
            }
            0x20 => extension = true,
            0x30 => {
                let value = resident(attr)?;
                if value.len() < 66 {
                    return Err(invalid());
                }
                let ns = value[65];
                let count = value[64] as usize;
                let encoded = value.get(66..66 + count * 2).ok_or_else(invalid)?;
                // Prefer Win32/Win32+DOS over a DOS alias. One identity is counted
                // once; hardlink aliases are not summed as separate file bytes.
                let priority = match ns {
                    1 | 3 => 3,
                    0 => 2,
                    2 => 1,
                    _ => return Err(invalid()),
                };
                if priority > namespace {
                    let units: Vec<u16> = encoded
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    if units.is_empty() || units.iter().any(|u| matches!(u, 0 | 47 | 58 | 92)) {
                        return Err(invalid());
                    }
                    name = Some(String::from_utf16_lossy(&units));
                    parent = u64_at(value, 0)?;
                    namespace = priority;
                }
            }
            0x80 if attr[9] == 0 => {
                if attr[8] == 0 {
                    bytes = resident(attr)?.len() as u64;
                    data_found = true;
                } else if u64_at(attr, 16)? == 0 {
                    bytes = u64_at(attr, 48)?;
                    data_found = true;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let Some(name) = name else {
        return Ok(None);
    };
    let directory = flags & 2 != 0;
    if bytes > i64::MAX as u64 {
        return Err(invalid());
    }
    if extension && !directory && !data_found {
        return Err(invalid());
    }
    Ok(Some(Record {
        reference: number | sequence << 48,
        parent,
        name,
        bytes: if directory { 0 } else { bytes },
        attributes: attrs,
        directory,
    }))
}

/// Requires an already elevated process. A normal-user access denial is an
/// expected fallback, not a reason to elevate the file manager automatically.
pub fn scan(
    root: &Path,
    mut control: impl FnMut() -> bool,
    mut progress: impl FnMut(u64, u64),
) -> io::Result<Index> {
    if !control() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Scan cancelled"));
    }
    let text = root.to_string_lossy();
    let letter = text
        .as_bytes()
        .first()
        .copied()
        .filter(u8::is_ascii_alphabetic)
        .ok_or_else(invalid)?;
    if text.len() != 3 || !text.ends_with(['\\', '/']) || text.as_bytes()[1] != b':' {
        return Err(invalid());
    }
    let device: Vec<u16> = format!(r"\\.\{}:", letter as char)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            device.as_ptr(),
            0x8000_0000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_SEQUENTIAL_SCAN,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut volume = unsafe { File::from_raw_handle(handle as *mut _) };
    // A SUBST drive can name a subdirectory of a different volume. Its file
    // reference must be the actual NTFS root (record 5), or raw-volume indexing
    // would display metadata outside the requested drive's namespace.
    let root_wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let root_handle = unsafe {
        CreateFileW(
            root_wide.as_ptr(),
            0x80,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            0x0200_0000 | 0x0020_0000,
            0,
        )
    };
    if root_handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let root_file = unsafe { File::from_raw_handle(root_handle as *mut _) };
    let mut root_info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
            root_file.as_raw_handle() as HANDLE,
            &mut root_info,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if ((u64::from(root_info.nFileIndexHigh) << 32) | u64::from(root_info.nFileIndexLow)) & MASK
        != 5
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS root is a substituted directory",
        ));
    }
    let mut info = [0u8; 512];
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            volume.as_raw_handle() as HANDLE,
            FSCTL_GET_NTFS_VOLUME_DATA,
            std::ptr::null(),
            0,
            info.as_mut_ptr().cast(),
            info.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned < 104 || u16_at(&info, 100)? != 3 || u16_at(&info, 102)? > 1 {
        return Err(invalid());
    }
    let sector = u32_at(&info, 40)? as usize;
    let cluster = u32_at(&info, 44)? as u64;
    let record_size = u32_at(&info, 48)? as usize;
    let length = u64_at(&info, 56)?;
    let volume_bytes = u64_at(&info, 8)?
        .checked_mul(sector as u64)
        .ok_or_else(invalid)?;
    if !sector.is_power_of_two()
        || !(512..=4096).contains(&sector)
        || !cluster.is_power_of_two()
        || cluster < sector as u64
        || !record_size.is_power_of_two()
        || !(512..=65536).contains(&record_size)
        || length == 0
        || !length.is_multiple_of(record_size as u64)
        || length > 16 * 1024 * 1024 * 1024
    {
        return Err(invalid());
    }
    let mut first = vec![0u8; record_size.max(sector)];
    let at = u64_at(&info, 64)?
        .checked_mul(cluster)
        .ok_or_else(invalid)?;
    volume.seek(SeekFrom::Start(at))?;
    volume.read_exact(&mut first)?;
    let first = &mut first[..record_size];
    fixup(first)?;
    let mut extents = None;
    attributes(first, |kind, attr| {
        if kind == 0x80 && attr[9] == 0 {
            extents = Some(runs(attr, cluster, volume_bytes)?);
        }
        Ok(())
    })?;
    let extents = extents.ok_or_else(invalid)?;
    if extents.last().is_none_or(|r| r.logical + r.bytes < length) {
        return Err(invalid());
    }
    let mut records = Vec::new();
    let mut names = 0usize;
    let mut skipped = 0;
    let mut buffer = vec![0u8; BLOCK.max(record_size).max(sector)];
    let mut logical = 0u64;
    while logical < length {
        if !control() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Scan cancelled"));
        }
        let count = (length - logical).min(buffer.len() as u64) as usize;
        let mut filled = 0;
        while filled < count {
            let offset = logical + filled as u64;
            let run = extents
                .iter()
                .find(|r| offset >= r.logical && offset < r.logical + r.bytes)
                .ok_or_else(invalid)?;
            let n = (run.logical + run.bytes - offset).min((count - filled) as u64) as usize;
            volume.seek(SeekFrom::Start(run.physical + offset - run.logical))?;
            volume.read_exact(&mut buffer[filled..filled + n])?;
            filled += n;
        }
        for (i, record) in buffer[..count].chunks_exact_mut(record_size).enumerate() {
            if record.get(..4) != Some(b"FILE") {
                continue;
            }
            let number = logical / record_size as u64 + i as u64;
            if number > MASK {
                return Err(invalid());
            }
            match fixup(record).and_then(|_| parse_record(record, number)) {
                Ok(Some(record)) => {
                    names += record.name.len();
                    if records.len() >= MAX_RECORDS
                        || records.len() * std::mem::size_of::<Record>() + names > BUDGET
                    {
                        return Err(io::Error::other("NTFS index memory budget exceeded"));
                    }
                    records.push(record);
                }
                Ok(None) => {}
                Err(_) => skipped += 1,
            }
        }
        logical += count as u64;
        progress(logical, length);
    }
    if !records
        .iter()
        .any(|r| r.directory && r.reference & MASK == 5)
    {
        return Err(io::Error::other("NTFS root record could not be verified"));
    }
    Ok(Index { records, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resident_file_name_identity_and_data_size_are_read_from_one_record() {
        let mut record = vec![0u8; 1024];
        record[..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[16..18].copy_from_slice(&4u16.to_le_bytes());
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[22..24].copy_from_slice(&1u16.to_le_bytes());
        let mut offset = 56;
        let mut add = |kind: u32, value: &[u8]| {
            let size = (24 + value.len() + 7) & !7;
            record[offset..offset + 4].copy_from_slice(&kind.to_le_bytes());
            record[offset + 4..offset + 8].copy_from_slice(&(size as u32).to_le_bytes());
            record[offset + 16..offset + 20].copy_from_slice(&(value.len() as u32).to_le_bytes());
            record[offset + 20..offset + 22].copy_from_slice(&24u16.to_le_bytes());
            record[offset + 24..offset + 24 + value.len()].copy_from_slice(value);
            offset += size;
        };
        let mut standard = [0; 72];
        standard[32..36].copy_from_slice(&0x20u32.to_le_bytes());
        add(0x10, &standard);
        let mut name = vec![0; 80];
        name[..8].copy_from_slice(&(5u64 | (1 << 48)).to_le_bytes());
        name[64] = 7;
        name[65] = 1;
        for (i, u) in "old.tmp".encode_utf16().enumerate() {
            name[66 + i * 2..68 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        add(0x30, &name);
        add(0x80, b"abc");
        record[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        record[24..28].copy_from_slice(&((offset + 8) as u32).to_le_bytes());
        record[48..54].copy_from_slice(&[0x34, 0x12, 0, 0, 0, 0]);
        record[510..512].copy_from_slice(&[0x34, 0x12]);
        record[1022..1024].copy_from_slice(&[0x34, 0x12]);
        fixup(&mut record).unwrap();
        let parsed = parse_record(&record, 18).unwrap().unwrap();
        assert_eq!(parsed.reference, 18 | (4 << 48));
        assert_eq!(parsed.parent, 5 | (1 << 48));
        assert_eq!(parsed.bytes, 3);
        assert_eq!(parsed.name, "old.tmp");
        assert!(!parsed.directory);
    }
    #[test]
    fn update_sequence_rejects_torn_record_and_restores_sector_tails() {
        let mut record = vec![0; 1024];
        record[..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[48..54].copy_from_slice(&[0x34, 0x12, 0xaa, 0xbb, 0xcc, 0xdd]);
        record[510..512].copy_from_slice(&[0x34, 0x12]);
        record[1022..1024].copy_from_slice(&[0x34, 0x12]);
        let mut torn = record.clone();
        torn[510] = 0;
        assert!(fixup(&mut torn).is_err());
        fixup(&mut record).unwrap();
        assert_eq!(&record[510..512], &[0xaa, 0xbb]);
        assert_eq!(&record[1022..1024], &[0xcc, 0xdd]);
    }
    #[test]
    fn runlist_validates_signed_deltas_bounds_and_sparse_runs() {
        let mut attr = vec![0; 80];
        attr[8] = 1;
        attr[32..34].copy_from_slice(&64u16.to_le_bytes());
        attr[64..71].copy_from_slice(&[0x11, 2, 10, 0x11, 1, 0xfe, 0]);
        let list = runs(&attr, 4096, 1024 * 1024).unwrap();
        assert_eq!(list[0].physical, 40960);
        assert_eq!(list[1].physical, 32768);
        assert_eq!(list[1].logical, 8192);
        assert!(runs(&attr, 4096, 8192).is_err());
        attr[64] = 1;
        assert!(runs(&attr, 4096, 1024 * 1024).is_err());
    }
    #[test]
    fn malformed_attribute_lengths_never_escape_record() {
        let mut record = vec![0; 1024];
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[24..28].copy_from_slice(&128u32.to_le_bytes());
        record[56..60].copy_from_slice(&0x80u32.to_le_bytes());
        record[60..64].copy_from_slice(&2048u32.to_le_bytes());
        assert!(attributes(&record, |_, _| panic!("malformed attribute reached parser")).is_err());
    }
}
