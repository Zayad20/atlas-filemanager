#![allow(clippy::disallowed_methods)]

use anyhow::{Context as _, Result};
use atlas_model::ext_model::{SidecarCommandRequest, SidecarCommandResponse};
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let request: SidecarCommandRequest = serde_json::from_reader(std::io::stdin())?;
    anyhow::ensure!(request.version == 1, "unsupported command protocol version");
    let response = match request.command_id.as_str() {
        "icon-manager.download-theme" => download_theme(&request).await,
        other => anyhow::bail!("unsupported icon-manager command: {other}"),
    };
    let response = match response {
        Ok(()) => SidecarCommandResponse {
            error: None,
            changed_dirs: Vec::new(),
        },
        Err(error) => SidecarCommandResponse {
            error: Some(error.to_string()),
            changed_dirs: Vec::new(),
        },
    };
    serde_json::to_writer(std::io::stdout(), &response)?;
    Ok(())
}

async fn download_theme(request: &SidecarCommandRequest) -> Result<()> {
    let extension_id = request
        .answers
        .get("theme")
        .map(String::as_str)
        .unwrap_or("PKief/material-icon-theme")
        .trim();
    anyhow::ensure!(
        valid_extension_id(extension_id),
        "invalid Open VSX extension id"
    );

    let client = reqwest::Client::new();
    let metadata: serde_json::Value = client
        .get(format!("https://open-vsx.org/api/{extension_id}/latest"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let download_url = metadata
        .pointer("/files/download")
        .and_then(serde_json::Value::as_str)
        .context("Open VSX response has no download URL")?;
    let bytes = client
        .get(download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let slug = extension_id.replace('/', "_");
    let theme_root = request.data_dir.join(&slug);
    if theme_root.exists() {
        std::fs::remove_dir_all(&theme_root)?;
    }
    std::fs::create_dir_all(&theme_root)?;
    extract_zip(&bytes, &theme_root)?;
    build_mappings(&request.data_dir, &theme_root)?;
    Ok(())
}

fn valid_extension_id(value: &str) -> bool {
    let mut parts = value.split('/');
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    parts.next().is_some_and(valid) && parts.next().is_some_and(valid) && parts.next().is_none()
}

fn extract_zip(bytes: &[u8], destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(output)?;
        } else {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(output)?;
            std::io::copy(&mut entry, &mut file)?;
        }
    }
    Ok(())
}

fn build_mappings(data_dir: &Path, theme_root: &Path) -> Result<()> {
    let package_path = theme_root.join("extension/package.json");
    let package: serde_json::Value = serde_json::from_reader(std::fs::File::open(package_path)?)?;
    let themes = package
        .pointer("/contributes/iconThemes")
        .and_then(serde_json::Value::as_array)
        .context("extension contains no icon theme")?;
    let theme_path = themes
        .first()
        .and_then(|theme| theme.get("path"))
        .and_then(serde_json::Value::as_str)
        .context("icon theme has no path")?
        .trim_start_matches("./");
    let theme_file = theme_root.join("extension").join(theme_path);
    let contents = std::fs::read_to_string(&theme_file)?;
    let clean = contents
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let theme: serde_json::Value = serde_json::from_str(&clean)?;
    let extensions = theme
        .get("fileExtensions")
        .and_then(serde_json::Value::as_object)
        .context("theme contains no file extension mappings")?;
    let definitions = theme
        .get("iconDefinitions")
        .and_then(serde_json::Value::as_object)
        .context("theme contains no icon definitions")?;
    let theme_dir = theme_file
        .parent()
        .context("theme has no parent directory")?;
    let mut mappings = serde_json::Map::new();
    for (extension, icon_id) in extensions {
        let Some(icon_path) = icon_id
            .as_str()
            .and_then(|id| definitions.get(id))
            .and_then(|definition| definition.get("iconPath"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let path = normalize_icon_path(&theme_root.join("extension"), theme_dir, icon_path)?;
        mappings.insert(
            extension.clone(),
            serde_json::Value::String(path.to_string_lossy().into_owned()),
        );
    }
    let mut output = std::fs::File::create(data_dir.join("theme_mappings.json"))?;
    serde_json::to_writer(&mut output, &mappings)?;
    output.flush()?;
    Ok(())
}

fn normalize_icon_path(root: &Path, base: &Path, relative: &str) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let path = base
        .join(relative.trim_start_matches("./"))
        .canonicalize()?;
    anyhow::ensure!(
        path.starts_with(root),
        "icon path escapes extension directory"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_marketplace_ids() {
        assert!(valid_extension_id("PKief/material-icon-theme"));
        assert!(!valid_extension_id("PKief/../../evil"));
        assert!(!valid_extension_id("single"));
    }
}
