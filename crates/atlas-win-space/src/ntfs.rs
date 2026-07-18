//! Low-level, metadata-only NTFS catalog access.
//!
//! This module opens a volume, never individual file contents. It enumerates MFT metadata and
//! reads the USN journal from a durable checkpoint supplied by the caller.

use std::ffi::OsStr;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use windows_sys::Wdk::Storage::FileSystem::{MFT_ENUM_DATA, USN_RECORD};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys_copy::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};

const BUFFER_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MftRecord {
    pub file_reference: u64,
    pub parent_reference: u64,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalState {
    pub id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
    pub lowest_valid_usn: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRecord {
    pub file_reference: u64,
    pub parent_reference: u64,
    pub usn: i64,
    pub reason: u32,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug)]
pub enum CheckpointError {
    JournalChanged,
    JournalTruncated,
    Io(io::Error),
}

impl std::fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JournalChanged => formatter.write_str("USN journal identity changed"),
            Self::JournalTruncated => formatter.write_str("USN checkpoint was truncated"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CheckpointError {}

impl From<io::Error> for CheckpointError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

struct Volume(HANDLE);

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn enumerate_mft(letter: char, mut emit: impl FnMut(MftRecord) -> bool) -> io::Result<()> {
    let volume = open_volume(letter)?;
    let mut request = MFT_ENUM_DATA {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: i64::MAX,
        MinMajorVersion: 2,
        MaxMajorVersion: 2,
    };
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    loop {
        let returned = ioctl(volume.0, FSCTL_ENUM_USN_DATA, &request, &mut buffer)?;
        if returned <= size_of::<u64>() {
            return Ok(());
        }
        request.StartFileReferenceNumber = u64::from_ne_bytes(
            buffer[..8]
                .try_into()
                .map_err(|_| io::Error::other("invalid MFT cursor"))?,
        );
        if !parse_records(&buffer[8..returned], |record| {
            emit(MftRecord {
                file_reference: record.file_reference,
                parent_reference: record.parent_reference,
                name: record.name,
                is_dir: record.is_dir,
            })
        }) {
            return Ok(());
        }
    }
}

pub fn query_journal(letter: char) -> io::Result<JournalState> {
    let volume = open_volume(letter)?;
    query_journal_handle(volume.0)
}

/// Reads all journal entries currently available after `start_usn`.
///
/// The returned state is the checkpoint to persist only after the caller has atomically stored
/// the applied records. A changed/truncated journal explicitly requests a volume rebuild.
pub fn read_journal(
    letter: char,
    expected_journal_id: u64,
    start_usn: i64,
    mut emit: impl FnMut(JournalRecord) -> bool,
) -> Result<JournalState, CheckpointError> {
    let volume = open_volume(letter)?;
    let state = query_journal_handle(volume.0)?;
    if state.id != expected_journal_id {
        return Err(CheckpointError::JournalChanged);
    }
    if start_usn < state.lowest_valid_usn || start_usn > state.next_usn {
        return Err(CheckpointError::JournalTruncated);
    }
    let mut cursor = start_usn;
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    while cursor < state.next_usn {
        let request = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: cursor,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: expected_journal_id,
        };
        let returned = ioctl(volume.0, FSCTL_READ_USN_JOURNAL, &request, &mut buffer)?;
        if returned <= size_of::<i64>() {
            break;
        }
        let next = i64::from_ne_bytes(
            buffer[..8]
                .try_into()
                .map_err(|_| io::Error::other("invalid USN cursor"))?,
        );
        if !parse_records(&buffer[8..returned], |record| {
            emit(JournalRecord {
                file_reference: record.file_reference,
                parent_reference: record.parent_reference,
                usn: record.usn,
                reason: record.reason,
                name: record.name,
                is_dir: record.is_dir,
            })
        }) {
            break;
        }
        if next <= cursor {
            return Err(io::Error::other("USN cursor did not advance").into());
        }
        cursor = next;
    }
    query_journal_handle(volume.0).map_err(Into::into)
}

struct ParsedRecord {
    file_reference: u64,
    parent_reference: u64,
    usn: i64,
    reason: u32,
    name: String,
    is_dir: bool,
}

fn parse_records(buffer: &[u8], mut emit: impl FnMut(ParsedRecord) -> bool) -> bool {
    let mut offset = 0;
    while offset + size_of::<USN_RECORD>() <= buffer.len() {
        let record = unsafe {
            buffer
                .as_ptr()
                .add(offset)
                .cast::<USN_RECORD>()
                .read_unaligned()
        };
        let length = record.RecordLength as usize;
        if length == 0 || offset + length > buffer.len() {
            break;
        }
        let name_offset = offset + record.FileNameOffset as usize;
        let name_units = record.FileNameLength as usize / 2;
        if name_offset + name_units * 2 <= buffer.len() {
            let name = String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().add(name_offset).cast::<u16>(),
                    name_units,
                )
            });
            if !emit(ParsedRecord {
                file_reference: record.FileReferenceNumber,
                parent_reference: record.ParentFileReferenceNumber,
                usn: record.Usn,
                reason: record.Reason,
                name,
                is_dir: record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            }) {
                return false;
            }
        }
        offset += length;
    }
    true
}

fn query_journal_handle(volume: HANDLE) -> io::Result<JournalState> {
    let mut state = USN_JOURNAL_DATA_V0::default();
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            volume,
            FSCTL_QUERY_USN_JOURNAL,
            null_mut(),
            0,
            (&raw mut state).cast(),
            size_of::<USN_JOURNAL_DATA_V0>() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(JournalState {
        id: state.UsnJournalID,
        first_usn: state.FirstUsn,
        next_usn: state.NextUsn,
        lowest_valid_usn: state.LowestValidUsn,
    })
}

fn ioctl<T>(handle: HANDLE, code: u32, input: &T, output: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            code,
            (input as *const T).cast(),
            size_of::<T>() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        // Enumeration signals completion with ERROR_HANDLE_EOF.
        if code == 38 {
            return Ok(0);
        }
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    Ok(returned as usize)
}

fn open_volume(letter: char) -> io::Result<Volume> {
    if !letter.is_ascii_alphabetic() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid volume letter",
        ));
    }
    let path = format!(r"\\.\{}:", letter.to_ascii_uppercase());
    let wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0x8000_0000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Volume(handle))
    }
}
