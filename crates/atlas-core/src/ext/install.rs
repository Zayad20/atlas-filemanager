// reason: extension install/uninstall is a one-shot user action off the gpui
// render thread; the sync-fs ban targets blocking the UI foreground.
#![allow(clippy::disallowed_methods)]
//! Install / uninstall extensions on disk. An extension is a directory under
//! [`super::extensions_dir`] containing an `extension.toml`. Installing accepts a
//! `.zip` bundle or a folder; uninstalling removes the directory.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

use super::manifest::Manifest;
pub use super::manifest::{is_valid_extension_id, validate_extension_id};

/// Resolve and validate the target directory for an extension ID (SEC-01).
///
/// Ensures the ID is well-formed ([a-z0-9_-]+), contains no traversal components,
/// and that `extensions_dir().join(id)` resides strictly inside `extensions_dir()`.
pub fn resolve_extension_dir(id: &str) -> anyhow::Result<PathBuf> {
    validate_extension_id(id)?;
    let base = super::extensions_dir();
    let dest = base.join(id);

    // Lexical containment checks
    if !dest.starts_with(&base) || dest == base {
        anyhow::bail!(
            "Extension path {:?} does not reside inside extensions directory {:?}",
            dest,
            base
        );
    }
    let rel = dest.strip_prefix(&base)?;
    let comps: Vec<_> = rel.components().collect();
    if comps.len() != 1 || !matches!(comps[0], std::path::Component::Normal(_)) {
        anyhow::bail!("Extension ID {:?} produces invalid path components", id);
    }

    // Canonical check if base directory already exists
    if let Ok(canon_base) = base.canonicalize() {
        if dest.exists() {
            if let Ok(canon_dest) = dest.canonicalize() {
                if !canon_dest.starts_with(&canon_base) || canon_dest == canon_base {
                    anyhow::bail!(
                        "Canonical extension path {:?} escapes extensions directory {:?}",
                        canon_dest,
                        canon_base
                    );
                }
            }
        }
    }

    Ok(dest)
}

/// Validate `extension.toml` in `dir` and return its parsed manifest.
fn read_manifest(dir: &Path) -> anyhow::Result<Manifest> {
    let text =
        std::fs::read_to_string(dir.join("extension.toml")).context("extension.toml not found")?;
    Manifest::parse(&text)
}

/// Install from a folder by copying it to `<extensions>/<id>`.
pub fn install_folder(src: &Path) -> anyhow::Result<String> {
    let manifest = read_manifest(src)?;
    let dest = resolve_extension_dir(&manifest.id)?;
    copy_dir(src, &dest)?;
    Ok(manifest.id)
}

/// Install from a `.zip` whose root contains `extension.toml`.
pub fn install_zip(zip_path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Extract to a temp dir first, then validate + move into place.
    let tmp = std::env::temp_dir().join(format!("fp-ext-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&tmp)?;
    archive.extract(&tmp).context("unzip extension")?;

    // The manifest may be at the zip root or in a single top-level folder.
    let root = if tmp.join("extension.toml").is_file() {
        tmp.clone()
    } else {
        std::fs::read_dir(&tmp)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.join("extension.toml").is_file())
            .ok_or_else(|| anyhow!("no extension.toml in zip"))?
    };

    let manifest = read_manifest(&root)?;
    let dest = resolve_extension_dir(&manifest.id)?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest).ok();
    }
    copy_dir(&root, &dest)?;
    std::fs::remove_dir_all(&tmp).ok();
    Ok(manifest.id)
}

/// Remove an installed extension by id. Builtins (no on-disk dir) are a no-op.
pub fn uninstall(id: &str) -> anyhow::Result<()> {
    let dir = resolve_extension_dir(id)?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).context("remove extension dir")?;
    }
    Ok(())
}

/// Write/update a manifest-only bundled extension on disk.
pub fn ensure_bundled(id: &str, manifest_toml: &str) -> anyhow::Result<()> {
    let dir = resolve_extension_dir(id)?;
    let manifest_path = dir.join("extension.toml");
    std::fs::create_dir_all(&dir)?;
    let current = std::fs::read_to_string(&manifest_path).unwrap_or_default();
    if current != manifest_toml {
        std::fs::write(&manifest_path, manifest_toml)?;
    }
    Ok(())
}

/// Seed or update one official bundled asset when its packaged bytes change.
pub fn ensure_asset(id: &str, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = resolve_extension_dir(id)?;
    let path = dir.join(name);
    let current = std::fs::read(&path).unwrap_or_default();
    if current != bytes {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

/// Seed the official icon provider from the application package on first use.
/// This runs on the extension worker, preserving confined sidecar execution.
pub(crate) fn ensure_packaged_icon_provider(directory: &Path) -> anyhow::Result<()> {
    let filename = if cfg!(windows) { "atlas-icon-extension.exe" } else { "atlas-icon-extension" };
    let source = std::env::current_exe()?.parent().context("application has no directory")?.join(filename);
    let target = directory.join(filename);
    if !source.is_file() && target.is_file() {
        return Ok(()); // Independently installed provider in a development build.
    }
    copy_packaged_program(&source, &target)
}

fn copy_packaged_program(source: &Path, target: &Path) -> anyhow::Result<()> {
    use sha2::{Digest as _, Sha256};
    use std::io::Write as _;
    let bytes = std::fs::read(source).context("Packaged icon provider is missing; reinstall Atlas or the official icon extension")?;
    if std::fs::read(target).is_ok_and(|current| Sha256::digest(&current) == Sha256::digest(&bytes)) {
        return Ok(());
    }
    let directory = target.parent().context("provider has no destination directory")?;
    std::fs::create_dir_all(directory)?;
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    staged.write_all(&bytes)?;
    staged.as_file().sync_all()?;
    std::fs::set_permissions(staged.path(), std::fs::metadata(source)?.permissions())?;
    staged.persist(target).map_err(|error| error.error).context("Cannot update the icon provider; close other Atlas operations and retry")?;
    Ok(())
}

/// Seed a bundled extension and copy any bundled assets that are not already present.
pub fn ensure_bundled_with_assets(id: &str, manifest_toml: &str, src: &Path) -> anyhow::Result<()> {
    ensure_bundled(id, manifest_toml)?;
    let dir = resolve_extension_dir(id)?;
    if src.is_dir() {
        copy_dir_missing(src, &dir)?;
    }
    Ok(())
}

/// Recursively copy `src` into `dest`.
fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_dir_missing(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_missing(&from, &to)?;
        } else if !to.exists() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_provider_seeds_a_clean_profile_and_replaces_an_old_build() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("packaged.exe");
        let target = directory.path().join("fresh-profile/extensions/icon-manager/provider.exe");
        std::fs::write(&source, b"new-build").unwrap();
        copy_packaged_program(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new-build");
        std::fs::write(&target, b"old-build-with-a-longer-payload").unwrap();
        copy_packaged_program(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new-build");
        copy_packaged_program(&source, &target).unwrap();
        assert_eq!(std::fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn test_resolve_extension_dir_rejects_path_traversal() {
        let dangerous_ids = [
            "../../../.ssh",
            "../etc/passwd",
            "/absolute/path",
            "nested/../../secret",
            "extension:name",
            "ext;rm -rf",
            "..",
            ".",
            "sub/dir",
            "sub\\dir",
        ];

        for id in &dangerous_ids {
            assert!(
                resolve_extension_dir(id).is_err(),
                "Extension ID '{id}' must be rejected for path traversal or invalid characters"
            );
            assert!(
                uninstall(id).is_err(),
                "Uninstalling with ID '{id}' must be rejected"
            );
        }

        let valid_ids = ["simple-ext", "my_extension_123", "theme-dark"];
        for id in &valid_ids {
            let res = resolve_extension_dir(id);
            assert!(res.is_ok(), "Valid ID '{id}' must be accepted");
            let path = res.unwrap();
            assert!(
                path.starts_with(crate::ext::extensions_dir()),
                "Path must reside within extensions_dir"
            );
        }
    }
}

