//! User-invoked, read-only elevated reader. The only command is a drive letter;
//! output travels over a random local pipe, never an elevated filesystem write.
use super::{Index, Record, BUDGET, MAX_RECORDS};
use std::{
    ffi::OsStr,
    io::{self, BufReader, BufWriter, Read, Write},
    mem::zeroed,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG},
    Storage::FileSystem::{
        CreateFileW, ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{CreateEventW, GetProcessId, ResetEvent, WaitForSingleObject},
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
    },
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    },
};
const MAGIC: &[u8; 9] = b"ATLASMFT1";
const PREFIX: &str = r"\\.\pipe\AtlasCleanupMft-";
const COMMAND: &str = "--atlas-mft-reader";
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
fn error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid NTFS helper response")
}
fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn write_index(writer: &mut impl Write, index: &Index) -> io::Result<()> {
    writer.write_all(&[2])?;
    writer.write_all(&index.skipped.to_le_bytes())?;
    writer.write_all(&(index.records.len() as u32).to_le_bytes())?;
    for record in &index.records {
        writer.write_all(&record.reference.to_le_bytes())?;
        writer.write_all(&record.parent.to_le_bytes())?;
        writer.write_all(&record.bytes.to_le_bytes())?;
        writer.write_all(&record.attributes.to_le_bytes())?;
        writer.write_all(&[u8::from(record.directory)])?;
        writer.write_all(&(record.name.len() as u16).to_le_bytes())?;
        writer.write_all(record.name.as_bytes())?;
    }
    writer.flush()
}
fn read_index(reader: &mut impl Read) -> io::Result<Index> {
    let skipped = read_u64(reader)?;
    let mut b = [0; 4];
    reader.read_exact(&mut b)?;
    let count = u32::from_le_bytes(b) as usize;
    if count > MAX_RECORDS {
        return Err(error());
    }
    let mut records = Vec::new();
    records.try_reserve_exact(count).map_err(|_| error())?;
    let mut names = 0usize;
    for _ in 0..count {
        let reference = read_u64(reader)?;
        let parent = read_u64(reader)?;
        let bytes = read_u64(reader)?;
        reader.read_exact(&mut b)?;
        let attributes = u32::from_le_bytes(b);
        let mut flag = [0];
        reader.read_exact(&mut flag)?;
        let mut n = [0; 2];
        reader.read_exact(&mut n)?;
        let length = u16::from_le_bytes(n) as usize;
        names = names.checked_add(length).ok_or_else(error)?;
        if flag[0] > 1
            || length == 0
            || length > 1024
            || bytes > i64::MAX as u64
            || count * std::mem::size_of::<Record>() + names > BUDGET
        {
            return Err(error());
        }
        let mut name = vec![0; length];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| error())?;
        if name.contains(['\0', '/', '\\', ':']) || name == ".." {
            return Err(error());
        }
        records.push(Record {
            reference,
            parent,
            bytes,
            attributes,
            directory: flag[0] != 0,
            name,
        });
    }
    Ok(Index { records, skipped })
}

/// Must run before GUI/logging initialization in the executable. It accepts no
/// paths, scripts, deletion verbs, or arbitrary pipe names.
pub fn try_run() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|s| s != COMMAND) {
        return false;
    }
    let run = || -> io::Result<()> {
        let args = args
            .iter()
            .map(|s| s.to_str().ok_or_else(error))
            .collect::<io::Result<Vec<_>>>()?;
        if args.len() != 3
            || args[1].len() != 1
            || !args[1].as_bytes()[0].is_ascii_alphabetic()
            || args[2].len() != 32
            || !args[2].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(error());
        }
        let pipe = wide(format!("{PREFIX}{}", args[2]));
        let handle = unsafe {
            CreateFileW(
                pipe.as_ptr(),
                0x4000_0000,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut writer = BufWriter::with_capacity(1024 * 1024, unsafe {
            std::fs::File::from_raw_handle(handle as *mut _)
        });
        writer.write_all(MAGIC)?;
        writer.flush()?;
        let alive = std::cell::Cell::new(true);
        let root = format!("{}:\\", args[1]);
        let result = super::scan(
            Path::new(&root),
            || alive.get(),
            |done, total| {
                let send = (|| -> io::Result<()> {
                    writer.write_all(&[1])?;
                    writer.write_all(&done.to_le_bytes())?;
                    writer.write_all(&total.to_le_bytes())?;
                    writer.flush()
                })();
                if send.is_err() {
                    alive.set(false);
                }
            },
        );
        match result {
            Ok(index) => write_index(&mut writer, &index),
            Err(error) => {
                writer.write_all(&[3])?;
                writer.write_all(&error.raw_os_error().unwrap_or(0).to_le_bytes())?;
                let message = error.to_string();
                let bytes = message.as_bytes();
                let length = bytes.len().min(4096);
                writer.write_all(&(length as u16).to_le_bytes())?;
                writer.write_all(&bytes[..length])?;
                writer.flush()
            }
        }
    };
    let _ = run();
    true
}

struct PipeReader<'a, F> {
    pipe: Owned,
    event: Owned,
    process: Owned,
    control: &'a mut F,
}
impl<F: FnMut() -> bool> PipeReader<'_, F> {
    fn wait(&mut self, operation: &mut OVERLAPPED) -> io::Result<()> {
        loop {
            let status = unsafe { WaitForSingleObject(self.event.0, 50) };
            if status == WAIT_OBJECT_0 {
                return Ok(());
            }
            let continuing = (self.control)();
            if status != WAIT_TIMEOUT
                || !continuing
                || unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0
            {
                unsafe {
                    CancelIoEx(self.pipe.0, operation);
                    let mut bytes = 0;
                    // Keep OVERLAPPED alive until the cancelled IO has drained.
                    GetOverlappedResult(self.pipe.0, operation, &mut bytes, 1);
                }
                return Err(io::Error::new(
                    if continuing {
                        io::ErrorKind::BrokenPipe
                    } else {
                        io::ErrorKind::Interrupted
                    },
                    "NTFS helper stopped",
                ));
            }
        }
    }
}
impl<F: FnMut() -> bool> Read for PipeReader<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !(self.control)() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }
        unsafe {
            ResetEvent(self.event.0);
        }
        let mut operation: OVERLAPPED = unsafe { zeroed() };
        operation.hEvent = self.event.0;
        let mut bytes = 0;
        let ok = unsafe {
            ReadFile(
                self.pipe.0,
                buffer.as_mut_ptr(),
                buffer.len().min(u32::MAX as usize) as u32,
                &mut bytes,
                &mut operation,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            if code != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
            self.wait(&mut operation)?;
            if unsafe { GetOverlappedResult(self.pipe.0, &operation, &mut bytes, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(bytes as usize)
    }
}

pub fn scan(
    root: &Path,
    mut control: impl FnMut() -> bool,
    mut progress: impl FnMut(u64, u64),
) -> io::Result<Index> {
    // The normal direct attempt avoids UAC for an already elevated process.
    match super::scan(root, &mut control, &mut progress) {
        Ok(index) => return Ok(index),
        Err(error) if error.kind() != io::ErrorKind::PermissionDenied => return Err(error),
        Err(_) => {}
    }
    let letter = root
        .to_string_lossy()
        .as_bytes()
        .first()
        .copied()
        .filter(u8::is_ascii_alphabetic)
        .ok_or_else(error)?;
    let mut random = [0u8; 16];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            random.as_mut_ptr(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err(error());
    }
    let token = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let name = wide(format!("{PREFIX}{token}"));
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            1 | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            4 * 1024 * 1024,
            1000,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let pipe = Owned(handle);
    let event = Owned(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) });
    if event.0 == 0 {
        return Err(io::Error::last_os_error());
    }
    let executable = wide(std::env::current_exe()?);
    // Shell activation is performed from a scanner thread, not the GUI's COM
    // apartment. Balance only a COM initialization that succeeded here.
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    windows::Win32::System::Com::CoUninitialize();
                }
            }
        }
    }
    let _apartment = Apartment(
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
        }
        .is_ok(),
    );
    let verb = wide("runas");
    let parameters = wide(format!("{COMMAND} {} {token}", letter as char));
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = std::mem::size_of_val(&execute) as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = executable.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    execute.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let process = Owned(execute.hProcess);
    if process.0 == 0 {
        return Err(error());
    }
    let mut reader = PipeReader {
        pipe,
        event,
        process,
        control: &mut control,
    };
    let mut operation: OVERLAPPED = unsafe { zeroed() };
    operation.hEvent = reader.event.0;
    if unsafe { ConnectNamedPipe(reader.pipe.0, &mut operation) } == 0 {
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => {}
            ERROR_IO_PENDING => reader.wait(&mut operation)?,
            code => return Err(io::Error::from_raw_os_error(code as i32)),
        }
    }
    let mut client = 0;
    if unsafe { GetNamedPipeClientProcessId(reader.pipe.0, &mut client) } == 0
        || client != unsafe { GetProcessId(reader.process.0) }
    {
        return Err(error());
    }
    let mut reader = BufReader::with_capacity(4 * 1024 * 1024, reader);
    let mut magic = [0; 9];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(error());
    }
    loop {
        let mut kind = [0];
        reader.read_exact(&mut kind)?;
        match kind[0] {
            1 => {
                let done = read_u64(&mut reader)?;
                let total = read_u64(&mut reader)?;
                if done > total || total == 0 {
                    return Err(error());
                }
                progress(done, total);
            }
            2 => return read_index(&mut reader),
            3 => {
                let mut code = [0; 4];
                reader.read_exact(&mut code)?;
                let mut size = [0; 2];
                reader.read_exact(&mut size)?;
                let size = u16::from_le_bytes(size) as usize;
                if size > 4096 {
                    return Err(error());
                }
                let mut message = vec![0; size];
                reader.read_exact(&mut message)?;
                return Err(io::Error::other(format!(
                    "NTFS reader: {} ({})",
                    String::from_utf8_lossy(&message),
                    i32::from_le_bytes(code)
                )));
            }
            _ => return Err(error()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_overlapped_local_pipe_delivers_bounded_index() {
        use windows_sys::Win32::System::Threading::OpenProcess;
        let token = format!(
            "test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let name = wide(format!("{PREFIX}{token}"));
        let pipe = Owned(unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                1 | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                65536,
                1000,
                std::ptr::null(),
            )
        });
        assert_ne!(pipe.0, INVALID_HANDLE_VALUE);
        let producer = std::thread::spawn(move || {
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    0x4000_0000,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };
            assert_ne!(handle, INVALID_HANDLE_VALUE);
            let mut output = unsafe { std::fs::File::from_raw_handle(handle as *mut _) };
            let index = Index {
                skipped: 0,
                records: vec![Record {
                    reference: 9,
                    parent: 5,
                    name: "native.tmp".into(),
                    bytes: 42,
                    attributes: 0,
                    directory: false,
                }],
            };
            output.write_all(MAGIC).unwrap();
            write_index(&mut output, &index).unwrap();
        });
        let event = Owned(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) });
        let process = Owned(unsafe { OpenProcess(0x0010_1000, 0, std::process::id()) });
        assert_ne!(event.0, 0);
        assert_ne!(process.0, 0);
        let mut control = || true;
        let mut reader = PipeReader {
            pipe,
            event,
            process,
            control: &mut control,
        };
        let mut operation: OVERLAPPED = unsafe { zeroed() };
        operation.hEvent = reader.event.0;
        if unsafe { ConnectNamedPipe(reader.pipe.0, &mut operation) } == 0 {
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => {}
                ERROR_IO_PENDING => reader.wait(&mut operation).unwrap(),
                code => panic!("connect: {code}"),
            }
        }
        let mut client = 0;
        assert_ne!(
            unsafe { GetNamedPipeClientProcessId(reader.pipe.0, &mut client) },
            0
        );
        assert_eq!(client, std::process::id());
        let mut reader = BufReader::new(reader);
        let mut magic = [0; 9];
        reader.read_exact(&mut magic).unwrap();
        assert_eq!(&magic, MAGIC);
        let mut kind = [0];
        reader.read_exact(&mut kind).unwrap();
        assert_eq!(kind[0], 2);
        let index = read_index(&mut reader).unwrap();
        assert_eq!(index.records[0].bytes, 42);
        producer.join().unwrap();
    }
    #[test]
    fn pipe_codec_round_trip_and_oversized_response_rejected() {
        let index = Index {
            skipped: 2,
            records: vec![Record {
                reference: 7,
                parent: 5,
                name: "fixture.tmp".into(),
                bytes: 20,
                attributes: 0,
                directory: false,
            }],
        };
        let mut buffer = Vec::new();
        write_index(&mut buffer, &index).unwrap();
        assert_eq!(buffer[0], 2);
        let decoded = read_index(&mut &buffer[1..]).unwrap();
        assert_eq!(decoded.records[0].name, "fixture.tmp");
        assert_eq!(decoded.records[0].bytes, 20);
        assert_eq!(decoded.skipped, 2);
        let mut invalid = vec![0; 8];
        invalid.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_index(&mut &invalid[..]).is_err());
    }
}
