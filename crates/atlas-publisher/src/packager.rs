use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_author")]
    pub author: String,
    pub version: String,
}

fn default_author() -> String {
    "Unknown".to_string()
}

pub fn parse_manifest(dir: &Path) -> Result<ExtensionManifest> {
    let manifest_path = dir.join("extension.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {:?}", manifest_path))?;

    let manifest: ExtensionManifest =
        toml::from_str(&content).with_context(|| "Failed to parse extension.toml")?;

    Ok(manifest)
}

pub fn zip_extension(source_dir: &Path, output_path: &Path) -> Result<()> {
    let file = File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let walker = WalkDir::new(source_dir).into_iter();
    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip hidden files and the build artifact if it's inside
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }

        let name = path.strip_prefix(source_dir).unwrap();
        if path.is_file() {
            zip.start_file(name.to_string_lossy().replace("\\", "/"), options)?;
            let mut f = File::open(path)?;
            let mut buffer = Vec::new();
            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
        } else if !name.as_os_str().is_empty() {
            zip.add_directory(name.to_string_lossy().replace("\\", "/"), options)?;
        }
    }

    zip.finish()?;
    Ok(())
}
