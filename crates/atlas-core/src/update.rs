use anyhow::Result;
use serde::Deserialize;

const UPDATE_MANIFEST_URL: &str =
    "https://pub-89dd9e8d0b06444ca2f59e0dc245dbc9.r2.dev/app/windows/latest.json";

#[derive(Debug, Deserialize)]
pub struct AppUpdateManifest {
    pub version: String,
    pub notes: String,
    pub windows: WindowsUpdate,
}

#[derive(Debug, Deserialize)]
pub struct WindowsUpdate {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub async fn check_for_update(current_version: &str) -> Result<Option<AppUpdateManifest>> {
    let manifest: AppUpdateManifest = reqwest::get(UPDATE_MANIFEST_URL)
        .await?
        .error_for_status()?
        .json()
        .await?;

    if manifest.version == current_version {
        return Ok(None);
    }

    Ok(Some(manifest))
}
