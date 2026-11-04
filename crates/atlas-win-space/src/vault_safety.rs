//! Handle-based checks used before retiring plaintext during vault migration.
use std::{
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FileDispositionInfo, FindClose, FindFirstStreamW, FindNextStreamW,
        FindStreamInfoStandard, GetFileInformationByHandle, SetFileInformationByHandle,
        BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING, WIN32_FIND_STREAM_DATA,
    },
};

pub fn validate_source(path: &Path, directory: bool) -> io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    let error = if ok == 0 {
        Some(io::Error::last_os_error())
    } else {
        None
    };
    unsafe {
        CloseHandle(handle);
    }
    if let Some(e) = error {
        return Err(e);
    }
    if info.dwFileAttributes & 0x400 != 0 || (!directory && info.nNumberOfLinks != 1) {
        return Err(io::Error::other("vault_unsafe_link"));
    }
    let mut data: WIN32_FIND_STREAM_DATA = unsafe { zeroed() };
    let stream = unsafe {
        FindFirstStreamW(
            wide.as_ptr(),
            FindStreamInfoStandard,
            (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(),
            0,
        )
    };
    if stream == INVALID_HANDLE_VALUE {
        let e = io::Error::last_os_error();
        // Filesystems without named streams return INVALID_FUNCTION or EOF.
        return if matches!(e.raw_os_error(), Some(1 | 38)) {
            Ok(())
        } else {
            Err(e)
        };
    }
    let result = loop {
        let len = data
            .cStreamName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(data.cStreamName.len());
        if String::from_utf16_lossy(&data.cStreamName[..len]) != "::$DATA" {
            break Err(io::Error::other("vault_unsafe_link"));
        }
        if unsafe { FindNextStreamW(stream, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast()) }
            == 0
        {
            let error = io::Error::last_os_error();
            break if error.raw_os_error() == Some(38) {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    unsafe {
        FindClose(stream);
    }
    result
}

/// The caller has hashed this exact handle while denying writers/deleters.
/// Marking that handle prevents deleting a substituted path between check/delete.
pub fn delete_on_close(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle() as _,
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
