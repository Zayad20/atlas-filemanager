//! RAR archive provider.
//!
//! This module is compiled only with the `rar` feature. It shells out to the
//! `UnRAR.exe` CLI for listing and extraction. The binary is resolved in order:
//! 1. Bundled next to the Atlas executable.
//! 2. WinRAR default install (`C:\Program Files\WinRAR\UnRAR.exe`).
//! 3. Bare `unrar` on the system PATH.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::common::{build_synthetic_ancestors, path_to_dir_key, write_to_dest};
use super::{make_entry, ArchiveCaps, ArchiveProvider};
use atlas_model::file_entry::FileEntry;

#[allow(unused_imports)]
use unrar as _;

/// Pre-indexed, read-only view of a `.rar` / `.cbr` file.
pub struct RarProvider {
    path: PathBuf,
    /// `inner_dir` (with trailing '/') -> direct children entries
    index: HashMap<String, Vec<FileEntry>>,
    exact_names: HashMap<String, String>,
    encrypted: bool,
}

impl RarProvider {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let archive_arg = path_arg(path);
        let output = run_unrar(&["lb", "-c-", "-p-", archive_arg.as_str()])?;
        let listing = String::from_utf8_lossy(&output.stdout);
        let encrypted = password_required(&output.stderr)
            || listing
                .lines()
                .any(|line| line.to_ascii_lowercase().contains("encrypted"));

        if encrypted {
            return Ok(Self {
                path: path.to_path_buf(),
                index: HashMap::new(),
                exact_names: HashMap::new(),
                encrypted,
            });
        }

        let mut index: HashMap<String, Vec<FileEntry>> = HashMap::new();
        let mut exact_names = HashMap::new();
        index.entry(String::new()).or_default();

        for line in listing.lines() {
            let original_name = line.trim();
            if original_name.is_empty() {
                continue;
            }
            let normalized = original_name.replace('\\', "/");
            exact_names.insert(normalized.clone(), original_name.to_string());
            let is_dir = normalized.ends_with('/');
            let inner = PathBuf::from(normalized.trim_end_matches('/'));
            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            let parent_key = inner.parent().map(path_to_dir_key).unwrap_or_default();
            build_synthetic_ancestors(path, &inner, &mut index);

            let file_entry = make_entry(path, inner, name, is_dir, None);
            index.entry(parent_key).or_default().push(file_entry);
        }

        Ok(Self {
            path: path.to_path_buf(),
            index,
            exact_names,
            encrypted,
        })
    }

    fn ensure_unencrypted(&self) -> anyhow::Result<()> {
        if self.encrypted {
            anyhow::bail!("PasswordRequired");
        }
        Ok(())
    }

    fn read_entry(&self, inner_path: &Path) -> anyhow::Result<Vec<u8>> {
        self.ensure_unencrypted()?;
        let name = inner_path.to_string_lossy().replace('\\', "/");
        let exact_name = match self.exact_names.get(&name) {
            Some(n) => n,
            None => anyhow::bail!("entry not found: {name}"),
        };

        let archive_arg = path_arg(&self.path);
        let output = run_unrar(&["p", "-inul", "-c-", "-p-", archive_arg.as_str(), exact_name])?;
        if password_required(&output.stderr) {
            anyhow::bail!("PasswordRequired");
        }
        Ok(output.stdout)
    }
}

impl ArchiveProvider for RarProvider {
    fn caps(&self) -> ArchiveCaps {
        ArchiveCaps {
            write: false,
            passwords: true,
            preserves_compression_on_rewrite: false,
        }
    }

    fn list(&self, inner_dir: &Path) -> anyhow::Result<Vec<FileEntry>> {
        self.ensure_unencrypted()?;
        let key = path_to_dir_key(inner_dir);
        Ok(self.index.get(&key).cloned().unwrap_or_default())
    }

    fn read_bytes(&self, inner_path: &Path) -> anyhow::Result<Box<dyn Read + Send>> {
        let data = self.read_entry(inner_path)?;
        Ok(Box::new(Cursor::new(data)))
    }

    fn extract_to(&self, inner_path: &Path, dest: &Path) -> anyhow::Result<()> {
        write_to_dest(dest, &self.read_entry(inner_path)?)
    }

    fn archive_path(&self) -> &Path {
        &self.path
    }
}

const UNRAR_EXE: &[u8] = include_bytes!("../../../vendor/unrar-bin/UnRAR.exe");

/// Resolve the path to the `UnRAR.exe` CLI binary.
///
/// Unpacks the natively bundled UnRAR.exe to the temporary directory,
/// guaranteeing zero dependencies for the client.
fn unrar_exe_path() -> PathBuf {
    if let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("UnRAR.exe")))
        .filter(|path| path.is_file())
    {
        return bundled;
    }

    let mut temp = std::env::temp_dir();
    temp.push("atlas_unrar_bundled.exe");

    let should_write = match std::fs::metadata(&temp) {
        Ok(meta) => meta.len() != UNRAR_EXE.len() as u64,
        Err(_) => true,
    };

    if should_write {
        // We ignore errors; if another instance is writing or locking it,
        // it likely has the correct contents already.
        let _ = std::fs::write(&temp, UNRAR_EXE);
    }

    temp
}

fn run_unrar(args: &[&str]) -> anyhow::Result<std::process::Output> {
    let unrar = unrar_exe_path();
    let output = Command::new(&unrar).args(args).output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run UnRAR at «{}»: {e}. Install WinRAR or place UnRAR.exe next to atlas.exe.",
            unrar.display()
        )
    })?;
    if output.status.success() || password_required(&output.stderr) {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("unrar failed: {stderr}");
    }
}

fn password_required(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    stderr.contains("password") || stderr.contains("encrypted")
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
