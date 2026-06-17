//! Remote marketplace client — fetches the R2 registry and downloads extension
//! zips for installation. The actual install step delegates to `ext::install`.

use anyhow::Result;
use atlas_model::ext_model::RemoteRegistry;
use std::io::Write as _;
use std::path::PathBuf;

const REGISTRY_BASE: &str = "https://pub-89dd9e8d0b06444ca2f59e0dc245dbc9.r2.dev";

/// Download the remote catalogue from `{REGISTRY_BASE}/registry.json`.
pub async fn fetch_registry() -> Result<RemoteRegistry> {
    let url = format!("{REGISTRY_BASE}/registry.json");
    let registry: RemoteRegistry = reqwest::get(&url).await?.error_for_status()?.json().await?;
    Ok(registry)
}

/// Download a zip from `download_url` to a temporary file, returning the path.
/// The caller is responsible for deleting the file after installation.
pub async fn download_zip(download_url: &str) -> Result<PathBuf> {
    let bytes = reqwest::get(download_url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(&bytes)?;

    // Persist so the path outlives this scope; caller installs then can clean up.
    let (_, path) = tmp.keep()?;
    Ok(path)
}

/// Download an icon from `url` to `dest`.
pub async fn download_icon(url: &str, dest: &std::path::Path) -> Result<()> {
    let bytes = reqwest::get(url).await?.error_for_status()?.bytes().await?;
    let mut file = std::fs::File::create(dest)?;
    file.write_all(&bytes)?;
    Ok(())
}
