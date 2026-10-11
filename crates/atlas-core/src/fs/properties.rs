//! Filesystem metadata and permission edits for the Properties dialog.
// reason: callers run permission edits on a background executor; metadata reads
// are a one-shot snapshot, never part of rendering.+#![allow(clippy::disallowed_methods)]

use atlas_model::file_entry::FileKind;
use atlas_model::overlay::PropertyRow;
use std::path::Path;

pub fn read_properties(path: &Path) -> PropertyRow {
    let result = std::fs::symlink_metadata(path);
    let meta = result.as_ref().ok();
    let is_symlink = meta.is_some_and(|m| m.file_type().is_symlink());
    let kind = if is_symlink {
        FileKind::Symlink
    } else if meta.is_some_and(std::fs::Metadata::is_dir) {
        FileKind::Dir
    } else {
        FileKind::File
    };
    PropertyRow {
        name: path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        path: path.to_path_buf(),
        kind,
        size: meta.filter(|m| m.is_file()).map(std::fs::Metadata::len),
        created: meta.and_then(|m| m.created().ok()),
        modified: meta.and_then(|m| m.modified().ok()),
        accessed: meta.and_then(|m| m.accessed().ok()),
        readonly: meta.is_some_and(|m| m.permissions().readonly()),
        is_symlink,
        link_target: is_symlink.then(|| std::fs::read_link(path).ok()).flatten(),
        error: result.as_ref().err().map(ToString::to_string),
        #[cfg(unix)]
        permissions: meta.map(|m| {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let mode = m.permissions().mode() & 0o7777;
            atlas_model::overlay::UnixPermissions {
                uid: m.uid(),
                gid: m.gid(),
                device: m.dev(),
                inode: m.ino(),
                mode,
                edited_mode: mode,
            }
        }),
    }
}

/// Edit only rwx bits on the original inode, preserving special mode bits.
/// Reject links and stale snapshots rather than modifying a different target.
#[cfg(unix)]
pub fn apply_permissions(
    path: &Path,
    expected: &atlas_model::overlay::UnixPermissions,
) -> std::io::Result<()> {
    use rustix::fs::{open, Mode, OFlags};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // O_PATH allows chmod even when the owner has removed all read/write access.
    // NOFOLLOW pins a symlink itself so the metadata check below rejects it.
    #[cfg(target_os = "linux")]
    let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file = std::fs::File::from(open(path, flags, Mode::empty())?);
    let meta = file.metadata()?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Symbolic link permissions cannot be edited",
        ));
    }
    if meta.dev() != expected.device
        || meta.ino() != expected.inode
        || (meta.permissions().mode() & 0o7777) != expected.mode
    {
        return Err(std::io::Error::other(
            "The item or its permissions changed; reopen Properties before applying",
        ));
    }
    let mode = (meta.permissions().mode() & !0o777) | (expected.edited_mode & 0o777);
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // fchmod does not accept O_PATH handles. /proc/self/fd follows our pinned
        // handle, so renames/replacements of the original path cannot redirect chmod.
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", file.as_raw_fd()),
            std::fs::Permissions::from_mode(mode),
        )
    }
    #[cfg(not(target_os = "linux"))]
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn permission_edit_preserves_special_bits_and_directory_contents() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let child = folder.join("child");
        std::fs::write(&child, b"contents").unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o1755)).unwrap();
        let mut permissions = read_properties(&folder).permissions.unwrap();
        permissions.edited_mode = 0o700;
        apply_permissions(&folder, &permissions).unwrap();
        assert_eq!(read_properties(&folder).permissions.unwrap().mode, 0o1700);
        assert_eq!(read_properties(&child).permissions.unwrap().mode, 0o640);
    }

    #[test]
    fn links_include_their_target_and_cannot_change_target_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        let row = read_properties(&link);
        assert_eq!(row.kind, FileKind::Symlink);
        assert_eq!(row.link_target.as_deref(), Some(target.as_path()));
        let mut permissions = row.permissions.unwrap();
        permissions.edited_mode = 0o777;
        assert!(apply_permissions(&link, &permissions).is_err());
        assert_eq!(read_properties(&target).permissions.unwrap().mode, 0o750);
        std::fs::remove_dir(&target).unwrap();
        assert!(read_properties(&link).error.is_none());
        assert_eq!(read_properties(&link).kind, FileKind::Symlink);
    }

    #[test]
    fn stale_mode_or_replaced_inode_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut permissions = read_properties(&path).permissions.unwrap();
        permissions.edited_mode = 0o600;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(apply_permissions(&path, &permissions).is_err());
        assert_eq!(read_properties(&path).permissions.unwrap().mode, 0o640);
        std::fs::rename(&path, dir.path().join("original")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(apply_permissions(&path, &permissions).is_err());
        assert_eq!(read_properties(&path).permissions.unwrap().mode, 0o644);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owner_can_restore_access_to_an_unreadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"contents").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0)).unwrap();
        let mut permissions = read_properties(&path).permissions.unwrap();
        permissions.edited_mode = 0o600;
        apply_permissions(&path, &permissions).unwrap();
        assert_eq!(read_properties(&path).permissions.unwrap().mode, 0o600);
    }

    #[test]
    fn missing_item_reports_metadata_error() {
        let dir = tempfile::tempdir().unwrap();
        let row = read_properties(&dir.path().join("missing"));
        assert!(row.error.is_some());
        assert!(row.permissions.is_none());
    }
}
