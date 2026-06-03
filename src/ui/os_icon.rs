//! Native OS icon loader — Windows Shell (SHGetFileInfo) and macOS NSWorkspace.
//!
//! Icons are keyed by (extension, is_dir, size_bucket) and cached as egui
//! textures.  URI scheme: `os-icon://<ext>?dir=<0|1>&size=<16|32>`
//!
//! On Windows extraction runs in a background thread so the render thread is
//! never stalled.  On macOS it runs synchronously in `load()` (which is the
//! main thread in eframe) because NSWorkspace is main-thread-only; the per-
//! extension cost is tiny (~1 ms) and paid only once per session.
//!
//! On Linux (and any other platform) the loader returns `NotSupported`
//! immediately so the phosphor fallback in `show_file_icon` always kicks in.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use egui::load::{LoadError, SizeHint, SizedTexture, TextureLoadResult, TextureLoader, TexturePoll};

const URI_PREFIX: &str = "os-icon://";

// ─── Public API ──────────────────────────────────────────────────────────────

pub fn install(ctx: &egui::Context) {
    if !ctx.is_loader_installed(OsIconLoader::ID) {
        ctx.add_texture_loader(Arc::new(OsIconLoader::new()));
    }
}

/// Stable URI for an OS icon.  `size` is the *requested* pixel size; it is
/// bucketed to 16 or 32 internally.
pub fn uri(ext: Option<&str>, is_dir: bool, size: f32) -> String {
    let bucket: u32 = if size <= 20.0 { 16 } else { 32 };
    let dir: u8 = u8::from(is_dir);
    format!("{URI_PREFIX}{}?dir={dir}&size={bucket}", ext.unwrap_or(""))
}

/// Render the native OS icon for a file entry.
///
/// Falls back to the phosphor glyph while the background extraction is in
/// progress, or on platforms without native icon support (Linux).
pub fn show_file_icon(ui: &mut egui::Ui, ext: Option<&str>, is_dir: bool, pixel_size: f32) {
    let icon_uri = uri(ext, is_dir, pixel_size);
    if let Ok(TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
        &icon_uri,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
        SizeHint::default(),
    ) {
        ui.add(
            egui::Image::from_texture(texture)
                .fit_to_exact_size(egui::vec2(pixel_size, pixel_size)),
        );
        return;
    }

    // Phosphor fallback (also used while the first frame is loading)
    let icon = crate::util::extensions::ExtensionRegistry::icon(ext);
    let color = if is_dir {
        crate::util::display::FOLDER_ICON_COLOR
    } else {
        crate::util::extensions::ExtensionRegistry::color(ext)
    };
    ui.colored_label(color, icon);
}

/// Like [`show_file_icon`] but renders at a large size inside a constrained
/// region (used by the icon grid for non-previewable files).
pub fn show_file_icon_large(
    ui: &mut egui::Ui,
    ext: Option<&str>,
    is_dir: bool,
    thumbnail_size: f32,
    display_size: egui::Vec2,
) {
    let icon_uri = uri(ext, is_dir, thumbnail_size);
    if let Ok(TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
        &icon_uri,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
        SizeHint::default(),
    ) {
        ui.add(
            egui::Image::from_texture(texture)
                .fit_to_exact_size(display_size)
                .corner_radius(4.0),
        );
        return;
    }

    // Phosphor fallback
    let icon = crate::util::extensions::ExtensionRegistry::icon(ext);
    let color = if is_dir {
        crate::util::display::FOLDER_ICON_COLOR
    } else {
        crate::util::extensions::ExtensionRegistry::color(ext)
    };
    ui.add(
        egui::Label::new(
            egui::RichText::new(icon)
                .size((thumbnail_size * 0.32).max(24.0))
                .color(color),
        )
        .selectable(false),
    );
}

// ─── Loader ──────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq, Hash)]
struct IconKey {
    ext: String,
    is_dir: bool,
    size: u32,
}

impl IconKey {
    fn parse(uri: &str) -> Option<Self> {
        let tail = uri.strip_prefix(URI_PREFIX)?;
        let (ext_part, query) = tail.split_once('?').unwrap_or((tail, ""));
        let mut is_dir = false;
        let mut size = 16u32;
        for seg in query.split('&') {
            if let Some(v) = seg.strip_prefix("dir=") {
                is_dir = v == "1";
            } else if let Some(v) = seg.strip_prefix("size=") {
                size = v.parse().unwrap_or(16);
            }
        }
        Some(IconKey { ext: ext_part.to_owned(), is_dir, size })
    }
}

enum IconEntry {
    Pending,
    Ready(egui::TextureHandle),
    Failed(String),
}

pub struct OsIconLoader {
    cache: Arc<Mutex<HashMap<IconKey, IconEntry>>>,
    #[cfg(target_os = "windows")]
    worker: windows_backend::Worker,
}

impl OsIconLoader {
    const ID: &'static str = egui::generate_loader_id!(OsIconLoader);

    fn new() -> Self {
        let cache: Arc<Mutex<HashMap<IconKey, IconEntry>>> = Arc::default();
        Self {
            #[cfg(target_os = "windows")]
            worker: windows_backend::Worker::spawn(cache.clone()),
            cache,
        }
    }
}

impl TextureLoader for OsIconLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(
        &self,
        ctx: &egui::Context,
        uri: &str,
        _tex_opts: egui::TextureOptions,
        _hint: SizeHint,
    ) -> TextureLoadResult {
        let Some(key) = IconKey::parse(uri) else {
            return Err(LoadError::NotSupported);
        };

        {
            let cache = lock(&self.cache);
            match cache.get(&key) {
                Some(IconEntry::Ready(tex)) => {
                    let sz = egui::vec2(key.size as f32, key.size as f32);
                    return Ok(TexturePoll::Ready {
                        texture: SizedTexture::new(tex.id(), sz),
                    });
                }
                Some(IconEntry::Failed(msg)) => return Err(LoadError::Loading(msg.clone())),
                Some(IconEntry::Pending) => return Ok(TexturePoll::Pending { size: None }),
                None => {}
            }
        }

        // First request for this key
        lock(&self.cache).insert(key.clone(), IconEntry::Pending);

        #[cfg(target_os = "windows")]
        self.worker.request(key, ctx.clone());

        // macOS: extract synchronously (load() is always the main thread in eframe)
        #[cfg(target_os = "macos")]
        {
            let ext = if key.ext.is_empty() { None } else { Some(key.ext.as_str()) };
            match macos_backend::extract(ext, key.is_dir, key.size) {
                Ok(image) => {
                    let handle = upload(ctx, &key, image);
                    let sz = egui::vec2(key.size as f32, key.size as f32);
                    let sized = SizedTexture::new(handle.id(), sz);
                    lock(&self.cache).insert(key, IconEntry::Ready(handle));
                    return Ok(TexturePoll::Ready { texture: sized });
                }
                Err(e) => {
                    lock(&self.cache).insert(key, IconEntry::Failed(e.clone()));
                    return Err(LoadError::Loading(e));
                }
            }
        }

        // Linux / other: immediately mark failed so phosphor fallback is permanent
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        lock(&self.cache).insert(key, IconEntry::Failed("platform unsupported".into()));

        Ok(TexturePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        if let Some(key) = IconKey::parse(uri) {
            lock(&self.cache).remove(&key);
        }
    }

    fn forget_all(&self) {
        lock(&self.cache).clear();
    }

    fn byte_size(&self) -> usize {
        0
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn upload(ctx: &egui::Context, key: &IconKey, image: egui::ColorImage) -> egui::TextureHandle {
    ctx.load_texture(
        format!("os-icon:{}-{}-{}", key.ext, key.is_dir as u8, key.size),
        image,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
    )
}

// ─── Windows backend ─────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_backend {
    use super::{lock, upload, IconEntry, IconKey};
    use std::collections::HashMap;
    use std::sync::{Arc, Condvar, Mutex};

    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON,
        SHGFI_USEFILEATTRIBUTES,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    struct Request {
        key: IconKey,
        ctx: egui::Context,
    }

    pub struct Worker {
        queue: Arc<(Mutex<Vec<Request>>, Condvar)>,
    }

    impl Worker {
        pub fn spawn(cache: Arc<Mutex<HashMap<IconKey, IconEntry>>>) -> Self {
            let queue: Arc<(Mutex<Vec<Request>>, Condvar)> = Arc::default();
            let q2 = queue.clone();
            std::thread::Builder::new()
                .name("Atlas-os-icon".into())
                .spawn(move || run(&cache, &q2))
                .expect("spawn os-icon worker");
            Self { queue }
        }

        pub fn request(&self, key: IconKey, ctx: egui::Context) {
            let (lock, cvar) = &*self.queue;
            lock.lock().unwrap_or_else(|p| p.into_inner()).push(Request { key, ctx });
            cvar.notify_one();
        }
    }

    fn run(cache: &Arc<Mutex<HashMap<IconKey, IconEntry>>>, queue: &Arc<(Mutex<Vec<Request>>, Condvar)>) {
        // COM must be initialized before any Shell calls on this thread.
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }

        loop {
            let req = {
                let (mtx, cvar) = &**queue;
                let mut q = mtx.lock().unwrap_or_else(|p| p.into_inner());
                loop {
                    if let Some(r) = q.pop() {
                        break r;
                    }
                    q = cvar.wait(q).unwrap_or_else(|p| p.into_inner());
                }
            };

            let ext = if req.key.ext.is_empty() { None } else { Some(req.key.ext.as_str()) };
            let result = extract(ext, req.key.is_dir, req.key.size);

            let mut c = lock(cache);
            match result {
                Ok(image) => {
                    let handle = upload(&req.ctx, &req.key, image);
                    c.insert(req.key, IconEntry::Ready(handle));
                }
                Err(e) => {
                    c.insert(req.key, IconEntry::Failed(e));
                }
            }
            drop(c);
            req.ctx.request_repaint();
        }
    }

    fn extract(ext: Option<&str>, is_dir: bool, size: u32) -> Result<egui::ColorImage, String> {
        let path: Vec<u16> = match ext {
            Some(e) => format!("dummy.{e}"),
            None => "dummy".to_string(),
        }
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

        let size_flag = if size <= 16 { SHGFI_SMALLICON } else { SHGFI_LARGEICON };
        let flags = SHGFI_ICON | SHGFI_USEFILEATTRIBUTES | size_flag;
        let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };

        let mut info = SHFILEINFOW::default();
        let ok = unsafe {
            SHGetFileInfoW(
                windows::core::PCWSTR(path.as_ptr()),
                attrs,
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };
        if ok == 0 {
            return Err(format!("SHGetFileInfoW failed (ext={ext:?})"));
        }

        let hicon = info.hIcon;
        let result = hicon_to_rgba(hicon, size as i32);
        unsafe { let _ = DestroyIcon(hicon); }
        result
    }

    fn hicon_to_rgba(
        hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
        size: i32,
    ) -> Result<egui::ColorImage, String> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // negative = top-down (row 0 = top)
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();

        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            return Err("CreateCompatibleDC failed".into());
        }

        let hbm = unsafe { CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(|e| format!("CreateDIBSection: {e}"))?;

        let old = unsafe { SelectObject(hdc, hbm) };

        let drew = unsafe { DrawIconEx(hdc, 0, 0, hicon, size, size, 0, None, DI_NORMAL) };

        let result = if drew.is_ok() && !bits.is_null() {
            let len = (size * size * 4) as usize;
            let slice = unsafe { std::slice::from_raw_parts(bits as *const u8, len) };
            let mut px = slice.to_vec();
            // Windows DIBs are BGRA.  Swap B↔R → RGBA.
            // Icons drawn over a transparent background have alpha=0 on coloured
            // pixels (pre-multiplied-by-zero), so restore full opacity there.
            for ch in px.chunks_exact_mut(4) {
                ch.swap(0, 2);
                if ch[3] == 0 && (ch[0] | ch[1] | ch[2]) != 0 {
                    ch[3] = 255;
                }
            }
            Ok(egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &px))
        } else {
            Err("DrawIconEx failed".into())
        };

        unsafe {
            SelectObject(hdc, old);
            let _ = DeleteDC(hdc);
            let _ = DeleteObject(hbm);
        }

        result
    }
}

// ─── macOS backend ───────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos_backend {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSSize, NSString};

    pub fn extract(ext: Option<&str>, is_dir: bool, size: u32) -> Result<egui::ColorImage, String> {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };

        let ns_image = if is_dir {
            // `iconForFile:` on the root always returns the generic folder icon.
            let path = NSString::from_str("/");
            unsafe { workspace.iconForFile(&path) }
        } else {
            let ext_str = NSString::from_str(ext.unwrap_or(""));
            // iconForFileType: is deprecated in macOS 12 but still functional.
            #[allow(deprecated)]
            unsafe { workspace.iconForFileType(&ext_str) }
        };

        // Request the size we want before rasterising.
        let ns_size = NSSize { width: size as f64, height: size as f64 };
        unsafe { ns_image.setSize(ns_size) };

        // Serialise to TIFF and decode with the `image` crate (TIFF feature is
        // already enabled in Cargo.toml).
        let tiff = unsafe { ns_image.TIFFRepresentation() }
            .ok_or_else(|| "NSImage.TIFFRepresentation returned nil".to_string())?;

        let len = unsafe { tiff.length() };
        let ptr = unsafe { tiff.bytes() } as *const u8;
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };

        let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff)
            .map_err(|e| format!("TIFF decode: {e}"))?;

        let scaled =
            decoded.resize(size, size, image::imageops::FilterType::Triangle);
        let rgba = scaled.into_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            rgba.as_raw(),
        ))
    }
}
