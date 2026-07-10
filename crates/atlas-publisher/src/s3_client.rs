use anyhow::{Context, Result};
use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Registry {
    pub last_updated: String,
    pub extensions: HashMap<String, RegistryExtension>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegistryExtension {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub latest_version: String,
    pub download_url: String,
    #[serde(default)]
    pub sha256: String,
}

pub struct R2Client {
    client: Client,
    bucket: String,
    public_url: String,
}

impl R2Client {
    pub async fn new(bucket: String, public_url: String) -> Result<Self> {
        let region_provider = RegionProviderChain::default_provider().or_else(Region::new("auto"));
        let config = aws_config::from_env().region(region_provider).load().await;
        let client = Client::new(&config);

        Ok(Self {
            client,
            bucket,
            public_url,
        })
    }

    pub fn build_public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_url.trim_end_matches('/'), key)
    }

    pub async fn upload_file(
        &self,
        local_path: &Path,
        key: &str,
        content_type: &str,
        cache_control: &str,
    ) -> Result<()> {
        let body = ByteStream::from_path(local_path)
            .await
            .with_context(|| format!("Failed to read {:?}", local_path))?;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type(content_type)
            .cache_control(cache_control)
            .send()
            .await
            .with_context(|| format!("Failed to upload to S3: {}", key))?;

        Ok(())
    }

    pub async fn upload_bytes(
        &self,
        bytes: Vec<u8>,
        key: &str,
        content_type: &str,
        cache_control: &str,
    ) -> Result<()> {
        let body = ByteStream::from(bytes);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type(content_type)
            .cache_control(cache_control)
            .send()
            .await
            .with_context(|| format!("Failed to upload bytes to S3: {}", key))?;

        Ok(())
    }

    pub async fn get_registry(&self) -> Result<Registry> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key("registry.json")
            .send()
            .await;

        match resp {
            Ok(out) => {
                let data = out.body.collect().await?.into_bytes();
                let registry: Registry =
                    serde_json::from_slice(&data).unwrap_or_else(|_| Registry {
                        last_updated: String::new(),
                        extensions: HashMap::new(),
                    });
                Ok(registry)
            }
            Err(_) => Ok(Registry {
                last_updated: String::new(),
                extensions: HashMap::new(),
            }),
        }
    }
}
