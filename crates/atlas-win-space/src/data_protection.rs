//! User-scoped Windows protection for the private folder's locked display name.
//! This never protects file data or password keys; those remain password encrypted.
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};

struct Output(CRYPT_INTEGER_BLOB);

impl Drop for Output {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // DPAPI owns this allocation. Erase plaintext before returning it to Windows.
            unsafe {
                for offset in 0..self.0.cbData as usize {
                    ptr::write_volatile(self.0.pbData.add(offset), 0);
                }
                LocalFree(self.0.pbData.cast());
            }
        }
    }
}

pub fn protect(bytes: &[u8], context: &[u8]) -> io::Result<Vec<u8>> {
    transform(bytes, context, false)
}

pub fn unprotect(bytes: &[u8], context: &[u8]) -> io::Result<Vec<u8>> {
    transform(bytes, context, true)
}

fn transform(bytes: &[u8], context: &[u8], decrypt: bool) -> io::Result<Vec<u8>> {
    // This API is for short names, not file payloads. Bound allocations on both sides.
    if bytes.is_empty() || bytes.len() > 16 * 1024 || context.len() > 16 * 1024 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: context.len() as u32,
        pbData: context.as_ptr().cast_mut(),
    };
    let mut output = Output(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    });
    // Inputs remain alive and are read-only. No description or interactive prompt
    // is requested. Omitting LOCAL_MACHINE binds the result to the Windows user.
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        } else {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if output.0.pbData.is_null() || output.0.cbData == 0 || output.0.cbData > 16 * 1024 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    // Copy into Rust-owned memory before the Windows allocation is erased/freed.
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}
