use crate::util::display::is_hidden;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn build_index(current_root: PathBuf, injector: nucleo::Injector<PathBuf>) {
    rayon::spawn(move || {
        let home = dirs::home_dir().unwrap_or_else(|| current_root.clone());
        let mut seen = HashSet::new();

        for path in common_folders()
            .into_iter()
            .chain([home.clone(), current_root.clone()])
        {
            inject_folder(path, &mut seen, &injector);
        }

        index_tree(&home, 5, &mut seen, &injector);
        if !current_root.starts_with(&home) {
            index_tree(&current_root, 3, &mut seen, &injector);
        }
    });
}

pub fn prioritize_folders(folders: &mut [PathBuf], current: Option<&Path>) {
    let home = dirs::home_dir();
    let common = common_folders();
    folders.sort_by_key(|path| {
        let priority = if current == Some(path.as_path()) {
            0
        } else if common.iter().any(|folder| folder == path)
            || home.as_ref().is_some_and(|folder| folder == path)
        {
            1
        } else if current.is_some_and(|folder| path.starts_with(folder)) {
            2
        } else if common.iter().any(|folder| path.starts_with(folder)) {
            3
        } else if home.as_ref().is_some_and(|folder| path.starts_with(folder)) {
            4
        } else {
            5
        };
        (priority, path.components().count(), path.clone())
    });
}

fn index_tree(
    root: &Path,
    max_depth: usize,
    seen: &mut HashSet<PathBuf>,
    injector: &nucleo::Injector<PathBuf>,
) {
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.path()))
        .flatten()
        .filter(|entry| entry.file_type().is_dir())
    {
        inject_folder(entry.into_path(), seen, injector);
    }
}

fn inject_folder(path: PathBuf, seen: &mut HashSet<PathBuf>, injector: &nucleo::Injector<PathBuf>) {
    if !path.is_dir() || !seen.insert(path.clone()) {
        return;
    }
    let display = path.to_string_lossy().into_owned();
    injector.push(path, move |_item, cols| {
        cols[0] = display.as_str().into();
    });
}

fn common_folders() -> Vec<PathBuf> {
    [
        dirs::desktop_dir(),
        dirs::download_dir(),
        dirs::document_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        dirs::audio_dir(),
    ]
    .into_iter()
    .flatten()
    .collect()
}



#[cfg(test)]
mod tests {
    use super::prioritize_folders;
    use std::path::PathBuf;

    #[test]
    fn prioritizes_current_folder_and_its_nearby_children() {
        let current = PathBuf::from("/tmp/Atlas-project");
        let child = current.join("src");
        let nested = child.join("components");
        let mut folders = vec![nested.clone(), child.clone(), current.clone()];

        prioritize_folders(&mut folders, Some(&current));

        assert_eq!(folders, vec![current, child, nested]);
    }
}
