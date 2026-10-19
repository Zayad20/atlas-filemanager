//! Last-chance native crash reports. Paths are prepared before a fault occurs.
mod dbghelp;
use std::{
    io,
    path::PathBuf,
    ptr::null,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ},
    System::{
        Diagnostics::Debug::{
            SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
        },
        Threading::{GetCurrentProcessId, GetCurrentThreadId},
    },
};

static DUMP_PATH: OnceLock<Vec<u16>> = OnceLock::new();
static STACK_PATH: OnceLock<Vec<u16>> = OnceLock::new();
static WRITING: AtomicBool = AtomicBool::new(false);

pub fn install() -> io::Result<()> {
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA"))?
        .join("Atlas")
        .join("Crashes");
    install_at(directory)
}

fn install_at(directory: PathBuf) -> io::Result<()> {
    if DUMP_PATH.get().is_some() {
        return Ok(());
    }
    dbghelp::load()?;
    std::fs::create_dir_all(&directory)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("atlas-{}-{timestamp}.dmp", std::process::id()));
    let _ = STACK_PATH.set(super::wide(path.with_extension("stack.txt").as_os_str()));
    let _ = DUMP_PATH.set(super::wide(path.as_os_str()));
    unsafe {
        SetUnhandledExceptionFilter(Some(unhandled_exception));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        unsafe {
            write_report(null());
        }
        previous(info);
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_fault_child() {
        let Some(directory) = std::env::var_os("ATLAS_CRASH_TEST_DIR") else {
            return;
        };
        install_at(directory.into()).unwrap();
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(
                windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX,
            );
            windows_sys::Win32::System::Diagnostics::Debug::RaiseException(
                0xc0000005,
                1,
                0,
                std::ptr::null(),
            );
        }
    }
    #[test]
    fn child_native_fault_writes_minidump_and_stack_without_opening_ui() {
        let directory =
            std::env::temp_dir().join(format!("atlas-crash-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash::tests::native_fault_child", "--nocapture"])
            .env("ATLAS_CRASH_TEST_DIR", &directory)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let files: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let dump = files
            .iter()
            .find(|path| path.extension().is_some_and(|extension| extension == "dmp"))
            .expect("minidump file");
        assert_eq!(&std::fs::read(dump).unwrap()[..4], b"MDMP");
        let stack = files
            .iter()
            .find(|path| path.to_string_lossy().ends_with("stack.txt"))
            .expect("stack file");
        assert!(std::fs::read_to_string(stack).unwrap().starts_with("0x"));
    }
}

unsafe extern "system" fn unhandled_exception(exception: *const EXCEPTION_POINTERS) -> i32 {
    unsafe {
        write_report(exception);
    }
    1 // Never continue executing at the fault.
}

unsafe fn write_report(exception: *const EXCEPTION_POINTERS) {
    if WRITING.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(path) = DUMP_PATH.get() {
        let file = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ,
                null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if file != INVALID_HANDLE_VALUE {
            let info = MINIDUMP_EXCEPTION_INFORMATION {
                ThreadId: unsafe { GetCurrentThreadId() },
                ExceptionPointers: exception.cast_mut(),
                ClientPointers: 0,
            };
            unsafe {
                dbghelp::dump(
                    file,
                    GetCurrentProcessId(),
                    if exception.is_null() { null() } else { &info },
                );
                CloseHandle(file);
            }
        }
    }
    if let Some(path) = STACK_PATH.get() {
        let file = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ,
                null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if file != INVALID_HANDLE_VALUE {
            unsafe {
                dbghelp::stack(file, exception);
                CloseHandle(file);
            }
        }
    }
}
