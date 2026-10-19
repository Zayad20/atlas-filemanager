//! FreeDesktop.org Thumbnail Managing Standard support for Linux desktops.
//! Looks up system-cached thumbnails in `$XDG_CACHE_HOME/thumbnails/{large,x-large,normal}`.

use std::path::Path;

#[cfg(target_os = "linux")]
pub fn freedesktop_thumbnail(path: &Path, max_edge: u32) -> Option<(Vec<u8>, usize, usize)> {
    use md5::{Digest, Md5};
    use std::path::PathBuf;

    let canonical = std::fs::canonicalize(path)
        .ok()
        .unwrap_or_else(|| path.to_path_buf());
    let url = url::Url::from_file_path(&canonical).ok()?;
    let mut hasher = Md5::new();
    hasher.update(url.as_str().as_bytes());
    let hash = format!("{:x}.png", hasher.finalize());

    let cache_home = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".cache"))
                .unwrap_or_default()
        });
    let thumbs_dir = cache_home.join("thumbnails");

    // Check size tiers from preferred to smaller
    for subdir in &["large", "x-large", "normal"] {
        let candidate = thumbs_dir.join(subdir).join(&hash);
        if candidate.is_file() {
            if let Ok(img) = image::open(&candidate) {
                let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
                let (w, h) = (thumb.width() as usize, thumb.height() as usize);
                return Some((thumb.into_raw(), w, h));
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn freedesktop_thumbnail(_path: &Path, _max_edge: u32) -> Option<(Vec<u8>, usize, usize)> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn test_freedesktop_hash_matches_spec() {
        use md5::{Digest, Md5};
        let uri = "file:///home/user/test.png";
        let mut hasher = Md5::new();
        hasher.update(uri.as_bytes());
        let hash = format!("{:x}.png", hasher.finalize());
        // MD5 of file:///home/user/test.png is 03223f4f10458a8b5d14327f3ae23136
        assert_eq!(hash, "03223f4f10458a8b5d14327f3ae23136.png");
    }
}
