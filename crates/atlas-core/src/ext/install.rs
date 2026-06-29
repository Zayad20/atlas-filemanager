// reason: extension install/uninstall is a one-shot user action off the gpui
// render thread; the sync-fs ban targets blocking the UI foreground.
#![allow(clippy::disallowed_methods)]
//! Install / uninstall extensions on disk. An extension is a directory under
//! [`super::extensions_dir`] containing an `extension.toml`. Installing accepts a
//! `.zip` bundle or a folder; uninstalling removes the directory.

use std::path::Path;

use anyhow::{anyhow, Context};

use super::manifest::Manifest;

/// Validate `extension.toml` in `dir` and return its parsed manifest.
fn read_manifest(dir: &Path) -> anyhow::Result<Manifest> {
    let text =
        std::fs::read_to_string(dir.join("extension.toml")).context("extension.toml not found")?;
    Manifest::parse(&text)
}

/// Install from a folder by copying it to `<extensions>/<id>`.
pub fn install_folder(src: &Path) -> anyhow::Result<String> {
    let manifest = read_manifest(src)?;
    let dest = super::extensions_dir().join(&manifest.id);
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
    let dest = super::extensions_dir().join(&manifest.id);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).ok();
    }
    copy_dir(&root, &dest)?;
    std::fs::remove_dir_all(&tmp).ok();
    Ok(manifest.id)
}

/// Remove an installed extension by id. Builtins (no on-disk dir) are a no-op.
pub fn uninstall(id: &str) -> anyhow::Result<()> {
    let dir = super::extensions_dir().join(id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).context("remove extension dir")?;
    }
    Ok(())
}

/// Write/update a manifest-only bundled extension on disk.
pub fn ensure_bundled(id: &str, manifest_toml: &str) -> anyhow::Result<()> {
    let dir = super::extensions_dir().join(id);
    let manifest_path = dir.join("extension.toml");
    std::fs::create_dir_all(&dir)?;
    let current = std::fs::read_to_string(&manifest_path).unwrap_or_default();
    if current != manifest_toml {
        std::fs::write(&manifest_path, manifest_toml)?;
    }
    Ok(())
}

/// Seed a bundled extension and copy any bundled assets that are not already present.
pub fn ensure_bundled_with_assets(id: &str, manifest_toml: &str, src: &Path) -> anyhow::Result<()> {
    ensure_bundled(id, manifest_toml)?;
    if src.is_dir() {
        copy_dir_missing(src, &super::extensions_dir().join(id))?;
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
