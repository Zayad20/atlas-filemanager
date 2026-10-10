//! ZIP archive provider.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use super::common::{build_synthetic_ancestors, ensure_parent_dir, path_to_dir_key};
use super::{make_entry, ArchiveProvider};
use atlas_model::file_entry::FileEntry;

/// Pre-indexed, read-only view of a `.zip` file.
///
/// The central directory is parsed once at construction; subsequent `list()`
/// calls are O(entries in directory) with no disk I/O.
pub struct ZipProvider {
    path: PathBuf,
    /// `inner_dir` (with trailing '/') → direct children entries
    index: HashMap<String, Vec<FileEntry>>,
}

impl ZipProvider {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut index: HashMap<String, Vec<FileEntry>> = HashMap::new();

        // Seed the root so `list("")` works even for empty archives.
        index.entry(String::new()).or_default();

        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            // enclosed_name() rejects zip-slip paths (../, absolute, etc.).
            let mut inner = match entry.enclosed_name() {
                Some(n) => n.to_path_buf(),
                None => continue,
            };
            let s = inner.to_string_lossy();
            if s.ends_with('/') || s.ends_with('\\') {
                let mut s = s.into_owned();
                s.pop();
                if s.is_empty() {
                    continue;
                }
                inner = PathBuf::from(s);
            }
            let raw_name = entry.name();
            // Skip __MACOSX cruft and other meta entries.
            if raw_name.starts_with("__MACOSX/") || raw_name.ends_with(".DS_Store") {
                continue;
            }
            let is_dir = entry.is_dir();
            let size = if is_dir { None } else { Some(entry.size()) };
            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            let parent_key = inner.parent().map(path_to_dir_key).unwrap_or_default();
            build_synthetic_ancestors(path, &inner, &mut index);

            let file_entry = make_entry(path, inner.clone(), name, is_dir, size);
            index.entry(parent_key).or_default().push(file_entry);
        }

        Ok(Self {
            path: path.to_path_buf(),
            index,
        })
    }
}

impl ArchiveProvider for ZipProvider {
    fn list(&self, inner_dir: &Path) -> anyhow::Result<Vec<FileEntry>> {
        let key = path_to_dir_key(inner_dir);
        Ok(self.index.get(&key).cloned().unwrap_or_default())
    }

    fn read_bytes(&self, inner_path: &Path) -> anyhow::Result<Box<dyn Read + Send>> {
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let file = std::fs::File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut entry = zip.by_name(&name)?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        Ok(Box::new(Cursor::new(buf)))
    }

    fn extract_to(&self, inner_path: &Path, dest: &Path) -> anyhow::Result<()> {
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let file = std::fs::File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut entry = zip.by_name(&name)?;
        ensure_parent_dir(dest)?;
        let mut out = std::fs::File::create(dest)?;
        std::io::copy(&mut entry, &mut out)?;
        Ok(())
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn make_test_zip(entries: &[(&str, Option<&str>)]) -> tempfile::NamedTempFile {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let file = std::fs::File::create(tmp.path()).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, content) in entries {
            if let Some(data) = content {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data.as_bytes()).unwrap();
            } else {
                zip.add_directory(*name, opts).unwrap();
            }
        }
        zip.finish().unwrap();
        tmp
    }

    #[test]
    fn subdir_implicit_listed() {
        // Zip created with only files (no explicit dir entries).
        let tmp = make_test_zip(&[
            ("top.txt", Some("hello")),
            ("sub/a.txt", Some("aaa")),
            ("sub/b.txt", Some("bbb")),
            ("sub/deep/c.txt", Some("ccc")),
        ]);
        let prov = ZipProvider::new(tmp.path()).unwrap();

        // Root should show top.txt + sub/
        let root = prov.list(Path::new("")).unwrap();
        let names: Vec<_> = root.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"top.txt"),
            "root missing top.txt: {names:?}"
        );
        assert!(names.contains(&"sub"), "root missing sub/: {names:?}");

        // sub/ should show a.txt, b.txt, deep/
        let sub = prov.list(Path::new("sub")).unwrap();
        let sub_names: Vec<_> = sub.iter().map(|e| e.name.as_str()).collect();
        assert!(
            sub_names.contains(&"a.txt"),
            "sub missing a.txt: {sub_names:?}"
        );
        assert!(
            sub_names.contains(&"b.txt"),
            "sub missing b.txt: {sub_names:?}"
        );
        assert!(
            sub_names.contains(&"deep"),
            "sub missing deep/: {sub_names:?}"
        );

        // sub/deep/ should show c.txt
        let deep = prov.list(Path::new("sub/deep")).unwrap();
        let deep_names: Vec<_> = deep.iter().map(|e| e.name.as_str()).collect();
        assert!(
            deep_names.contains(&"c.txt"),
            "deep missing c.txt: {deep_names:?}"
        );
    }

    #[test]
    fn subdir_explicit_listed() {
        // Zip with explicit directory entries.
        let tmp = make_test_zip(&[
            ("sub/", None),
            ("sub/deep/", None),
            ("sub/a.txt", Some("aaa")),
            ("sub/deep/c.txt", Some("ccc")),
        ]);
        let prov = ZipProvider::new(tmp.path()).unwrap();

        let root = prov.list(Path::new("")).unwrap();
        let root_names: Vec<_> = root.iter().map(|e| e.name.as_str()).collect();
        assert!(
            root_names.contains(&"sub"),
            "root missing sub: {root_names:?}"
        );

        let sub = prov.list(Path::new("sub")).unwrap();
        let sub_names: Vec<_> = sub.iter().map(|e| e.name.as_str()).collect();
        assert!(
            sub_names.contains(&"a.txt"),
            "sub missing a.txt: {sub_names:?}"
        );
        assert!(
            sub_names.contains(&"deep"),
            "sub missing deep: {sub_names:?}"
        );

        let deep = prov.list(Path::new("sub/deep")).unwrap();
        let deep_names: Vec<_> = deep.iter().map(|e| e.name.as_str()).collect();
        assert!(
            deep_names.contains(&"c.txt"),
            "deep missing c.txt: {deep_names:?}"
        );
    }
}
