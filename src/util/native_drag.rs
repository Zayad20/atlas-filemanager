use std::path::PathBuf;

// A tiny valid PNG used as the native drag preview. Atlas can replace this
// with richer previews later without changing the platform integration.
#[cfg(any(target_os = "windows", target_os = "macos"))]
const DRAG_PREVIEW_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 248, 207, 192, 240, 31, 0, 5,
    0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn start(frame: &eframe::Frame, paths: Vec<PathBuf>) -> bool {
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|path| match path.canonicalize() {
            Ok(path) => Some(path),
            Err(err) => {
                tracing::warn!("cannot drag {}: {err}", path.display());
                None
            }
        })
        .collect();
    if paths.is_empty() {
        return false;
    }

    let result = drag::start_drag(
        frame,
        drag::DragItem::Files(paths),
        drag::Image::Raw(DRAG_PREVIEW_PNG.to_vec()),
        |_result, _cursor_position| {},
        drag::Options::default(),
    );
    if let Err(err) = result {
        tracing::warn!("failed to start native file drag: {err}");
        return false;
    }
    true
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn start(_frame: &eframe::Frame, _paths: Vec<PathBuf>) -> bool {
    false
}
