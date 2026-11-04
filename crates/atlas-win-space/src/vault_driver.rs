//! Load only the administrator-installed WinFsp DLL by absolute registry path.
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};
use windows_sys::Win32::System::{
    LibraryLoader::{
        LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    },
    Registry::*,
};

pub fn load() -> std::io::Result<()> {
    for key in ["SOFTWARE\\WOW6432Node\\WinFsp", "SOFTWARE\\WinFsp"] {
        let key: Vec<_> = key.encode_utf16().chain(Some(0)).collect();
        let value: Vec<_> = "InstallDir".encode_utf16().chain(Some(0)).collect();
        let mut buffer = [0u16; 32768];
        let mut bytes = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            continue;
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let name = if cfg!(target_arch = "aarch64") {
            "winfsp-a64.dll"
        } else if cfg!(target_arch = "x86") {
            "winfsp-x86.dll"
        } else {
            "winfsp-x64.dll"
        };
        let path = PathBuf::from(OsString::from_wide(&buffer[..len]))
            .join("bin")
            .join(name);
        if !path.is_absolute() {
            continue;
        }
        let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // Kept loaded for the process lifetime: all driver function pointers stay valid.
        if unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                0,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        } != 0
        {
            return Ok(());
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "vault_driver_missing",
    ))
}
