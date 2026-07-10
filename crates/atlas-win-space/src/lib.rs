#![cfg(windows)]

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::Instant;
use windows_sys::Wdk::Storage::FileSystem::{MFT_ENUM_DATA, USN_RECORD};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_HANDLE_EOF, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdType, GetFileInformationByHandle, GetVolumeInformationW, OpenFileById,
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_DESCRIPTOR,
    FILE_ID_DESCRIPTOR_0, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME};

const BUFFER_SIZE: usize = 1024 * 1024;
const SNAPSHOT_MS: u128 = 250;

#[derive(Clone, Debug)]
pub struct WinSpaceItem {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub enum ScanError {
    Unsupported,
    Win32,
    Cancelled,
}

pub fn type_name_for_path(path: &Path) -> Option<String> {
    let wide = wide(path.as_os_str());
    let mut info = unsafe { zeroed::<SHFILEINFOW>() };
    let ok = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_TYPENAME,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = info
        .szTypeName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szTypeName.len());
    (len > 0).then(|| String::from_utf16_lossy(&info.szTypeName[..len]))
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Clone)]
struct Node {
    parent: u64,
    name: String,
    is_dir: bool,
}

pub fn scan_ntfs<F, S>(root: &Path, mut cancelled: F, mut snapshot: S) -> Result<(), ScanError>
where
    F: FnMut() -> bool,
    S: FnMut(Vec<WinSpaceItem>, u64, u64, u64, bool) -> bool,
{
    let volume = volume_letter(root).ok_or(ScanError::Unsupported)?;
    if !is_ntfs_root(volume) {
        return Err(ScanError::Unsupported);
    }
    let target_id = file_id(root).ok_or(ScanError::Win32)?;
    let volume_handle = open_volume(volume).ok_or(ScanError::Win32)?;
    let nodes = enum_usn_nodes(volume_handle.0, &mut cancelled, &mut snapshot)?;
    let mut top_cache = HashMap::<u64, Option<u64>>::new();
    let mut items = HashMap::<u64, WinSpaceItem>::new();
    let mut scanned_bytes = 0_u64;
    let mut scanned_entries = 0_u64;
    let mut error_count = 0_u64;
    let mut last_snapshot = Instant::now();

    for (&id, node) in &nodes {
        if cancelled() {
            return Err(ScanError::Cancelled);
        }
        let Some(top_id) = top_child(id, target_id, &nodes, &mut top_cache) else {
            continue;
        };
        let Some(top) = nodes.get(&top_id) else {
            continue;
        };
        let item = items.entry(top_id).or_insert_with(|| WinSpaceItem {
            path: root.join(&top.name),
            name: top.name.clone(),
            is_dir: top.is_dir,
            size: 0,
        });
        if !node.is_dir {
            scanned_entries += 1;
            match file_size_by_id(volume_handle.0, id) {
                Some(size) => {
                    scanned_bytes = scanned_bytes.saturating_add(size);
                    item.size = item.size.saturating_add(size);
                }
                None => error_count += 1,
            }
        }
        if last_snapshot.elapsed().as_millis() >= SNAPSHOT_MS {
            if !snapshot(
                items.values().cloned().collect(),
                scanned_bytes,
                scanned_entries,
                error_count,
                false,
            ) {
                return Err(ScanError::Cancelled);
            }
            last_snapshot = Instant::now();
        }
    }

    let _ = snapshot(
        items.values().cloned().collect(),
        scanned_bytes,
        scanned_entries,
        error_count,
        true,
    );
    Ok(())
}

fn enum_usn_nodes<F, S>(
    volume: HANDLE,
    cancelled: &mut F,
    snapshot: &mut S,
) -> Result<HashMap<u64, Node>, ScanError>
where
    F: FnMut() -> bool,
    S: FnMut(Vec<WinSpaceItem>, u64, u64, u64, bool) -> bool,
{
    let mut data = MFT_ENUM_DATA {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: i64::MAX,
        MinMajorVersion: 2,
        MaxMajorVersion: 2,
    };
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    let mut nodes = HashMap::new();
    let mut scanned_entries = 0_u64;
    let mut last_snapshot = Instant::now();

    loop {
        if cancelled() {
            return Err(ScanError::Cancelled);
        }
        let mut returned = 0_u32;
        let ok = unsafe {
            DeviceIoControl(
                volume,
                FSCTL_ENUM_USN_DATA,
                (&data as *const MFT_ENUM_DATA).cast(),
                size_of::<MFT_ENUM_DATA>() as u32,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return if err == ERROR_HANDLE_EOF {
                Ok(nodes)
            } else {
                Err(ScanError::Win32)
            };
        }
        if returned as usize <= size_of::<u64>() {
            return Ok(nodes);
        }
        data.StartFileReferenceNumber = u64::from_ne_bytes(
            buffer[..size_of::<u64>()]
                .try_into()
                .map_err(|_| ScanError::Win32)?,
        );
        let mut offset = size_of::<u64>();
        while offset + size_of::<USN_RECORD>() <= returned as usize {
            let record = unsafe { &*(buffer.as_ptr().add(offset).cast::<USN_RECORD>()) };
            if record.RecordLength == 0 {
                break;
            }
            let name_start = offset + record.FileNameOffset as usize;
            let name_len = record.FileNameLength as usize / 2;
            if name_start + name_len * 2 <= returned as usize {
                let name_ptr = unsafe { buffer.as_ptr().add(name_start).cast::<u16>() };
                let name = String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(name_ptr, name_len)
                });
                let is_dir = record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                let is_reparse = record.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                if !is_reparse {
                    nodes.insert(
                        record.FileReferenceNumber,
                        Node {
                            parent: record.ParentFileReferenceNumber,
                            name,
                            is_dir,
                        },
                    );
                    scanned_entries += 1;
                }
            }
            offset += record.RecordLength as usize;
        }
        if last_snapshot.elapsed().as_millis() >= SNAPSHOT_MS {
            let _ = snapshot(Vec::new(), 0, scanned_entries, 0, false);
            last_snapshot = Instant::now();
        }
    }
}

fn top_child(
    id: u64,
    target: u64,
    nodes: &HashMap<u64, Node>,
    cache: &mut HashMap<u64, Option<u64>>,
) -> Option<u64> {
    if let Some(cached) = cache.get(&id) {
        return *cached;
    }
    if id == target {
        cache.insert(id, None);
        return None;
    }
    let mut cur = id;
    let mut seen = HashSet::new();
    loop {
        let node = nodes.get(&cur)?;
        if node.parent == target {
            cache.insert(id, Some(cur));
            return Some(cur);
        }
        if node.parent == cur || !seen.insert(cur) {
            cache.insert(id, None);
            return None;
        }
        cur = node.parent;
    }
}

fn file_size_by_id(volume: HANDLE, id: u64) -> Option<u64> {
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: id as i64 },
    };
    let handle = unsafe {
        OpenFileById(
            volume,
            &descriptor,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let handle = Handle(handle);
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle.0, &mut info) };
    (ok != 0).then(|| (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow))
}

fn file_id(path: &Path) -> Option<u64> {
    let handle = open_path(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle.0, &mut info) };
    (ok != 0).then(|| (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
}

fn open_path(path: &Path) -> Option<Handle> {
    let wide = wide(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            0,
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
}

fn open_volume(letter: char) -> Option<Handle> {
    let path = format!(r"\\.\{}:", letter.to_ascii_uppercase());
    let wide = wide(OsStr::new(&path));
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
}

fn is_ntfs_root(letter: char) -> bool {
    let root = format!("{}:\\", letter.to_ascii_uppercase());
    let root = wide(OsStr::new(&root));
    let mut fs_name = [0_u16; 16];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    ok != 0 && String::from_utf16_lossy(&fs_name).trim_end_matches('\0') == "NTFS"
}

fn volume_letter(path: &Path) -> Option<char> {
    let s = path.as_os_str().to_string_lossy();
    let bytes = s.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':')
        .then(|| bytes[0] as char)
        .filter(|c| c.is_ascii_alphabetic())
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}
