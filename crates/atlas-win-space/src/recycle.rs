//! Recycle Bin namespace columns; no recycled file data streams are opened.
use std::collections::HashMap;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{w, Interface, PWSTR};
use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::{
    VariantClear, VariantToFileTime, VariantToStringAlloc, PSTF_UTC, VARIANT,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::PropertiesSystem::PSGetPropertyKeyFromName;
use windows::Win32::UI::Shell::*;

pub struct RecycleMetadata {
    pub deleted_from: PathBuf,
    pub deleted_at: SystemTime,
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}
struct Variant(VARIANT);
impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

unsafe fn owned_path(value: PWSTR) -> PathBuf {
    let result = PathBuf::from(unsafe { std::ffi::OsString::from_wide(value.as_wide()) });
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    result
}

/// Return Shell's real deletion time and source folder, keyed by the physical
/// recycled path. Callers keep their direct `$I` reader as a fallback for Shell
/// extensions, access failures, and items removed during enumeration.
pub fn metadata() -> windows::core::Result<HashMap<PathBuf, RecycleMetadata>> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let root = Pidl(SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None)?);
        let desktop = SHGetDesktopFolder()?;
        let folder: IShellFolder2 = desktop.BindToObject(root.0, None)?;
        let parent: IShellFolder = folder.cast()?;
        let mut date_key = PROPERTYKEY::default();
        let mut from_key = PROPERTYKEY::default();
        PSGetPropertyKeyFromName(w!("System.Recycle.DateDeleted"), &mut date_key)?;
        PSGetPropertyKeyFromName(w!("System.Recycle.DeletedFrom"), &mut from_key)?;
        let mut enumeration = None;
        folder
            .EnumObjects(
                HWND::default(),
                (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32,
                &mut enumeration,
            )
            .ok()?;
        let mut result = HashMap::new();
        let Some(enumeration) = enumeration else {
            return Ok(result);
        };
        loop {
            let mut ids = [std::ptr::null_mut()];
            let mut fetched = 0;
            enumeration.Next(&mut ids, Some(&mut fetched)).ok()?;
            if fetched == 0 {
                break;
            }
            let child = Pidl(ids[0]);
            let read = || -> windows::core::Result<(PathBuf, RecycleMetadata)> {
                let item: IShellItem = SHCreateItemWithParent(None, &parent, child.0)?;
                let physical = owned_path(item.GetDisplayName(SIGDN_FILESYSPATH)?);
                let from = Variant(folder.GetDetailsEx(child.0, &from_key)?);
                let original_dir = owned_path(VariantToStringAlloc(&from.0)?);
                let date = Variant(folder.GetDetailsEx(child.0, &date_key)?);
                let ft = VariantToFileTime(&date.0, PSTF_UTC)?;
                let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
                let epoch = 116_444_736_000_000_000u64;
                let ticks_delta = ticks.abs_diff(epoch);
                let delta = Duration::new(
                    ticks_delta / 10_000_000,
                    ((ticks_delta % 10_000_000) * 100) as u32,
                );
                let deleted_at = if ticks >= epoch {
                    UNIX_EPOCH + delta
                } else {
                    UNIX_EPOCH - delta
                };
                Ok((
                    physical,
                    RecycleMetadata {
                        deleted_from: original_dir,
                        deleted_at,
                    },
                ))
            };
            if let Ok((path, metadata)) = read() {
                result.insert(path, metadata);
            }
        }
        Ok(result)
    }
}
