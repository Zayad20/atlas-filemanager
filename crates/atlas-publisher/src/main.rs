mod packager;
mod s3_client;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use s3_client::{R2Client, RegistryExtension};
use std::path::PathBuf;

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

            // 5. Upload icon if exists
            let mut icon_url = None;
            let icon_path = path.join("icon.png");
            if icon_path.exists() {
                let icon_key = format!("extensions/{}/icon.png", manifest.id);
                println!("Uploading icon to R2: {}", icon_key);
                client
                    .upload_file(&icon_path, &icon_key, "image/png", "public, max-age=86400")
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
    }

    Ok(())
}
