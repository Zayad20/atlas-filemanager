mod packager;
mod s3_client;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use s3_client::{R2Client, RegistryExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Publish an extension to R2
    Publish {
        /// The path to the extension directory
        path: PathBuf,
        /// The R2 bucket name
        #[arg(long, env = "R2_BUCKET")]
        bucket: String,
        /// The public URL mapping for the bucket
        #[arg(long, env = "R2_PUBLIC_URL")]
        public_url: String,
    },
    /// Publish a Windows installer and update app/windows/latest.json
    PublishApp {
        /// The installer exe to upload
        installer: PathBuf,
        /// The app version in the installer
        #[arg(long)]
        version: String,
        /// Release notes shown by the updater
        #[arg(long, default_value = "Atlas update")]
        notes: String,
        /// The R2 bucket name
        #[arg(long, env = "R2_BUCKET")]
        bucket: String,
        /// The public URL mapping for the bucket
        #[arg(long, env = "R2_PUBLIC_URL")]
        public_url: String,
    },
}

#[derive(Serialize)]
struct AppUpdateManifest {
    version: String,
    notes: String,
    published_at: String,
    windows: WindowsUpdate,
}

#[derive(Serialize)]
struct WindowsUpdate {
    url: String,
    sha256: String,
    size: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Publish {
            path,
            bucket,
            public_url,
        } => {
            println!("Publishing extension from: {:?}", path);

            // 1. Parse manifest
            let manifest = packager::parse_manifest(path)?;
            println!("Parsed manifest: {} v{}", manifest.name, manifest.version);

            // 2. Zip extension
            let temp_zip_path =
                std::env::temp_dir().join(format!("{}-{}.zip", manifest.id, manifest.version));
            println!("Packaging extension to: {:?}", temp_zip_path);
            packager::zip_extension(path, &temp_zip_path)?;
            let zip_sha256 = sha256_file(&temp_zip_path)?;

            // 3. Connect to R2
            let client = R2Client::new(bucket.clone(), public_url.clone()).await?;

            // 4. Upload zip
            let zip_key = format!(
                "extensions/{}/versions/{}.zip",
                manifest.id, manifest.version
            );
            println!("Uploading zip to R2: {}", zip_key);
            client
                .upload_file(
                    &temp_zip_path,
                    &zip_key,
                    "application/zip",
                    "public, max-age=31536000, immutable",
                )
                .await?;

            // 5. Upload a versioned icon. Prefer SVG for crisp UI rendering.
            let mut icon_url = None;
            let svg_path = path.join("icon.svg");
            let png_path = path.join("icon.png");
            let icon = if svg_path.exists() {
                Some((svg_path, "svg", "image/svg+xml"))
            } else if png_path.exists() {
                ensure_png(&png_path)?;
                Some((png_path, "png", "image/png"))
            } else {
                None
            };
            if let Some((icon_path, extension, content_type)) = icon {
                let icon_key = format!(
                    "extensions/{}/versions/{}/icon.{extension}",
                    manifest.id, manifest.version
                );
                println!("Uploading icon to R2: {}", icon_key);
                client
                    .upload_file(
                        &icon_path,
                        &icon_key,
                        content_type,
                        "public, max-age=31536000, immutable",
                    )
                    .await?;
                icon_url = Some(client.build_public_url(&icon_key));
            }

            // 6. Update registry
            println!("Updating registry.json...");
            let mut registry = client.get_registry().await?;
            registry.last_updated = Utc::now().to_rfc3339();

            let ext_entry = RegistryExtension {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                description: manifest.description.clone().unwrap_or_default(),
                author: manifest.author.clone(),
                icon_url,
                latest_version: manifest.version.clone(),
                download_url: client.build_public_url(&zip_key),
                sha256: zip_sha256,
            };

            registry.extensions.insert(manifest.id.clone(), ext_entry);

            let registry_json = serde_json::to_vec_pretty(&registry)?;
            client
                .upload_bytes(
                    registry_json,
                    "registry.json",
                    "application/json",
                    "no-cache",
                )
                .await?;

            println!(
                "Successfully published {} v{}",
                manifest.name, manifest.version
            );

            // Clean up temp file
            let _ = std::fs::remove_file(temp_zip_path);
        }
        Commands::PublishApp {
            installer,
            version,
            notes,
            bucket,
            public_url,
        } => {
            let file_name = installer
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("AtlasSetup.exe");
            let installer_key = format!("app/windows/versions/{file_name}");
            let latest_key = "app/windows/latest.json";
            let size = std::fs::metadata(installer)?.len();
            let sha256 = sha256_file(installer)?;
            let client = R2Client::new(bucket.clone(), public_url.clone()).await?;

            println!("Uploading installer to R2: {installer_key}");
            client
                .upload_file(
                    installer,
                    &installer_key,
                    "application/vnd.microsoft.portable-executable",
                    "public, max-age=31536000, immutable",
                )
                .await?;

            let manifest = AppUpdateManifest {
                version: version.clone(),
                notes: notes.clone(),
                published_at: Utc::now().to_rfc3339(),
                windows: WindowsUpdate {
                    url: client.build_public_url(&installer_key),
                    sha256,
                    size,
                },
            };

            println!("Updating {latest_key}");
            client
                .upload_bytes(
                    serde_json::to_vec_pretty(&manifest)?,
                    latest_key,
                    "application/json",
                    "no-cache",
                )
                .await?;

            println!(
                "Published Atlas {}: {}",
                version,
                client.build_public_url(latest_key)
            );
        }
    }

    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 64 * 1024];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn ensure_png(path: &Path) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut signature = [0_u8; 8];
    file.read_exact(&mut signature)?;
    anyhow::ensure!(
        signature == [137, 80, 78, 71, 13, 10, 26, 10],
        "{} is named icon.png but is not a PNG file",
        path.display()
    );
    Ok(())
}
