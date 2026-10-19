//! Windows clipboard format IDs used by Atlas drag-and-drop.
//!
//! Well-known CFSTR_* string formats are registered once via
//! `RegisterClipboardFormatW` and cached behind `OnceLock`.
#![allow(unsafe_code)]
// reason: RegisterClipboardFormatW is an unsafe Win32 call wrapped behind OnceLock.

use std::sync::OnceLock;
use windows::core::w;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;

pub fn cf_open_window() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("Atlas-OpenWindowFormat")) as u16 })
}

pub fn cf_drop_description() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("DropDescription")) as u16 })
}

/// "Preferred DropEffect" — set on our data object to announce our preference.
pub fn cf_preferred_drop_effect() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16 })
}

/// "Performed DropEffect" — the target writes this after a drop; we read it to
/// decide whether to delete source files on a move.
pub fn cf_performed_drop_effect() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("Performed DropEffect")) as u16 })
}

/// Private Atlas format. Signals an intra-Atlas drag so the drop target can
/// choose MOVE instead of COPY by default.
pub fn cf_atlas_source() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("AtlasInternalSource")) as u16 })
}

/// "FileGroupDescriptorW" — array of `FILEDESCRIPTORW` for virtual file drag-out
/// (e.g. extracting archive contents into Explorer).
pub fn cf_file_descriptor_w() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("FileGroupDescriptorW")) as u16 })
}

/// "FileContents" — stream content for virtual files, indexed by `lindex`.
pub fn cf_file_contents() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("FileContents")) as u16 })
}
