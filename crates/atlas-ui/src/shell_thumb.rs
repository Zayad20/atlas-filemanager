#![allow(unsafe_code)]
//! Fast thumbnail extraction via the Windows Shell thumbnail cache.
//!
//! Replaces the per-file ffmpeg subprocess approach with
//! `IShellItemImageFactory::GetImage`, which reads from (and writes into) the
//! same `thumbcache_*.db` that Explorer uses.  Second visits are instant; no
//! temp files; no spawned processes; Windows manages eviction.
//!
//! **Call only from `tokio::task::spawn_blocking`** — `GetImage` may block
//! while generating the thumbnail for the first visit.

use std::mem::size_of;
use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName};

// SIIGBF flag constants (from Windows SDK ShObjIdl_core.h).
// declare locally — named constants appeared later in windows-rs; avoids tracking
//           which exact crate release exports them.
const SIIGBF_THUMBNAILONLY: i32 = 0x0000_0008;

thread_local! {
    /// COM is initialized once per blocking-pool thread.  Each tokio blocking
    /// thread is an independent STA, so all concurrent `GetImage` calls proceed
    /// in parallel without serialization.
    static COM_INITED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn ensure_com_init() {
    COM_INITED.with(|flag| {
        if !flag.get() {
            // S_FALSE (already initialized on this thread) is fine — we ignore it.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            flag.set(true);
        }
    });
}

/// Extract a thumbnail for `path` from the Windows Shell thumbnail cache.
///
/// Returns `(RGBA pixels, width, height)` where `width * height * 4 == pixels.len()`.
/// The longest edge is at most `max_edge` pixels (Windows scales down if needed).
///
/// Returns `None` when:
/// - The path is invalid / not found.
/// - No thumbnail provider is registered for this file type.
/// - `IShellItemImageFactory::GetImage` fails for any other reason.
///
/// The caller falls back to the file-type glyph on `None`, matching the
/// existing behavior for unknown formats.
pub fn shell_thumbnail(path: &Path, max_edge: u32) -> Option<(Vec<u8>, usize, usize)> {
    ensure_com_init();

    // Encode path as null-terminated UTF-16 for the Shell API.
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // Create an IShellItem for the path, then QI for IShellItemImageFactory.
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;

        // SIIGBF_THUMBNAILONLY: fail (→ None) rather than returning a generic icon.
        // Windows scales the result to fit within `max_edge × max_edge`.
        let size = SIZE {
            cx: max_edge as i32,
            cy: max_edge as i32,
        };
        let flags = windows::Win32::UI::Shell::SIIGBF(SIIGBF_THUMBNAILONLY);
        let hbitmap = factory.GetImage(size, flags).ok()?;

        // --- HBITMAP → RGBA pixels ---

        // Obtain the actual pixel dimensions (may differ from the requested size).
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(hbitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        if got == 0 {
            let _ = DeleteObject(HGDIOBJ(hbitmap.0));
            return None;
        }
        let w = bm.bmWidth as usize;
        let h = bm.bmHeight.unsigned_abs() as usize;
        if w == 0 || h == 0 {
            let _ = DeleteObject(HGDIOBJ(hbitmap.0));
            return None;
        }

        // GetDIBits needs any valid HDC (used only for palette operations with
        // DIB_PAL_COLORS; harmless for our 32bpp BI_RGB request).
        let hdc = CreateCompatibleDC(None);

        // Request 32bpp top-down DIB (negative biHeight = top-down scanlines).
        let mut bi_header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..Default::default()
        };
        let mut pixels = vec![0u8; w * h * 4];
        // BITMAPINFO = BITMAPINFOHEADER + RGBQUAD[1]; no RGBQUAD needed for 32bpp BI_RGB.
        let bi_ptr = &mut bi_header as *mut BITMAPINFOHEADER as *mut BITMAPINFO;
        let rows = GetDIBits(
            hdc,
            hbitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            bi_ptr,
            DIB_RGB_COLORS,
        );

        let _ = DeleteDC(hdc);
        let _ = DeleteObject(HGDIOBJ(hbitmap.0));

        if rows == 0 {
            return None;
        }

        // Windows DIBs with BI_RGB store pixels as BGRA (4th byte undefined/0).
        // Swap B↔R → RGBA and force alpha=255 (thumbnails are always opaque; the
        // 4th byte from BI_RGB bitmaps is unreliable).
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.swap(0, 2); // B↔R
            chunk[3] = 255;
        }

        Some((pixels, w, h))
    }
}
