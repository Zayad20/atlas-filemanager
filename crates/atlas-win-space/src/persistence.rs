//! Cross-process ownership of Atlas configuration writes.
use std::{io, marker::PhantomData, rc::Rc};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
};

/// A Win32 mutex must be released by the thread that acquired it.
pub struct ConfigWriteGuard {
    handle: HANDLE,
    _same_thread: PhantomData<Rc<()>>,
}

impl ConfigWriteGuard {
    pub fn acquire() -> io::Result<Self> {
        let name = super::wide(std::ffi::OsStr::new(r"Local\AtlasConfigMutex"));
        // SAFETY: name is NUL terminated and no security descriptor is supplied.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle == 0 {
            return Err(io::Error::last_os_error());
        }
        // Abandoned ownership is recoverable: writes replace complete JSON files.
        let wait = unsafe { WaitForSingleObject(handle, 30_000) };
        if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
            let error = if wait == 258 {
                io::Error::new(io::ErrorKind::TimedOut, "Atlas config mutex timeout")
            } else {
                io::Error::last_os_error()
            };
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self {
            handle,
            _same_thread: PhantomData,
        })
    }
}

impl Drop for ConfigWriteGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}
