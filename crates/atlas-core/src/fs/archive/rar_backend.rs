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

#[cfg(unix)]
fn resolve_unrar_binary() -> anyhow::Result<PathBuf> {
    // 1. Check if unrar next to atlas executable exists
    if let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("unrar")))
        .filter(|path| path.is_file())
    {
        return Ok(bundled);
    }

    // 2. Check if which unrar returns a valid binary
    if let Ok(output) = Command::new("which").arg("unrar").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                let path = PathBuf::from(path_str);
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
    }

    // 3. Search in system PATH
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("unrar");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    // 4. Test executing `unrar` directly
    if Command::new("unrar").arg("-?").output().is_ok() {
        return Ok(PathBuf::from("unrar"));
    }

    anyhow::bail!(
        "El binario del sistema 'unrar' no está disponible. Se requiere instalar 'unrar' en el sistema (ej. 'sudo apt install unrar' o 'sudo pacman -S unrar') para abrir archivos RAR."
    )
}

#[cfg(windows)]
const UNRAR_EXE: &[u8] = include_bytes!("../../../vendor/unrar-bin/UnRAR.exe");

#[cfg(windows)]
fn resolve_unrar_binary() -> anyhow::Result<PathBuf> {
    use sha2::{Digest, Sha256};

    // 1. Check if bundled next to atlas.exe
    if let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("UnRAR.exe")))
        .filter(|path| path.is_file())
    {
        return Ok(bundled);
    }

    // 2. WinRAR default install
    let default_winrar = PathBuf::from(r"C:\Program Files\WinRAR\UnRAR.exe");
    if default_winrar.is_file() {
        return Ok(default_winrar);
    }

    // 3. User private data_dir/bin (SEC-02)
    let bin_dir = crate::util::config::data_dir().join("bin");
    std::fs::create_dir_all(&bin_dir)?;
    let target = bin_dir.join("UnRAR.exe");

    let expected_hash = Sha256::digest(UNRAR_EXE);

    let needs_write = match std::fs::read(&target) {
        Ok(existing) => Sha256::digest(&existing) != expected_hash,
        Err(_) => true,
    };

    if needs_write {
        let tmp = bin_dir.join(format!("unrar-{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, UNRAR_EXE)?;
        if target.exists() {
            let _ = std::fs::remove_file(&target);
        }
        std::fs::rename(&tmp, &target).or_else(|_| std::fs::write(&target, UNRAR_EXE))?;
    }

    Ok(target)
}

#[allow(dead_code)]
fn unrar_exe_path() -> anyhow::Result<PathBuf> {
    resolve_unrar_binary()
}

fn run_unrar(args: &[&str]) -> anyhow::Result<std::process::Output> {
    let unrar = resolve_unrar_binary()?;
    let mut cmd = Command::new(&unrar);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd.output().map_err(|e| {
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
