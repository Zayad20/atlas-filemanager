//! Windows-specific cloud mount helpers.

use std::path::Path;

/// True if WinFsp kernel driver is installed (required for FUSE-style drive mounting via rclone).
/// Checks the SCM service first (covers standalone WinFsp, SSHFS-Win bundled, Bitvise bundled, etc.),
/// then falls back to known DLL paths for the standard MSI installer.
pub fn is_winfsp_installed() -> bool {
    // Service check: `sc query WinFsp` exits 0 when the service exists.
    if let Ok(status) = std::process::Command::new("sc.exe")
        .args(["query", "WinFsp"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        if status.success() {
            return true;
        }
    }
    // Fallback: DLL presence for standard MSI layout.
    Path::new(r"C:\Program Files (x86)\WinFsp\bin\winfsp-x64.dll").exists()
        || Path::new(r"C:\Program Files\WinFsp\bin\winfsp-x64.dll").exists()
}

/// Download and install WinFsp silently. Requires the process to be elevated
/// (UAC prompt is shown by Windows when running msiexec /passive).
/// `progress` is called with `(bytes_done, bytes_total)` during download.
pub async fn install_winfsp(progress: impl Fn(u64, u64) + Send + 'static) -> anyhow::Result<()> {
    use anyhow::Context;

    const URL: &str =
        "https://github.com/winfsp/winfsp/releases/download/v2.0/winfsp-2.0.23075.msi";

    let msi_path = crate::util::config::data_dir().join("winfsp-installer.msi");

    // Download the MSI.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut response = client
        .get(URL)
        .send()
        .await
        .context("download WinFsp")?
        .error_for_status()
        .context("WinFsp download HTTP error")?;

    let total = response.content_length().unwrap_or(0);
    let mut done = 0u64;
    let mut file = tokio::fs::File::create(&msi_path)
        .await
        .context("create winfsp MSI file")?;

    while let Some(chunk) = response.chunk().await.context("WinFsp download chunk")? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("write MSI chunk")?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    drop(file);

    // Run the installer. /passive shows a progress bar but no dialogs (UAC
    // is still triggered by Windows for the kernel driver installation).
    let status = tokio::process::Command::new("msiexec")
        .args(["/i", &msi_path.to_string_lossy(), "/passive", "/norestart"])
        .status()
        .await
        .context("run msiexec")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "WinFsp installer exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}
